use super::auth::{get_current_user, AppState};
use crate::accounting::deltas::counter_day_key;
use crate::calendar::parse_timezone;
use crate::crypto::{generate_wireguard_keypair, SecretBox};
use crate::db::models::{Peer, Router};
use crate::routeros::factory::make_client;
use axum::extract::{Json, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct PeerListDTO {
    pub id: i64,
    pub router_id: i64,
    pub router_name: String,
    pub interface: String,
    pub ros_id: String,
    pub name: String,
    pub public_key: String,
    pub allowed_address: String,
    pub comment: String,
    pub disabled: bool,
    pub selected: bool,
    pub router_sync_status: String,
    pub router_sync_first_seen_at: Option<String>,
    pub router_sync_last_seen_at: Option<String>,
    pub today_rx: i64,
    pub today_tx: i64,
    pub month_rx: i64,
    pub month_tx: i64,
    pub alltime_rx: i64,
    pub alltime_tx: i64,
    pub current_rx: i64,
    pub current_tx: i64,
    pub total_rx: i64,
    pub total_tx: i64,
    pub is_online: bool,
    pub online: bool,
    pub last_handshake_ago: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ListPeersQuery {
    pub router_id: Option<i64>,
    pub router_ids: Option<String>,
    pub interface: Option<String>,
    pub selected_only: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdatePeerReq {
    pub name: Option<String>,
    pub allowed_address: Option<String>,
    pub disabled: Option<bool>,
    pub selected: Option<bool>,
    pub comment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportPrefsDTO {
    pub endpoint: String,
    pub dns: String,
    pub mtu: u16,
    pub persistent_keepalive: u16,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PrivateKeyDTO {
    pub private_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RenewKeysResp {
    pub public_key: String,
    pub private_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResolveSyncReq {
    pub resolution: String, // "accept" | "hide" | "delete"
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QuotaDTO {
    pub monthly_limit_bytes: i64,
    pub reset_day: i64,
}

pub async fn list_peers(headers: HeaderMap, State(state): State<AppState>, Query(query): Query<ListPeersQuery>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let now_utc = Utc::now();
    let tz = parse_timezone(&state.settings.timezone);
    let day_key = counter_day_key(now_utc, tz);
    let month_key = now_utc.format("%Y-%m").to_string();

    let mut sql = r#"
        SELECT p.id, p.router_id, r.name, p.interface, p.ros_id, p.name, p.public_key, p.allowed_address,
               p.comment, p.disabled, p.selected, p.router_sync_status,
               p.router_sync_first_seen_at, p.router_sync_last_seen_at,
               COALESCE(d.rx, 0), COALESCE(d.tx, 0),
               COALESCE(m.rx, 0), COALESCE(m.tx, 0),
               (SELECT COALESCE(SUM(rx), 0) FROM usage_monthly WHERE peer_id = p.id),
               (SELECT COALESCE(SUM(tx), 0) FROM usage_monthly WHERE peer_id = p.id)
        FROM peers p
        JOIN routers r ON p.router_id = r.id
        LEFT JOIN usage_daily d ON p.id = d.peer_id AND d.day = ?1
        LEFT JOIN usage_monthly m ON p.id = m.peer_id AND m.month_key = ?2
        WHERE 1=1
    "#.to_string();

    if query.selected_only.unwrap_or(false) {
        sql.push_str(" AND p.selected = 1");
    }
    let mut router_id_list = Vec::new();
    if let Some(r_id) = query.router_id {
        router_id_list.push(r_id);
    }
    if let Some(ref rids_str) = query.router_ids {
        for part in rids_str.split(',') {
            if let Ok(id) = part.trim().parse::<i64>() {
                if !router_id_list.contains(&id) {
                    router_id_list.push(id);
                }
            }
        }
    }
    if !router_id_list.is_empty() {
        let in_clause = router_id_list.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
        sql.push_str(&format!(" AND p.router_id IN ({})", in_clause));
    }
    if let Some(ref iface) = query.interface {
        if !iface.is_empty() {
            sql.push_str(&format!(" AND p.interface = '{}'", iface.replace('\'', "''")));
        }
    }
    sql.push_str(" ORDER BY p.id ASC");

    let mut stmt = conn.prepare(&sql).unwrap();

    let rows = stmt.query_map(params![day_key, month_key], |row| {
        let today_rx: i64 = row.get(14)?;
        let today_tx: i64 = row.get(15)?;
        let month_rx: i64 = row.get(16)?;
        let month_tx: i64 = row.get(17)?;
        let alltime_rx: i64 = row.get(18)?;
        let alltime_tx: i64 = row.get(19)?;

        Ok(PeerListDTO {
            id: row.get(0)?,
            router_id: row.get(1)?,
            router_name: row.get(2)?,
            interface: row.get(3)?,
            ros_id: row.get(4)?,
            name: row.get(5)?,
            public_key: row.get(6)?,
            allowed_address: row.get(7)?,
            comment: row.get(8)?,
            disabled: row.get(9)?,
            selected: row.get(10)?,
            router_sync_status: row.get(11)?,
            router_sync_first_seen_at: row.get(12)?,
            router_sync_last_seen_at: row.get(13)?,
            today_rx,
            today_tx,
            month_rx,
            month_tx,
            alltime_rx,
            alltime_tx,
            current_rx: today_rx,
            current_tx: today_tx,
            total_rx: alltime_rx,
            total_tx: alltime_tx,
            is_online: false,
            online: false,
            last_handshake_ago: None,
        })
    }).unwrap();

    let mut list = Vec::new();
    for r in rows {
        if let Ok(p) = r {
            list.push(p);
        }
    }

    (StatusCode::OK, Json(list)).into_response()
}

pub async fn patch_peer(headers: HeaderMap, State(state): State<AppState>, Path(peer_id): Path<i64>, Json(payload): Json<UpdatePeerReq>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let peer = match get_peer_by_id(&conn, peer_id) {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, "Peer not found").into_response(),
    };

    if let Some(name) = payload.name {
        let _ = conn.execute("UPDATE peers SET name = ?1 WHERE id = ?2", params![name, peer_id]);
    }
    if let Some(addr) = payload.allowed_address {
        let _ = conn.execute("UPDATE peers SET allowed_address = ?1 WHERE id = ?2", params![addr, peer_id]);
    }
    if let Some(dis) = payload.disabled {
        let _ = conn.execute("UPDATE peers SET disabled = ?1 WHERE id = ?2", params![dis, peer_id]);
        // Update on RouterOS
        if let Some(router) = get_router_by_id(&conn, peer.router_id) {
            let client = make_client(&router, &state.settings.secret_key, Some(5));
            let _ = client.set_peer_disabled(&peer.interface, &peer.ros_id, dis).await;
        }
    }
    if let Some(sel) = payload.selected {
        let _ = conn.execute("UPDATE peers SET selected = ?1 WHERE id = ?2", params![sel, peer_id]);
    }
    if let Some(comment) = payload.comment {
        let _ = conn.execute("UPDATE peers SET comment = ?1 WHERE id = ?2", params![comment, peer_id]);
    }

    (StatusCode::OK, Json(serde_json::json!({"status": "updated"}))).into_response()
}

pub async fn delete_peer(headers: HeaderMap, State(state): State<AppState>, Path(peer_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let peer = match get_peer_by_id(&conn, peer_id) {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, "Peer not found").into_response(),
    };

    if let Some(router) = get_router_by_id(&conn, peer.router_id) {
        let client = make_client(&router, &state.settings.secret_key, Some(5));
        let _ = client.remove_wireguard_peer(&peer.interface, &peer.ros_id).await;
    }

    let _ = conn.execute("DELETE FROM peers WHERE id = ?1", params![peer_id]);
    (StatusCode::OK, Json(serde_json::json!({"status": "deleted"}))).into_response()
}

pub async fn renew_peer_keys(headers: HeaderMap, State(state): State<AppState>, Path(peer_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let peer = match get_peer_by_id(&conn, peer_id) {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, "Peer not found").into_response(),
    };

    let (priv_k, pub_k) = generate_wireguard_keypair();

    if let Some(router) = get_router_by_id(&conn, peer.router_id) {
        let client = make_client(&router, &state.settings.secret_key, Some(5));
        let _ = client.set_peer_keys(&peer.interface, &peer.ros_id, &pub_k, &priv_k).await;
    }

    let _ = conn.execute("UPDATE peers SET public_key = ?1 WHERE id = ?2", params![pub_k, peer_id]);
    let sbox = SecretBox::new(&state.settings.secret_key);
    let enc = sbox.encrypt(&priv_k);
    let _ = conn.execute(
        "INSERT INTO settings_kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![format!("peer_private_key:{}", peer_id), enc],
    );

    (StatusCode::OK, Json(RenewKeysResp { public_key: pub_k, private_key: priv_k })).into_response()
}

pub async fn resolve_peer_sync(headers: HeaderMap, State(state): State<AppState>, Path(peer_id): Path<i64>, Json(payload): Json<ResolveSyncReq>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let peer = match get_peer_by_id(&conn, peer_id) {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, "Peer not found").into_response(),
    };

    match payload.resolution.as_str() {
        "accept" => {
            let _ = conn.execute("UPDATE peers SET selected = 1, router_sync_status = 'synced' WHERE id = ?1", params![peer_id]);
        }
        "hide" => {
            let _ = conn.execute("UPDATE peers SET selected = 0, router_sync_status = 'synced' WHERE id = ?1", params![peer_id]);
        }
        "delete" => {
            if let Some(router) = get_router_by_id(&conn, peer.router_id) {
                let client = make_client(&router, &state.settings.secret_key, Some(5));
                let _ = client.remove_wireguard_peer(&peer.interface, &peer.ros_id).await;
            }
            let _ = conn.execute("DELETE FROM peers WHERE id = ?1", params![peer_id]);
        }
        _ => return (StatusCode::BAD_REQUEST, "Invalid resolution").into_response(),
    }

    (StatusCode::OK, Json(serde_json::json!({"status": "resolved"}))).into_response()
}

pub async fn get_peer_actions(headers: HeaderMap, State(state): State<AppState>, Path(peer_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let mut stmt = conn.prepare("SELECT id, peer_id, ts, action, note FROM actions WHERE peer_id = ?1 ORDER BY ts DESC LIMIT 50").unwrap();
    let rows = stmt.query_map(params![peer_id], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, i64>(0)?,
            "peer_id": row.get::<_, Option<i64>>(1)?,
            "ts": row.get::<_, String>(2)?,
            "action": row.get::<_, String>(3)?,
            "note": row.get::<_, String>(4)?,
        }))
    }).unwrap();

    let mut list = Vec::new();
    for r in rows {
        if let Ok(a) = r {
            list.push(a);
        }
    }

    (StatusCode::OK, Json(list)).into_response()
}

pub async fn get_peer_quota(headers: HeaderMap, State(state): State<AppState>, Path(peer_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let q = conn.query_row(
        "SELECT monthly_limit_bytes, reset_day FROM quotas WHERE peer_id = ?1",
        params![peer_id],
        |row| Ok(QuotaDTO {
            monthly_limit_bytes: row.get(0)?,
            reset_day: row.get(1)?,
        }),
    ).unwrap_or(QuotaDTO { monthly_limit_bytes: 0, reset_day: 1 });

    (StatusCode::OK, Json(q)).into_response()
}

pub async fn patch_peer_quota(headers: HeaderMap, State(state): State<AppState>, Path(peer_id): Path<i64>, Json(payload): Json<QuotaDTO>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let _ = conn.execute(
        r#"
        INSERT INTO quotas (peer_id, monthly_limit_bytes, reset_day)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(peer_id) DO UPDATE SET
            monthly_limit_bytes = excluded.monthly_limit_bytes,
            reset_day = excluded.reset_day
        "#,
        params![peer_id, payload.monthly_limit_bytes, payload.reset_day],
    );

    (StatusCode::OK, Json(payload)).into_response()
}

#[derive(Debug, Default, Deserialize)]
pub struct PeerUsageQuery {
    pub window: Option<String>,
    pub seconds: Option<i64>,
    pub interval: Option<i64>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub all_time: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UsagePointDTO {
    pub day: String,
    pub rx: i64,
    pub tx: i64,
}

pub async fn get_peer(headers: HeaderMap, State(state): State<AppState>, Path(peer_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let now_utc = Utc::now();
    let tz = parse_timezone(&state.settings.timezone);
    let day_key = counter_day_key(now_utc, tz);
    let month_key = now_utc.format("%Y-%m").to_string();

    let sql = r#"
        SELECT p.id, p.router_id, r.name, p.interface, p.ros_id, p.name, p.public_key, p.allowed_address,
               p.comment, p.disabled, p.selected, p.router_sync_status,
               p.router_sync_first_seen_at, p.router_sync_last_seen_at,
               COALESCE(d.rx, 0), COALESCE(d.tx, 0),
               COALESCE(m.rx, 0), COALESCE(m.tx, 0),
               (SELECT COALESCE(SUM(rx), 0) FROM usage_monthly WHERE peer_id = p.id),
               (SELECT COALESCE(SUM(tx), 0) FROM usage_monthly WHERE peer_id = p.id)
        FROM peers p
        JOIN routers r ON p.router_id = r.id
        LEFT JOIN usage_daily d ON p.id = d.peer_id AND d.day = ?1
        LEFT JOIN usage_monthly m ON p.id = m.peer_id AND m.month_key = ?2
        WHERE p.id = ?3
    "#;

    let res = conn.query_row(sql, params![day_key, month_key, peer_id], |row| {
        let today_rx: i64 = row.get(14)?;
        let today_tx: i64 = row.get(15)?;
        let month_rx: i64 = row.get(16)?;
        let month_tx: i64 = row.get(17)?;
        let alltime_rx: i64 = row.get(18)?;
        let alltime_tx: i64 = row.get(19)?;

        Ok(PeerListDTO {
            id: row.get(0)?,
            router_id: row.get(1)?,
            router_name: row.get(2)?,
            interface: row.get(3)?,
            ros_id: row.get(4)?,
            name: row.get(5)?,
            public_key: row.get(6)?,
            allowed_address: row.get(7)?,
            comment: row.get(8)?,
            disabled: row.get(9)?,
            selected: row.get(10)?,
            router_sync_status: row.get(11)?,
            router_sync_first_seen_at: row.get(12)?,
            router_sync_last_seen_at: row.get(13)?,
            today_rx,
            today_tx,
            month_rx,
            month_tx,
            alltime_rx,
            alltime_tx,
            current_rx: today_rx,
            current_tx: today_tx,
            total_rx: alltime_rx,
            total_tx: alltime_tx,
            is_online: false,
            online: false,
            last_handshake_ago: None,
        })
    });

    match res {
        Ok(dto) => (StatusCode::OK, Json(dto)).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "Peer not found").into_response(),
    }
}

pub async fn get_peer_usage(
    headers: HeaderMap,
    Path(peer_id): Path<i64>,
    Query(query): Query<PeerUsageQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let window = query.window.as_deref().unwrap_or("daily");
    let now_utc = Utc::now();
    let mut points = Vec::new();

    if window == "raw" {
        let seconds = query.seconds.unwrap_or(86400).max(60);
        let start_str = query.start.unwrap_or_else(|| (now_utc - chrono::Duration::seconds(seconds)).format("%Y-%m-%d %H:%M:%S").to_string());
        let end_str = query.end.unwrap_or_else(|| now_utc.format("%Y-%m-%d %H:%M:%S").to_string());

        let mut stmt = match conn.prepare(
            "SELECT minute_ts, rx, tx FROM usage_minute WHERE peer_id = ?1 AND minute_ts >= ?2 AND minute_ts <= ?3 ORDER BY minute_ts ASC"
        ) {
            Ok(s) => s,
            Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
        };

        let rows = stmt.query_map(params![peer_id, start_str, end_str], |row| {
            Ok(UsagePointDTO {
                day: row.get(0)?,
                rx: row.get(1)?,
                tx: row.get(2)?,
            })
        });

        if let Ok(iter) = rows {
            for r in iter {
                if let Ok(p) = r {
                    points.push(p);
                }
            }
        }
    } else {
        // Daily window
        let mut stmt = match conn.prepare(
            "SELECT day, rx, tx FROM usage_daily WHERE peer_id = ?1 ORDER BY day ASC"
        ) {
            Ok(s) => s,
            Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
        };

        let rows = stmt.query_map(params![peer_id], |row| {
            Ok(UsagePointDTO {
                day: row.get(0)?,
                rx: row.get(1)?,
                tx: row.get(2)?,
            })
        });

        if let Ok(iter) = rows {
            for r in iter {
                if let Ok(p) = r {
                    points.push(p);
                }
            }
        }
    }

    (StatusCode::OK, Json(points)).into_response()
}

pub async fn reset_peer_metrics(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(peer_id): Path<i64>,
) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let deleted_samples = conn.execute("DELETE FROM usage_samples WHERE peer_id = ?1", params![peer_id]).unwrap_or(0);
    let deleted_minutes = conn.execute("DELETE FROM usage_minute WHERE peer_id = ?1", params![peer_id]).unwrap_or(0);
    let deleted_daily = conn.execute("DELETE FROM usage_daily WHERE peer_id = ?1", params![peer_id]).unwrap_or(0);
    let deleted_monthly = conn.execute("DELETE FROM usage_monthly WHERE peer_id = ?1", params![peer_id]).unwrap_or(0);

    (StatusCode::OK, Json(serde_json::json!({
        "ok": true,
        "deleted_samples": deleted_samples,
        "deleted_minutes": deleted_minutes,
        "deleted_daily": deleted_daily,
        "deleted_monthly": deleted_monthly,
    }))).into_response()
}

pub async fn get_peer_client_private_key(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(peer_id): Path<i64>,
) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let key = format!("peer_private_key:{}", peer_id);
    let enc: Option<String> = conn.query_row(
        "SELECT value FROM settings_kv WHERE key = ?1",
        params![key],
        |row| row.get(0),
    ).ok();

    let priv_key = enc.and_then(|e| {
        let sbox = SecretBox::new(&state.settings.secret_key);
        sbox.decrypt(&e)
    });

    (StatusCode::OK, Json(serde_json::json!({
        "private_key": priv_key
    }))).into_response()
}

pub async fn get_peer_client_export_prefs(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(peer_id): Path<i64>,
) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let get_pref = |suffix: &str| -> Option<String> {
        conn.query_row(
            "SELECT value FROM settings_kv WHERE key = ?1",
            params![format!("{}:{}", suffix, peer_id)],
            |r| r.get(0),
        ).ok()
    };

    let endpoint = get_pref("peer_export_endpoint").unwrap_or_default();
    let dns = get_pref("peer_export_dns").unwrap_or_else(|| "1.1.1.1, 8.8.8.8".to_string());
    let mtu: i64 = get_pref("peer_export_mtu").and_then(|v| v.parse().ok()).unwrap_or(1420);
    let persistent_keepalive: i64 = get_pref("peer_export_keepalive").and_then(|v| v.parse().ok()).unwrap_or(25);

    (StatusCode::OK, Json(serde_json::json!({
        "endpoint": endpoint,
        "dns": dns,
        "mtu": mtu,
        "persistent_keepalive": persistent_keepalive,
    }))).into_response()
}

pub async fn reconcile_peer(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(peer_id): Path<i64>,
) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let peer = match get_peer_by_id(&conn, peer_id) {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, "Peer not found").into_response(),
    };

    let router = match get_router_by_id(&conn, peer.router_id) {
        Some(r) => r,
        None => return (StatusCode::NOT_FOUND, "Router not found").into_response(),
    };

    let client = make_client(&router, &state.settings.secret_key, Some(10));
    let live_peers = client.list_all_wireguard_peers().await.unwrap_or_default();
    let live_matching = live_peers.into_iter().find(|lp| lp.interface == peer.interface && lp.public_key == peer.public_key);

    if let Some(lp) = live_matching {
        let _ = conn.execute(
            "UPDATE peers SET ros_id = ?1, name = ?2, allowed_address = ?3, disabled = ?4, router_sync_status = 'synced' WHERE id = ?5",
            params![lp.ros_id, lp.name, lp.allowed_address, lp.disabled, peer_id],
        );
    }

    (StatusCode::OK, Json(serde_json::json!({"status": "reconciled"}))).into_response()
}

