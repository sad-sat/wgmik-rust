use super::auth::{get_current_user, AppState};
use crate::crypto::SecretBox;
use crate::telegram::TelegramBot;
use axum::extract::{Json, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::Utc;
use rand::distributions::Alphanumeric;
use rand::Rng;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
pub struct TelegramConfigDTO {
    pub tg_bot_token: String,
    pub tg_bot_enabled: bool,
    pub tg_admin_chat_id: String,
    pub tg_bot_language: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateTokenReq {
    pub peer_ids: Vec<i64>,
    pub single_use: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateBroadcastReq {
    pub body: String,
    pub recipient_mode: Option<String>,
}

pub async fn get_telegram_config(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let token_enc = conn.query_row("SELECT value FROM settings_kv WHERE key = 'tg_bot_token'", [], |r| r.get::<_, String>(0)).unwrap_or_default();
    let sbox = SecretBox::new(&state.settings.secret_key);
    let token = sbox.decrypt(&token_enc).unwrap_or_else(|| token_enc.clone());

    let enabled = conn.query_row("SELECT value FROM settings_kv WHERE key = 'tg_bot_enabled'", [], |r| r.get::<_, String>(0))
        .map(|v| v == "true" || v == "1").unwrap_or(false);
    let admin_chat_id = conn.query_row("SELECT value FROM settings_kv WHERE key = 'tg_admin_chat_id'", [], |r| r.get::<_, String>(0)).unwrap_or_default();
    let language = conn.query_row("SELECT value FROM settings_kv WHERE key = 'tg_bot_language'", [], |r| r.get::<_, String>(0)).unwrap_or_else(|_| "en".to_string());

    (StatusCode::OK, Json(TelegramConfigDTO {
        tg_bot_token: token,
        tg_bot_enabled: enabled,
        tg_admin_chat_id: admin_chat_id,
        tg_bot_language: language,
    })).into_response()
}

pub async fn update_telegram_config(headers: HeaderMap, State(state): State<AppState>, Json(payload): Json<TelegramConfigDTO>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let sbox = SecretBox::new(&state.settings.secret_key);
    let enc = sbox.encrypt(&payload.tg_bot_token);

    let conn = state.pool.get().unwrap();
    let kvs = [
        ("tg_bot_token", enc),
        ("tg_bot_enabled", payload.tg_bot_enabled.to_string()),
        ("tg_admin_chat_id", payload.tg_admin_chat_id.clone()),
        ("tg_bot_language", payload.tg_bot_language.clone()),
    ];

    for (k, v) in kvs {
        let _ = conn.execute(
            "INSERT INTO settings_kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![k, v],
        );
    }

    (StatusCode::OK, Json(payload)).into_response()
}

pub async fn get_telegram_status(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let lock = state.bot.lock().await;
    let running = lock.is_some();
    (StatusCode::OK, Json(serde_json::json!({
        "running": running,
        "started_at": if running { Some(Utc::now().to_rfc3339()) } else { None },
    }))).into_response()
}

pub async fn restart_telegram_bot(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let mut lock = state.bot.lock().await;
    if let Some(bot) = lock.take() {
        bot.stop();
    }

    let conn = state.pool.get().unwrap();
    let token_enc = conn.query_row("SELECT value FROM settings_kv WHERE key = 'tg_bot_token'", [], |r| r.get::<_, String>(0)).unwrap_or_default();
    let sbox = SecretBox::new(&state.settings.secret_key);
    let token = sbox.decrypt(&token_enc).unwrap_or(token_enc);
    let enabled = conn.query_row("SELECT value FROM settings_kv WHERE key = 'tg_bot_enabled'", [], |r| r.get::<_, String>(0))
        .map(|v| v == "true" || v == "1").unwrap_or(false);

    if enabled && !token.trim().is_empty() {
        let new_bot = Arc::new(TelegramBot::new(token, state.pool.clone(), state.settings.secret_key.clone()));
        let b = new_bot.clone();
        tokio::spawn(async move {
            b.start_polling().await;
        });
        *lock = Some(new_bot);
    }

    (StatusCode::OK, Json(serde_json::json!({"status": "restarted"}))).into_response()
}

pub async fn list_telegram_tokens(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let mut stmt = conn.prepare("SELECT id, token, peer_ids, created_by, used_by, created_at, used_at, expires_at, single_use FROM telegram_signup_tokens ORDER BY id DESC").unwrap();
    let rows = stmt.query_map([], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, i64>(0)?,
            "token": row.get::<_, String>(1)?,
            "peer_ids": serde_json::from_str::<serde_json::Value>(&row.get::<_, String>(2)?).unwrap_or(serde_json::json!([])),
            "created_by": row.get::<_, Option<i64>>(3)?,
            "used_by": row.get::<_, Option<i64>>(4)?,
            "created_at": row.get::<_, String>(5)?,
            "used_at": row.get::<_, Option<String>>(6)?,
            "expires_at": row.get::<_, Option<String>>(7)?,
            "single_use": row.get::<_, bool>(8)?,
        }))
    }).unwrap();

    let mut list = Vec::new();
    for r in rows {
        if let Ok(item) = r {
            list.push(item);
        }
    }

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
    let now_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let single_use = payload.single_use.unwrap_or(true);

    let conn = state.pool.get().unwrap();
    let _ = conn.execute(
        r#"
        INSERT INTO telegram_signup_tokens (token, peer_ids, created_by, created_at, single_use)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![token_str, peer_ids_json, user.id, now_str, single_use],
    );

    let id = conn.last_insert_rowid();
    (StatusCode::OK, Json(serde_json::json!({
        "id": id,
        "token": token_str,
        "peer_ids": payload.peer_ids,
        "single_use": single_use,
    }))).into_response()
}

