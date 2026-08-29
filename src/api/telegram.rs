use super::auth::{get_current_user, AppState};
use crate::crypto::SecretBox;
use crate::telegram::TelegramBot;
use axum::extract::{Json, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::{Duration, Utc};
use rand::distributions::Alphanumeric;
use rand::Rng;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
pub struct TelegramConfigDTO {
    pub tg_bot_token: String,
    pub tg_bot_enabled: String,
    pub tg_admin_chat_id: String,
    pub tg_bot_language: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateTelegramConfigReq {
    pub tg_bot_token: Option<String>,
    pub tg_bot_enabled: Option<serde_json::Value>,
    pub tg_admin_chat_id: Option<String>,
    pub tg_bot_language: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateTokenReq {
    pub peer_ids: Vec<i64>,
    pub expires_hours: Option<i64>,
    pub single_use: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateBroadcastReq {
    pub text: Option<String>,
    pub body: Option<String>,
    pub recipient_mode: Option<String>,
    pub recipient_ids: Option<Vec<i64>>,
}

#[derive(Debug, Deserialize)]
pub struct ListBroadcastsQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct PatchUserReq {
    pub is_blocked: Option<bool>,
    pub language: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetUserPeersReq {
    pub peer_ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNotifsReq {
    pub configs: Option<Vec<NotifConfigItem>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NotifConfigItem {
    pub event_type: String,
    pub notify_clients: Option<bool>,
    pub notify_admin: Option<bool>,
    pub enabled: Option<bool>,
}

pub async fn get_telegram_config(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let token_enc = conn.query_row("SELECT value FROM settings_kv WHERE key = 'tg_bot_token'", [], |r| r.get::<_, String>(0)).unwrap_or_default();
    let sbox = SecretBox::new(&state.settings.secret_key);
    let token = sbox.decrypt(&token_enc).unwrap_or(token_enc);

    let enabled = conn.query_row("SELECT value FROM settings_kv WHERE key = 'tg_bot_enabled'", [], |r| r.get::<_, String>(0))
        .map(|v| v == "true" || v == "1").unwrap_or(false);
    let admin_chat_id = conn.query_row("SELECT value FROM settings_kv WHERE key = 'tg_admin_chat_id'", [], |r| r.get::<_, String>(0)).unwrap_or_default();
    let language = conn.query_row("SELECT value FROM settings_kv WHERE key = 'tg_bot_language'", [], |r| r.get::<_, String>(0)).unwrap_or_else(|_| "both".to_string());

    (StatusCode::OK, Json(TelegramConfigDTO {
        tg_bot_token: token,
        tg_bot_enabled: if enabled { "true".to_string() } else { "false".to_string() },
        tg_admin_chat_id: admin_chat_id,
        tg_bot_language: language,
    })).into_response()
}

pub async fn update_telegram_config(headers: HeaderMap, State(state): State<AppState>, Json(payload): Json<UpdateTelegramConfigReq>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let sbox = SecretBox::new(&state.settings.secret_key);
    let mut token_changed = false;

    if let Some(ref t) = payload.tg_bot_token {
        if !t.trim().is_empty() {
            let enc = sbox.encrypt(t.trim());
            let _ = conn.execute(
                "INSERT INTO settings_kv (key, value) VALUES ('tg_bot_token', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![enc],
            );
            token_changed = true;
        }
    }

    if let Some(ref enabled_val) = payload.tg_bot_enabled {
        let is_enabled = match enabled_val {
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::String(s) => s == "true" || s == "1",
            serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) == 1,
            _ => false,
        };
        let val_str = if is_enabled { "true" } else { "false" };
        let _ = conn.execute(
            "INSERT INTO settings_kv (key, value) VALUES ('tg_bot_enabled', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![val_str],
        );
        token_changed = true;
    }

    if let Some(ref admin_id) = payload.tg_admin_chat_id {
        let _ = conn.execute(
            "INSERT INTO settings_kv (key, value) VALUES ('tg_admin_chat_id', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![admin_id.trim()],
        );
    }

    if let Some(ref lang) = payload.tg_bot_language {
        let _ = conn.execute(
            "INSERT INTO settings_kv (key, value) VALUES ('tg_bot_language', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![lang.trim()],
        );
    }

    if token_changed {
        // Auto restart bot
        let mut lock = state.bot.lock().await;
        if let Some(bot) = lock.take() {
            bot.stop();
        }
        let token_enc = conn.query_row("SELECT value FROM settings_kv WHERE key = 'tg_bot_token'", [], |r| r.get::<_, String>(0)).unwrap_or_default();
        let token = sbox.decrypt(&token_enc).unwrap_or(token_enc);
        let enabled = conn.query_row("SELECT value FROM settings_kv WHERE key = 'tg_bot_enabled'", [], |r| r.get::<_, String>(0))
            .map(|v| v == "true" || v == "1").unwrap_or(false);

        if enabled && token.trim().len() >= 20 {
            let new_bot = Arc::new(TelegramBot::new(token, state.pool.clone(), state.settings.secret_key.clone()));
            if let Ok(_) = new_bot.get_me().await {
                let b = new_bot.clone();
                tokio::spawn(async move {
                    b.start_polling().await;
                });
                *lock = Some(new_bot);
            }
        }
    }

    (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response()
}

pub async fn get_telegram_status(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let lock = state.bot.lock().await;
    if let Some(bot) = lock.as_ref() {
        (StatusCode::OK, Json(serde_json::json!({
            "running": true,
            "started_at": bot.started_at_str(),
            "uptime_seconds": bot.uptime_seconds(),
        }))).into_response()
    } else {
        (StatusCode::OK, Json(serde_json::json!({
            "running": false,
            "started_at": null,
            "uptime_seconds": 0,
        }))).into_response()
    }
}

pub async fn restart_telegram_bot(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let mut lock = state.bot.lock().await;
    if let Some(bot) = lock.take() {
        bot.stop();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let token_enc = conn.query_row("SELECT value FROM settings_kv WHERE key = 'tg_bot_token'", [], |r| r.get::<_, String>(0)).unwrap_or_default();
    let sbox = SecretBox::new(&state.settings.secret_key);
    let token = sbox.decrypt(&token_enc).unwrap_or(token_enc);
    let enabled = conn.query_row("SELECT value FROM settings_kv WHERE key = 'tg_bot_enabled'", [], |r| r.get::<_, String>(0))
        .map(|v| v == "true" || v == "1").unwrap_or(false);

    let mut started = false;
    if enabled && token.trim().len() >= 20 {
        let new_bot = Arc::new(TelegramBot::new(token.clone(), state.pool.clone(), state.settings.secret_key.clone()));
        match new_bot.get_me().await {
            Ok(bot_user) => {
                tracing::info!("Telegram bot verified as @{}", bot_user);
                let b = new_bot.clone();
                tokio::spawn(async move {
                    b.start_polling().await;
                });
                *lock = Some(new_bot);
                started = true;
            }
            Err(e) => {
                tracing::error!("Telegram getMe verification failed: {}", e);
            }
        }
    }

    (StatusCode::OK, Json(serde_json::json!({
        "ok": true,
        "started": started,
    }))).into_response()
}

pub async fn list_telegram_tokens(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    // Get bot username for deep links
    let lock = state.bot.lock().await;
    let bot_username = if let Some(bot) = lock.as_ref() {
        bot.get_me().await.unwrap_or_default()
    } else {
        String::new()
    };

    let mut stmt = match conn.prepare(
        "SELECT id, token, peer_ids, created_by, used_by, created_at, used_at, expires_at, single_use FROM telegram_signup_tokens ORDER BY id DESC"
    ) {
        Ok(s) => s,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let rows = stmt.query_map([], |row| {
        let id: i64 = row.get(0)?;
        let token: String = row.get(1)?;
        let peer_ids_raw: String = row.get(2)?;
        let peer_ids: Vec<i64> = serde_json::from_str(&peer_ids_raw).unwrap_or_default();
        let created_by: Option<i64> = row.get(3)?;
        let used_by_id: Option<i64> = row.get(4)?;
        let created_at: Option<String> = row.get(5)?;
        let used_at: Option<String> = row.get(6)?;
        let expires_at: Option<String> = row.get(7)?;
        let single_use: bool = row.get(8)?;

        let used_by_info = used_by_id.and_then(|uid| {
            conn.query_row(
                "SELECT telegram_username, first_name FROM telegram_users WHERE id = ?1",
                params![uid],
                |r| Ok(serde_json::json!({
                    "telegram_username": r.get::<_, String>(0).unwrap_or_default(),
                    "first_name": r.get::<_, String>(1).unwrap_or_default(),
                })),
            ).ok()
        });

        let deep_link = if !bot_username.is_empty() {
            Some(format!("https://t.me/{}?start={}", bot_username, token))
        } else {
            None
        };

        Ok(serde_json::json!({
            "id": id,
            "token": token,
            "peer_ids": peer_ids,
            "deep_link": deep_link,
            "created_by": created_by,
            "used_by": used_by_info,
            "created_at": created_at,
            "used_at": used_at,
            "expires_at": expires_at,
            "single_use": single_use,
        }))
    }).unwrap();

    let list: Vec<serde_json::Value> = rows.flatten().collect();
    (StatusCode::OK, Json(list)).into_response()
}

pub async fn create_telegram_token(headers: HeaderMap, State(state): State<AppState>, Json(payload): Json<CreateTokenReq>) -> impl IntoResponse {
    let user = match get_current_user(&headers, &state).await {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };

    let token_str: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();

    let peer_ids_json = serde_json::to_string(&payload.peer_ids).unwrap_or_else(|_| "[]".to_string());
    let now_utc = Utc::now();
    let now_str = now_utc.format("%Y-%m-%d %H:%M:%S").to_string();
    let expires_at = payload.expires_hours
        .filter(|&h| h > 0)
        .map(|h| (now_utc + Duration::hours(h)).format("%Y-%m-%d %H:%M:%S").to_string());
    let single_use = payload.single_use.unwrap_or(true);

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let _ = conn.execute(
        r#"
        INSERT INTO telegram_signup_tokens (token, peer_ids, created_by, created_at, expires_at, single_use)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        params![token_str, peer_ids_json, user.id, now_str, expires_at, single_use],
    );

    let id = conn.last_insert_rowid();

    let lock = state.bot.lock().await;
    let bot_username = if let Some(bot) = lock.as_ref() {
        bot.get_me().await.unwrap_or_default()
    } else {
        String::new()
    };
    let deep_link = if !bot_username.is_empty() {
        Some(format!("https://t.me/{}?start={}", bot_username, token_str))
    } else {
        None
    };

    (StatusCode::OK, Json(serde_json::json!({
        "id": id,
        "token": token_str,
        "peer_ids": payload.peer_ids,
        "deep_link": deep_link,
        "created_at": now_str,
        "used_at": null,
        "used_by": null,
        "expires_at": expires_at,
        "single_use": single_use,
    }))).into_response()
}

pub async fn delete_telegram_token(headers: HeaderMap, State(state): State<AppState>, Path(token_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let _ = conn.execute("DELETE FROM telegram_signup_tokens WHERE id = ?1", params![token_id]);
    (StatusCode::OK, Json(serde_json::json!({"status": "deleted"}))).into_response()
}

pub async fn list_telegram_users(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let mut stmt = match conn.prepare(
        "SELECT id, telegram_user_id, telegram_username, first_name, last_name, language, is_blocked, created_at FROM telegram_users ORDER BY id DESC"
    ) {
        Ok(s) => s,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let rows = stmt.query_map([], |row| {
        let id: i64 = row.get(0)?;
        let mut bindings_stmt = conn.prepare(
            "SELECT b.id, b.peer_id, p.name, p.public_key, p.interface, r.name, b.visible
             FROM telegram_peer_bindings b
             LEFT JOIN peers p ON b.peer_id = p.id
             LEFT JOIN routers r ON p.router_id = r.id
             WHERE b.telegram_user_id = ?1"
        ).unwrap();

        let peers: Vec<serde_json::Value> = bindings_stmt.query_map(params![id], |brow| {
            let b_id: i64 = brow.get(0)?;
            let p_id: i64 = brow.get(1)?;
            let p_name: Option<String> = brow.get(2)?;
            let p_pub: Option<String> = brow.get(3)?;
            let p_iface: Option<String> = brow.get(4)?;
            let r_name: Option<String> = brow.get(5)?;
            let visible: bool = brow.get(6)?;

            let display_name = p_name.filter(|s| !s.is_empty()).unwrap_or_else(|| p_pub.unwrap_or_default());
            Ok(serde_json::json!({
                "binding_id": b_id,
                "peer_id": p_id,
                "peer_name": display_name,
                "router_name": r_name.unwrap_or_default(),
                "interface": p_iface.unwrap_or_default(),
                "visible": visible,
            }))
        }).unwrap().flatten().collect();

        Ok(serde_json::json!({
            "id": id,
            "telegram_user_id": row.get::<_, i64>(1)?,
            "telegram_username": row.get::<_, String>(2)?,
            "first_name": row.get::<_, String>(3)?,
            "last_name": row.get::<_, String>(4)?,
            "language": row.get::<_, String>(5)?,
            "is_blocked": row.get::<_, bool>(6)?,
            "created_at": row.get::<_, String>(7)?,
            "peers": peers,
            "subscribed_notifications": [],
        }))
    }).unwrap();

    let list: Vec<serde_json::Value> = rows.flatten().collect();
    (StatusCode::OK, Json(list)).into_response()
}

pub async fn delete_telegram_user(headers: HeaderMap, State(state): State<AppState>, Path(user_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let _ = conn.execute("DELETE FROM telegram_peer_bindings WHERE telegram_user_id = ?1", params![user_id]);
    let _ = conn.execute("DELETE FROM telegram_users WHERE id = ?1", params![user_id]);
    (StatusCode::OK, Json(serde_json::json!({"status": "deleted"}))).into_response()
}

pub async fn patch_telegram_user(headers: HeaderMap, State(state): State<AppState>, Path(user_id): Path<i64>, Json(payload): Json<PatchUserReq>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    if let Some(b) = payload.is_blocked {
        let _ = conn.execute("UPDATE telegram_users SET is_blocked = ?1 WHERE id = ?2", params![b, user_id]);
    }
    if let Some(ref l) = payload.language {
        let _ = conn.execute("UPDATE telegram_users SET language = ?1 WHERE id = ?2", params![l, user_id]);
    }

    (StatusCode::OK, Json(serde_json::json!({"status": "updated"}))).into_response()
}

pub async fn set_telegram_user_peers(headers: HeaderMap, State(state): State<AppState>, Path(user_id): Path<i64>, Json(payload): Json<SetUserPeersReq>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let _ = conn.execute("DELETE FROM telegram_peer_bindings WHERE telegram_user_id = ?1", params![user_id]);
    for pid in payload.peer_ids {
        let _ = conn.execute(
            "INSERT INTO telegram_peer_bindings (telegram_user_id, peer_id, visible) VALUES (?1, ?2, 1)",
            params![user_id, pid],
        );
    }

    (StatusCode::OK, Json(serde_json::json!({"status": "updated"}))).into_response()
}

pub async fn get_telegram_notifications(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    // Ensure default notification events exist
    let default_events = [
        "quota_warning_80",
        "quota_warning_90",
        "quota_hit",
        "quota_lifted",
        "daily_summary",
        "weekly_summary",
    ];
    for evt in default_events {
        let _ = conn.execute(
            "INSERT INTO telegram_notification_config (event_type, notify_clients, notify_admin, enabled) VALUES (?1, 1, 1, 1) ON CONFLICT(event_type) DO NOTHING",
            params![evt],
        );
    }

    let mut stmt = match conn.prepare(
        "SELECT id, event_type, notify_clients, notify_admin, enabled FROM telegram_notification_config ORDER BY id ASC"
    ) {
        Ok(s) => s,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let rows = stmt.query_map([], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, i64>(0)?,
            "event_type": row.get::<_, String>(1)?,
            "notify_clients": row.get::<_, bool>(2)?,
            "notify_admin": row.get::<_, bool>(3)?,
            "enabled": row.get::<_, bool>(4)?,
        }))
    }).unwrap();

    let list: Vec<serde_json::Value> = rows.flatten().collect();
    (StatusCode::OK, Json(list)).into_response()
}

pub async fn update_telegram_notifications(headers: HeaderMap, State(state): State<AppState>, Json(payload): Json<UpdateNotifsReq>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    if let Some(configs) = payload.configs {
        for c in configs {
            let existing: Option<i64> = conn.query_row(
                "SELECT id FROM telegram_notification_config WHERE event_type = ?1",
                params![c.event_type],
                |r| r.get(0),
            ).ok();

            if let Some(id) = existing {
                if let Some(nc) = c.notify_clients {
                    let _ = conn.execute("UPDATE telegram_notification_config SET notify_clients = ?1 WHERE id = ?2", params![nc, id]);
                }
                if let Some(na) = c.notify_admin {
                    let _ = conn.execute("UPDATE telegram_notification_config SET notify_admin = ?1 WHERE id = ?2", params![na, id]);
                }
                if let Some(en) = c.enabled {
                    let _ = conn.execute("UPDATE telegram_notification_config SET enabled = ?1 WHERE id = ?2", params![en, id]);
                }
            } else {
                let _ = conn.execute(
                    "INSERT INTO telegram_notification_config (event_type, notify_clients, notify_admin, enabled) VALUES (?1, ?2, ?3, ?4)",
                    params![c.event_type, c.notify_clients.unwrap_or(true), c.notify_admin.unwrap_or(true), c.enabled.unwrap_or(true)],
                );
            }
        }
    }

    (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response()
}

pub async fn list_telegram_broadcasts(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<ListBroadcastsQuery>,
) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let limit = query.limit.unwrap_or(25);
    let offset = query.offset.unwrap_or(0);

    let total: i64 = conn.query_row("SELECT COUNT(*) FROM telegram_broadcasts", [], |r| r.get(0)).unwrap_or(0);

    let mut stmt = match conn.prepare(
        "SELECT id, body, recipient_mode, status, total_count, sent_count, failed_count, created_at FROM telegram_broadcasts ORDER BY id DESC LIMIT ?1 OFFSET ?2"
    ) {
        Ok(s) => s,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let rows = stmt.query_map(params![limit, offset], |row| {
        let body: String = row.get(1)?;
        let preview = if body.chars().count() > 50 {
            format!("{}...", body.chars().take(50).collect::<String>())
        } else {
            body.clone()
        };

        Ok(serde_json::json!({
            "id": row.get::<_, i64>(0)?,
            "body": body,
            "body_preview": preview,
            "has_photo": false,
            "photo_filename": "",
            "photo_mime": "",
            "recipient_mode": row.get::<_, String>(2)?,
            "status": row.get::<_, String>(3)?,
            "total_count": row.get::<_, i64>(4)?,
            "sent_count": row.get::<_, i64>(5)?,
            "failed_count": row.get::<_, i64>(6)?,
            "acknowledged_count": 0,
            "created_at": row.get::<_, String>(7)?,
            "started_at": null,
            "finished_at": null,
        }))
    }).unwrap();

    let items: Vec<serde_json::Value> = rows.flatten().collect();
    (StatusCode::OK, Json(serde_json::json!({
        "items": items,
        "total": total,
    }))).into_response()
}

pub async fn get_telegram_broadcast(headers: HeaderMap, State(state): State<AppState>, Path(broadcast_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let b_row = conn.query_row(
        "SELECT id, body, recipient_mode, status, total_count, sent_count, failed_count, created_at FROM telegram_broadcasts WHERE id = ?1",
        params![broadcast_id],
        |row| {
            let body: String = row.get(1)?;
            let preview = if body.chars().count() > 50 {
                format!("{}...", body.chars().take(50).collect::<String>())
            } else {
                body.clone()
            };
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "body": body,
                "body_preview": preview,
                "has_photo": false,
                "photo_filename": "",
                "photo_mime": "",
                "recipient_mode": row.get::<_, String>(2)?,
                "status": row.get::<_, String>(3)?,
                "total_count": row.get::<_, i64>(4)?,
                "sent_count": row.get::<_, i64>(5)?,
                "failed_count": row.get::<_, i64>(6)?,
                "acknowledged_count": 0,
                "created_at": row.get::<_, String>(7)?,
                "started_at": null,
                "finished_at": null,
                "recipients": [],
            }))
        },
    );

    match b_row {
        Ok(val) => (StatusCode::OK, Json(val)).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "Broadcast not found").into_response(),
    }
}

pub async fn retry_failed_telegram_broadcast(headers: HeaderMap, State(state): State<AppState>, Path(_broadcast_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({
        "ok": true,
        "queued": 0,
    }))).into_response()
}

pub async fn create_telegram_broadcast(headers: HeaderMap, State(state): State<AppState>, Json(payload): Json<CreateBroadcastReq>) -> impl IntoResponse {
    let user = match get_current_user(&headers, &state).await {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let text = payload.text.or(payload.body).unwrap_or_default();
    let now_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let recipient_mode = payload.recipient_mode.unwrap_or_else(|| "all".to_string());

    let _ = conn.execute(
        r#"
        INSERT INTO telegram_broadcasts (created_by_user_id, body, recipient_mode, status, created_at)
        VALUES (?1, ?2, ?3, 'queued', ?4)
        "#,
        params![user.id, text, recipient_mode, now_str],
    );

    let id = conn.last_insert_rowid();
    (StatusCode::OK, Json(serde_json::json!({
        "id": id,
        "status": "queued",
        "body": text,
        "recipient_mode": recipient_mode,
        "total_count": 0,
        "sent_count": 0,
        "failed_count": 0,
        "created_at": now_str,
    }))).into_response()
}

pub async fn test_telegram_notify(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let admin_chat_id: String = conn.query_row(
        "SELECT value FROM settings_kv WHERE key = 'tg_admin_chat_id'",
        [],
        |r| r.get(0),
    ).unwrap_or_default();

    if admin_chat_id.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "Admin chat ID not set").into_response();
    }

    let chat_id: i64 = match admin_chat_id.trim().parse() {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid admin chat ID").into_response(),
    };

    let lock = state.bot.lock().await;
    if let Some(bot) = lock.as_ref() {
        let msg = "🔔 <b>Test Notification</b>\nwgmik-server Telegram integration is operational.";
        let res = bot.send_message(chat_id, msg, None).await;
        if res.is_ok() {
            (StatusCode::OK, Json(serde_json::json!({"ok": true, "status": "sent"}))).into_response()
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to send message via Telegram").into_response()
        }
    } else {
        (StatusCode::BAD_REQUEST, "Telegram bot is not running").into_response()
    }
}

pub async fn test_telegram_notify_event(headers: HeaderMap, State(state): State<AppState>, Path(event_type): Path<String>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let admin_chat_id: String = conn.query_row(
        "SELECT value FROM settings_kv WHERE key = 'tg_admin_chat_id'",
        [],
        |r| r.get(0),
    ).unwrap_or_default();

    if admin_chat_id.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "Admin chat ID not set").into_response();
    }

    let chat_id: i64 = match admin_chat_id.trim().parse() {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid admin chat ID").into_response(),
    };

    let lock = state.bot.lock().await;
    if let Some(bot) = lock.as_ref() {
        let msg = format!("🔔 <b>Test Event: {}</b>\nThis is a simulation notification.", event_type);
        let res = bot.send_message(chat_id, &msg, None).await;
        if res.is_ok() {
            (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response()
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to send message via Telegram").into_response()
        }
    } else {
        (StatusCode::BAD_REQUEST, "Telegram bot is not running").into_response()
    }
}

#[cfg(test)]
mod tests {
    use crate::db::schema::initialize_database;

    #[test]
    fn test_telegram_config_and_token_db_flow() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let _ = initialize_database(&conn);

        // Test settings kv insertion
        conn.execute(
            "INSERT INTO settings_kv (key, value) VALUES ('tg_bot_enabled', 'true'), ('tg_bot_language', 'fa')",
            [],
        ).unwrap();

        let enabled_val: String = conn.query_row(
            "SELECT value FROM settings_kv WHERE key = 'tg_bot_enabled'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(enabled_val, "true");

        // Insert token
        conn.execute(
            r#"
            INSERT INTO telegram_signup_tokens (token, peer_ids, created_at, single_use)
            VALUES ('tok_12345', '[1, 2]', '2026-08-26 12:00:00', 1)
            "#,
            [],
        ).unwrap();

        let tok_row: (String, String, bool) = conn.query_row(
            "SELECT token, peer_ids, single_use FROM telegram_signup_tokens WHERE token = 'tok_12345'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        assert_eq!(tok_row.0, "tok_12345");
        assert_eq!(tok_row.1, "[1, 2]");
        assert!(tok_row.2);

        // Test telegram_notification_config
        conn.execute(
            "INSERT INTO telegram_notification_config (event_type, notify_clients, notify_admin, enabled) VALUES ('quota_hit', 1, 1, 1)",
            [],
        ).unwrap();

        let notif_row: (String, bool, bool, bool) = conn.query_row(
            "SELECT event_type, notify_clients, notify_admin, enabled FROM telegram_notification_config WHERE event_type = 'quota_hit'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        ).unwrap();
        assert_eq!(notif_row.0, "quota_hit");
        assert!(notif_row.1);
        assert!(notif_row.2);
        assert!(notif_row.3);

        // Test admin chat ID retrieval with formatting
        conn.execute(
            "INSERT INTO settings_kv (key, value) VALUES ('tg_admin_chat_id', '  123456789  ')",
            [],
        ).unwrap();

        let admin_raw: String = conn.query_row(
            "SELECT value FROM settings_kv WHERE key = 'tg_admin_chat_id'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(admin_raw.trim().parse::<i64>().unwrap(), 123456789i64);
    }
}
