use super::auth::{get_current_user, AppState};
use crate::crypto::SecretBox;
use crate::db::models::Router;
use crate::routeros::factory::make_client;
use crate::routeros::version::is_routeros_supported;
use axum::extract::{Json, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct RouterDTO {
    pub id: i64,
    pub name: String,
    pub host: String,
    pub proto: String,
    pub port: u16,
    pub username: String,
    pub tls_verify: bool,
    pub enabled: bool,
    pub ros_version: String,
    pub ros_supported: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateRouterReq {
    pub name: String,
    pub host: String,
    pub proto: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub tls_verify: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateRouterReq {
    pub name: Option<String>,
    pub host: Option<String>,
    pub proto: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub tls_verify: Option<bool>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RouterTestResp {
    pub success: bool,
    pub version: String,
    pub supported: bool,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RouterDeleteImpactDTO {
    pub peers_count: i64,
    pub samples_count: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImportPeersReq {
    pub public_keys: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AddPeerReq {
    pub interface: String,
    pub name: String,
    pub public_key: String,
    pub allowed_address: String,
    pub comment: Option<String>,
    pub disabled: Option<bool>,
    pub private_key: Option<String>,
    pub preshared_key: Option<String>,
    pub client_endpoint: Option<String>,
}

pub async fn list_routers(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let mut stmt = conn.prepare("SELECT id, name, host, proto, port, username, tls_verify, enabled, ros_version, ros_supported FROM routers").unwrap();
    let rows = stmt.query_map([], |row| {
        Ok(RouterDTO {
            id: row.get(0)?,
            name: row.get(1)?,
            host: row.get(2)?,
            proto: row.get(3)?,
            port: row.get(4)?,
            username: row.get(5)?,
            tls_verify: row.get(6)?,
            enabled: row.get(7)?,
            ros_version: row.get(8)?,
            ros_supported: row.get(9)?,
        })
    }).unwrap();

    let mut list = Vec::new();
    for r in rows {
        if let Ok(rt) = r {
            list.push(rt);
        }
    }

    (StatusCode::OK, Json(list)).into_response()
}

pub async fn get_router(headers: HeaderMap, State(state): State<AppState>, Path(router_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let rt = conn.query_row(
        "SELECT id, name, host, proto, port, username, tls_verify, enabled, ros_version, ros_supported FROM routers WHERE id = ?1",
        params![router_id],
        |row| Ok(RouterDTO {
            id: row.get(0)?,
            name: row.get(1)?,
            host: row.get(2)?,
            proto: row.get(3)?,
            port: row.get(4)?,
            username: row.get(5)?,
            tls_verify: row.get(6)?,
            enabled: row.get(7)?,
            ros_version: row.get(8)?,
            ros_supported: row.get(9)?,
        }),
    );

    match rt {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "Router not found").into_response(),
    }
}

pub async fn create_router(headers: HeaderMap, State(state): State<AppState>, Json(payload): Json<CreateRouterReq>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let sbox = SecretBox::new(&state.settings.secret_key);
    let secret_enc = sbox.encrypt(&payload.password);

    // Probe version
    let dummy_router = Router {
        id: 0,
        name: payload.name.clone(),
        host: payload.host.clone(),
        proto: payload.proto.clone(),
        port: payload.port,
        username: payload.username.clone(),
        secret_enc: secret_enc.clone(),
        tls_verify: payload.tls_verify,
        enabled: true,
        ros_version: String::new(),
        ros_version_checked_at: None,
        ros_supported: false,
    };

    let client = make_client(&dummy_router, &state.settings.secret_key, Some(6));
    let version = client.get_system_version().await.unwrap_or_default();
    let supported = is_routeros_supported(Some(&version));

    let conn = state.pool.get().unwrap();
    let now_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let res = conn.execute(
        r#"
        INSERT INTO routers (name, host, proto, port, username, secret_enc, tls_verify, enabled, ros_version, ros_version_checked_at, ros_supported)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, ?10)
        "#,
        params![payload.name, payload.host, payload.proto, payload.port, payload.username, secret_enc, payload.tls_verify, version, now_str, supported],
    );

    if res.is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to insert router").into_response();
    }

    let id = conn.last_insert_rowid();
    (StatusCode::OK, Json(RouterDTO {
        id,
        name: payload.name,
        host: payload.host,
        proto: payload.proto,
        port: payload.port,
        username: payload.username,
        tls_verify: payload.tls_verify,
        enabled: true,
        ros_version: version,
        ros_supported: supported,
    })).into_response()
}

pub async fn update_router(headers: HeaderMap, State(state): State<AppState>, Path(router_id): Path<i64>, Json(payload): Json<UpdateRouterReq>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    if let Some(name) = payload.name {
        let _ = conn.execute("UPDATE routers SET name = ?1 WHERE id = ?2", params![name, router_id]);
    }
    if let Some(host) = payload.host {
        let _ = conn.execute("UPDATE routers SET host = ?1 WHERE id = ?2", params![host, router_id]);
    }
    if let Some(proto) = payload.proto {
        let _ = conn.execute("UPDATE routers SET proto = ?1 WHERE id = ?2", params![proto, router_id]);
    }
    if let Some(port) = payload.port {
        let _ = conn.execute("UPDATE routers SET port = ?1 WHERE id = ?2", params![port, router_id]);
    }
    if let Some(username) = payload.username {
        let _ = conn.execute("UPDATE routers SET username = ?1 WHERE id = ?2", params![username, router_id]);
    }
    if let Some(password) = payload.password {
        let sbox = SecretBox::new(&state.settings.secret_key);
        let secret_enc = sbox.encrypt(&password);
        let _ = conn.execute("UPDATE routers SET secret_enc = ?1 WHERE id = ?2", params![secret_enc, router_id]);
    }
    if let Some(tls_verify) = payload.tls_verify {
        let _ = conn.execute("UPDATE routers SET tls_verify = ?1 WHERE id = ?2", params![tls_verify, router_id]);
    }
    if let Some(enabled) = payload.enabled {
        let _ = conn.execute("UPDATE routers SET enabled = ?1 WHERE id = ?2", params![enabled, router_id]);
    }

    get_router(headers, State(state), Path(router_id)).await.into_response()
}

pub async fn delete_router_impact(headers: HeaderMap, State(state): State<AppState>, Path(router_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let peers_count: i64 = conn.query_row("SELECT COUNT(*) FROM peers WHERE router_id = ?1", params![router_id], |r| r.get(0)).unwrap_or(0);
    let samples_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM usage_samples s JOIN peers p ON s.peer_id = p.id WHERE p.router_id = ?1",
        params![router_id],
        |r| r.get(0),
    ).unwrap_or(0);

    (StatusCode::OK, Json(RouterDeleteImpactDTO { peers_count, samples_count })).into_response()
}

pub async fn delete_router(headers: HeaderMap, State(state): State<AppState>, Path(router_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let _ = conn.execute("DELETE FROM routers WHERE id = ?1", params![router_id]);
    (StatusCode::OK, Json(serde_json::json!({"status": "deleted", "router_id": router_id}))).into_response()
}

pub async fn test_router(headers: HeaderMap, State(state): State<AppState>, Path(router_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let router_res = conn.query_row(
        "SELECT id, name, host, proto, port, username, secret_enc, tls_verify, enabled, ros_version, ros_supported FROM routers WHERE id = ?1",
        params![router_id],
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
    );

    let router = match router_res {
        Ok(r) => r,
        Err(_) => return (StatusCode::NOT_FOUND, "Router not found").into_response(),
    };

    let client = make_client(&router, &state.settings.secret_key, Some(6));
    match client.get_system_version().await {
        Ok(ver) => {
            let supp = is_routeros_supported(Some(&ver));
            let now_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
            let _ = conn.execute(
                "UPDATE routers SET ros_version = ?1, ros_version_checked_at = ?2, ros_supported = ?3 WHERE id = ?4",
                params![ver, now_str, supp, router_id],
            );
            (StatusCode::OK, Json(RouterTestResp {
                success: true,
                version: ver,
                supported: supp,
                error: None,
            })).into_response()
        }
        Err(e) => (StatusCode::OK, Json(RouterTestResp {
            success: false,
            version: String::new(),
            supported: false,
            error: Some(e),
        })).into_response(),
    }
}

pub async fn list_router_interfaces(headers: HeaderMap, State(state): State<AppState>, Path(router_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let router = match get_router_by_id(&conn, router_id) {
        Some(r) => r,
        None => return (StatusCode::NOT_FOUND, "Router not found").into_response(),
    };

    let client = make_client(&router, &state.settings.secret_key, Some(10));
    match client.list_wireguard_interfaces().await {
        Ok(ifaces) => (StatusCode::OK, Json(ifaces)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub async fn get_router_interface(headers: HeaderMap, State(state): State<AppState>, Path((router_id, iface)): Path<(i64, String)>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let router = match get_router_by_id(&conn, router_id) {
        Some(r) => r,
        None => return (StatusCode::NOT_FOUND, "Router not found").into_response(),
    };

    let client = make_client(&router, &state.settings.secret_key, Some(10));
    match client.get_wireguard_interface(&iface).await {
        Ok(cfg) => (StatusCode::OK, Json(cfg)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub async fn list_router_peers(headers: HeaderMap, State(state): State<AppState>, Path(router_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let router = match get_router_by_id(&conn, router_id) {
        Some(r) => r,
        None => return (StatusCode::NOT_FOUND, "Router not found").into_response(),
    };

    let client = make_client(&router, &state.settings.secret_key, Some(10));
    match client.list_all_wireguard_peers().await {
        Ok(peers) => (StatusCode::OK, Json(peers)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub async fn import_router_peers(headers: HeaderMap, State(state): State<AppState>, Path(router_id): Path<i64>, Json(payload): Json<ImportPeersReq>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let router = match get_router_by_id(&conn, router_id) {
        Some(r) => r,
        None => return (StatusCode::NOT_FOUND, "Router not found").into_response(),
    };

    let client = make_client(&router, &state.settings.secret_key, Some(10));
    let live_peers = match client.list_all_wireguard_peers().await {
        Ok(p) => p,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };

    let now_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let mut imported = 0;
    for lp in live_peers {
        if payload.public_keys.contains(&lp.public_key) {
            let _ = conn.execute(
                r#"
                INSERT INTO peers (router_id, interface, ros_id, name, public_key, allowed_address, disabled, selected, router_sync_status, router_sync_first_seen_at, router_sync_last_seen_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, 'synced', ?8, ?8)
                ON CONFLICT(router_id, interface, public_key) DO UPDATE SET
                    selected = 1,
                    router_sync_status = 'synced',
                    ros_id = excluded.ros_id,
                    name = excluded.name,
                    allowed_address = excluded.allowed_address
                "#,
                params![router_id, lp.interface, lp.ros_id, lp.name, lp.public_key, lp.allowed_address, lp.disabled, now_str],
            );
            imported += 1;
        }
    }

    (StatusCode::OK, Json(serde_json::json!({"imported": imported}))).into_response()
}

pub async fn add_router_peer(headers: HeaderMap, State(state): State<AppState>, Path(router_id): Path<i64>, Json(payload): Json<AddPeerReq>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let router = match get_router_by_id(&conn, router_id) {
        Some(r) => r,
        None => return (StatusCode::NOT_FOUND, "Router not found").into_response(),
    };

    let client = make_client(&router, &state.settings.secret_key, Some(10));
    let ros_id = match client.add_wireguard_peer(
        &payload.interface,
        &payload.public_key,
        &payload.allowed_address,
        &payload.name,
        payload.disabled.unwrap_or(false),
        payload.private_key.as_deref(),
        payload.preshared_key.as_deref(),
        payload.client_endpoint.as_deref(),
    ).await {
        Ok(id) => id,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };

    let now_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let _ = conn.execute(
        r#"
        INSERT INTO peers (router_id, interface, ros_id, name, public_key, allowed_address, comment, disabled, selected, router_sync_status, router_sync_first_seen_at, router_sync_last_seen_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, 'synced', ?9, ?9)
        "#,
        params![router_id, payload.interface, ros_id, payload.name, payload.public_key, payload.allowed_address, payload.comment.unwrap_or_default(), payload.disabled.unwrap_or(false), now_str],
    );

    let peer_id = conn.last_insert_rowid();
    (StatusCode::OK, Json(serde_json::json!({
        "id": peer_id,
        "router_id": router_id,
        "interface": payload.interface,
        "ros_id": ros_id,
        "name": payload.name,
        "public_key": payload.public_key,
        "allowed_address": payload.allowed_address,
    }))).into_response()
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