pub async fn delete_telegram_token(headers: HeaderMap, State(state): State<AppState>, Path(token_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let _ = conn.execute("DELETE FROM telegram_signup_tokens WHERE id = ?1", params![token_id]);
    (StatusCode::OK, Json(serde_json::json!({"status": "deleted"}))).into_response()
}

pub async fn list_telegram_users(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let mut stmt = conn.prepare("SELECT id, telegram_user_id, telegram_username, first_name, last_name, language, is_blocked, created_at FROM telegram_users ORDER BY id ASC").unwrap();
    let rows = stmt.query_map([], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, i64>(0)?,
            "telegram_user_id": row.get::<_, i64>(1)?,
            "telegram_username": row.get::<_, String>(2)?,
            "first_name": row.get::<_, String>(3)?,
            "last_name": row.get::<_, String>(4)?,
            "language": row.get::<_, String>(5)?,
            "is_blocked": row.get::<_, bool>(6)?,
            "created_at": row.get::<_, String>(7)?,
        }))
    }).unwrap();

    let mut list = Vec::new();
    for r in rows {
        if let Ok(item) = r {
            list.push(item);
        }
    }

    (StatusCode::OK, Json(list)).into_response()
}

pub async fn list_telegram_broadcasts(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let mut stmt = conn.prepare("SELECT id, body, recipient_mode, status, total_count, sent_count, failed_count, created_at FROM telegram_broadcasts ORDER BY id DESC").unwrap();
    let rows = stmt.query_map([], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, i64>(0)?,
            "body": row.get::<_, String>(1)?,
            "recipient_mode": row.get::<_, String>(2)?,
            "status": row.get::<_, String>(3)?,
            "total_count": row.get::<_, i64>(4)?,
            "sent_count": row.get::<_, i64>(5)?,
            "failed_count": row.get::<_, i64>(6)?,
            "created_at": row.get::<_, String>(7)?,
        }))
    }).unwrap();

    let mut list = Vec::new();
    for r in rows {
        if let Ok(item) = r {
            list.push(item);
        }
    }

    (StatusCode::OK, Json(list)).into_response()
}

pub async fn create_telegram_broadcast(headers: HeaderMap, State(state): State<AppState>, Json(payload): Json<CreateBroadcastReq>) -> impl IntoResponse {
    let user = match get_current_user(&headers, &state).await {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };

    let conn = state.pool.get().unwrap();
    let now_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let recipient_mode = payload.recipient_mode.unwrap_or_else(|| "all".to_string());

    let _ = conn.execute(
        r#"
        INSERT INTO telegram_broadcasts (created_by_user_id, body, recipient_mode, status, created_at)
        VALUES (?1, ?2, ?3, 'queued', ?4)
        "#,
        params![user.id, payload.body, recipient_mode, now_str],
    );

    let id = conn.last_insert_rowid();
    (StatusCode::OK, Json(serde_json::json!({"id": id, "status": "queued"}))).into_response()
}

pub async fn test_telegram_notify(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
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
            (StatusCode::OK, Json(serde_json::json!({"status": "sent"}))).into_response()
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to send message via Telegram").into_response()
        }
    } else {
        (StatusCode::BAD_REQUEST, "Telegram bot is not running").into_response()
    }
}
