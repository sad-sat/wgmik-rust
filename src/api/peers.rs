use super::auth::{get_current_user, AppState};
use crate::accounting::deltas::counter_day_key;
use crate::calendar::parse_timezone;
use crate::crypto::{generate_wireguard_keypair, SecretBox};
use crate::db::models::{Peer, Router};
use crate::routeros::factory::make_client;
use axum::extract::{Json, Path, State};
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
    pub today_rx: i64,
    pub today_tx: i64,
    pub month_rx: i64,
    pub month_tx: i64,
    pub alltime_rx: i64,
    pub alltime_tx: i64,
    pub is_online: bool,
    pub last_handshake_ago: Option<i64>,
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

pub async fn list_peers(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let now_utc = Utc::now();
    let tz = parse_timezone(&state.settings.timezone);
    let day_key = counter_day_key(now_utc, tz);
    let month_key = now_utc.format("%Y-%m").to_string();

    let mut stmt = conn.prepare(
        r#"
        SELECT p.id, p.router_id, r.name, p.interface, p.ros_id, p.name, p.public_key, p.allowed_address,
               p.comment, p.disabled, p.selected, p.router_sync_status,
               COALESCE(d.rx, 0), COALESCE(d.tx, 0),
               COALESCE(m.rx, 0), COALESCE(m.tx, 0),
               (SELECT COALESCE(SUM(rx), 0) FROM usage_monthly WHERE peer_id = p.id),
               (SELECT COALESCE(SUM(tx), 0) FROM usage_monthly WHERE peer_id = p.id)
        FROM peers p
        JOIN routers r ON p.router_id = r.id
        LEFT JOIN usage_daily d ON p.id = d.peer_id AND d.day = ?1
        LEFT JOIN usage_monthly m ON p.id = m.peer_id AND m.month_key = ?2
        ORDER BY p.id ASC
        "#,
    ).unwrap();

    let rows = stmt.query_map(params![day_key, month_key], |row| {
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
            today_rx: row.get(12)?,
            today_tx: row.get(13)?,
            month_rx: row.get(14)?,
            month_tx: row.get(15)?,
            alltime_rx: row.get(16)?,
            alltime_tx: row.get(17)?,
            is_online: false,
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