fn get_peer_by_id(conn: &rusqlite::Connection, id: i64) -> Option<Peer> {
    conn.query_row(
        "SELECT id, router_id, interface, ros_id, name, public_key, allowed_address, comment, disabled, selected, router_sync_status FROM peers WHERE id = ?1",
        params![id],
        |row| Ok(Peer {
            id: row.get(0)?,
            router_id: row.get(1)?,
            interface: row.get(2)?,
            ros_id: row.get(3)?,
            name: row.get(4)?,
            public_key: row.get(5)?,
            allowed_address: row.get(6)?,
            comment: row.get(7)?,
            disabled: row.get(8)?,
            selected: row.get(9)?,
            router_sync_status: row.get(10)?,
            router_sync_first_seen_at: None,
            router_sync_last_seen_at: None,
        }),
    ).ok()
}

fn get_router_by_id(conn: &rusqlite::Connection, id: i64) -> Option<Router> {
    conn.query_row(
        "SELECT id, name, host, proto, port, username, secret_enc, tls_verify, enabled, ros_version, ros_supported FROM routers WHERE id = ?1",
        params![id],
        |row| Ok(Router {
            id: row.get(0)?,
            name: row.get(1)?,
            host: row.get(2)?,
            proto: row.get(3)?,
            port: row.get(4)?,
            username: row.get(5)?,
            secret_enc: row.get(6)?,
            tls_verify: row.get(7)?,
            enabled: row.get(8)?,
            ros_version: row.get(9)?,
            ros_version_checked_at: None,
            ros_supported: row.get(10)?,
        }),
    ).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::initialize_database;

    #[test]
    fn test_peer_retrieval_and_usage_queries() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let _ = initialize_database(&conn);

        conn.execute(
            "INSERT INTO routers (name, host, proto, port, username, secret_enc, tls_verify, enabled, ros_version, ros_supported) VALUES ('Router1', '192.168.88.1', 'rest', 443, 'admin', 'enc', 0, 1, '7.12', 1)",
            [],
        ).unwrap();

        let now_s = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        conn.execute(
            r#"
            INSERT INTO peers (router_id, interface, ros_id, name, public_key, allowed_address, disabled, selected, router_sync_status, router_sync_first_seen_at, router_sync_last_seen_at)
            VALUES (1, 'wg0', '*1', 'UserA', 'pub_key_A', '10.0.0.5/32', 0, 1, 'synced', ?1, ?1)
            "#,
            params![now_s],
        ).unwrap();

        let peer = get_peer_by_id(&conn, 1).expect("Peer should exist");
        assert_eq!(peer.name, "UserA");
        assert_eq!(peer.public_key, "pub_key_A");
        assert_eq!(peer.router_sync_status, "synced");

        // Insert minute usage
        let min_s = Utc::now().format("%Y-%m-%d %H:%M:00").to_string();
        conn.execute(
            "INSERT INTO usage_minute (peer_id, minute_ts, rx, tx) VALUES (1, ?1, 4096, 8192)",
            params![min_s],
        ).unwrap();

        let minute_row: (i64, i64) = conn.query_row(
            "SELECT rx, tx FROM usage_minute WHERE peer_id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(minute_row.0, 4096);
        assert_eq!(minute_row.1, 8192);
    }
}
