use super::auth::{get_current_user, AppState};
use crate::crypto::SecretBox;
use crate::db::models::Router;
use crate::routeros::factory::make_client;
use crate::routeros::version::is_routeros_supported;
use axum::extract::{Json, Path, Query, State};
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
    pub ok: bool,
    pub ros_version: String,
    pub ros_version_checked_at: String,
    pub ros_supported: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WGInterfaceDTO {
    pub name: String,
    pub public_key: String,
    pub listen_port: u16,
    pub public_host: String,
    pub addresses: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RouterDeleteImpactDTO {
    pub peers_count: i64,
    pub samples_count: i64,
}

#[derive(Debug, Default, Deserialize)]
pub struct RouterPeersQuery {
    pub interface: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerImportItem {
    #[serde(default)]
    pub interface: Option<String>,
    #[serde(default)]
    pub public_key: String,
    #[serde(default)]
    pub selected: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PeerImportEntry {
    Item(PeerImportItem),
    Tuple(String, String),
    Key(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ImportPeersReq {
    List(Vec<PeerImportEntry>),
    ObjectItems { items: Vec<PeerImportEntry> },
    ObjectPeers { peers: Vec<PeerImportEntry> },
    ObjectKeys { public_keys: Vec<String> },
    Single(PeerImportEntry),
}

impl ImportPeersReq {
    pub fn into_items(self) -> Vec<PeerImportItem> {
        let entries = match self {
            ImportPeersReq::List(list) => list,
            ImportPeersReq::ObjectItems { items } => items,
            ImportPeersReq::ObjectPeers { peers } => peers,
            ImportPeersReq::ObjectKeys { public_keys } => {
                return public_keys
                    .into_iter()
                    .map(|k| PeerImportItem {
                        interface: None,
                        public_key: k,
                        selected: Some(true),
                    })
                    .collect();
            }
            ImportPeersReq::Single(entry) => vec![entry],
        };

        entries
            .into_iter()
            .map(|entry| match entry {
                PeerImportEntry::Item(item) => item,
                PeerImportEntry::Tuple(iface, key) => PeerImportItem {
                    interface: Some(iface),
                    public_key: key,
                    selected: Some(true),
                },
                PeerImportEntry::Key(key) => PeerImportItem {
                    interface: None,
                    public_key: key,
                    selected: Some(true),
                },
            })
            .collect()
    }
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

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"detail": format!("Database error: {}", e)}))).into_response(),
    };
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
        Err(_) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": "Router not found"}))).into_response(),
    };

    let client = make_client(&router, &state.settings.secret_key, Some(8));
    let ver = match client.get_system_version().await {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"detail": format!("Router connection failed: {}", e)})),
            ).into_response();
        }
    };

    let supp = is_routeros_supported(Some(&ver));
    let now_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let _ = conn.execute(
        "UPDATE routers SET ros_version = ?1, ros_version_checked_at = ?2, ros_supported = ?3 WHERE id = ?4",
        params![ver, now_str, supp, router_id],
    );

    if !supp {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"detail": format!("RouterOS 7.15 or newer is required (detected: {})", ver)})),
        ).into_response();
    }

    // Verify WireGuard interface querying
    if let Err(e) = client.list_wireguard_interfaces().await {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"detail": format!("Connected to router, but WireGuard interfaces query failed: {}", e)})),
        ).into_response();
    }

    (StatusCode::OK, Json(RouterTestResp {
        ok: true,
        ros_version: ver,
        ros_version_checked_at: now_str,
        ros_supported: true,
    })).into_response()
}

pub async fn list_router_interfaces(headers: HeaderMap, State(state): State<AppState>, Path(router_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"detail": format!("Database error: {}", e)}))).into_response(),
    };
    let router = match get_router_by_id(&conn, router_id) {
        Some(r) => r,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": "Router not found"}))).into_response(),
    };

    let client = make_client(&router, &state.settings.secret_key, Some(10));
    match client.list_wireguard_interfaces().await {
        Ok(ifaces) => (StatusCode::OK, Json(ifaces)).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"detail": format!("Failed to list interfaces: {}", e)}))).into_response(),
    }
}

pub async fn get_router_interface(headers: HeaderMap, State(state): State<AppState>, Path((router_id, iface)): Path<(i64, String)>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"detail": format!("Database error: {}", e)}))).into_response(),
    };
    let router = match get_router_by_id(&conn, router_id) {
        Some(r) => r,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": "Router not found"}))).into_response(),
    };

    let client = make_client(&router, &state.settings.secret_key, Some(10));
    match client.get_wireguard_interface(&iface).await {
        Ok(cfg) => {
            let primary_host = client.get_primary_ipv4().await.unwrap_or_default();
            let host = if !primary_host.is_empty() {
                primary_host
            } else {
                router.host.clone()
            };
            let dto = WGInterfaceDTO {
                name: cfg.name,
                public_key: cfg.public_key,
                listen_port: cfg.listen_port,
                public_host: host,
                addresses: cfg.addresses,
            };
            (StatusCode::OK, Json(dto)).into_response()
        }
        Err(e) => (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"detail": format!("Failed to get interface details: {}", e)}))).into_response(),
    }
}

pub async fn list_router_peers(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(router_id): Path<i64>,
    Query(query): Query<RouterPeersQuery>,
) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let router = match get_router_by_id(&conn, router_id) {
        Some(r) => r,
        None => return (StatusCode::NOT_FOUND, "Router not found").into_response(),
    };

    let client = make_client(&router, &state.settings.secret_key, Some(10));
    let peers_res = match &query.interface {
        Some(iface) if !iface.is_empty() => client.list_wireguard_peers(iface).await,
        _ => client.list_all_wireguard_peers().await,
    };

    match peers_res {
        Ok(peers) => (StatusCode::OK, Json(peers)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub async fn import_router_peers(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(router_id): Path<i64>,
    Json(payload): Json<ImportPeersReq>,
) -> impl IntoResponse {
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

    let items = payload.into_items();
    let now_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let mut imported = 0;

    for it in items {
        let matching_live = live_peers.iter().find(|lp| {
            if let Some(ref iface) = it.interface {
                if !iface.is_empty() && lp.interface != *iface {
                    return false;
                }
            }
            lp.public_key == it.public_key
        });

        if let Some(lp) = matching_live {
            let selected_val = if it.selected.unwrap_or(true) { 1 } else { 0 };
            let disabled_val = if lp.disabled { 1 } else { 0 };
            let comment_val = lp.comment.as_deref().unwrap_or("");

            let res = conn.execute(
                r#"
                INSERT INTO peers (
                    router_id, interface, ros_id, name, public_key, allowed_address,
                    comment, disabled, selected, router_sync_status,
                    router_sync_first_seen_at, router_sync_last_seen_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'synced', ?10, ?10)
                ON CONFLICT(router_id, interface, public_key) DO UPDATE SET
                    selected = excluded.selected,
                    disabled = excluded.disabled,
                    router_sync_status = 'synced',
                    router_sync_last_seen_at = excluded.router_sync_last_seen_at,
                    ros_id = excluded.ros_id,
                    name = excluded.name,
                    allowed_address = excluded.allowed_address,
                    comment = excluded.comment
                "#,
                params![
                    router_id,
                    lp.interface,
                    lp.ros_id,
                    lp.name,
                    lp.public_key,
                    lp.allowed_address,
                    comment_val,
                    disabled_val,
                    selected_val,
                    now_str,
                ],
            );
            if res.is_ok() {
                imported += 1;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_peer_import_item_list() {
        let json_str = r#"[
            {"interface": "wg0", "public_key": "pub1", "selected": true},
            {"interface": "wg0", "public_key": "pub2", "selected": false}
        ]"#;
        let req: ImportPeersReq = serde_json::from_str(json_str).unwrap();
        let items = req.into_items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].interface.as_deref(), Some("wg0"));
        assert_eq!(items[0].public_key, "pub1");
        assert_eq!(items[0].selected, Some(true));
        assert_eq!(items[1].selected, Some(false));
    }

    #[test]
    fn test_deserialize_string_key_list() {
        let json_str = r#"["pub1", "pub2"]"#;
        let req: ImportPeersReq = serde_json::from_str(json_str).unwrap();
        let items = req.into_items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].public_key, "pub1");
        assert_eq!(items[0].selected, Some(true));
    }

    #[test]
    fn test_deserialize_object_public_keys() {
        let json_str = r#"{"public_keys": ["pub1", "pub2"]}"#;
        let req: ImportPeersReq = serde_json::from_str(json_str).unwrap();
        let items = req.into_items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].public_key, "pub1");
    }

    #[test]
    fn test_deserialize_object_items() {
        let json_str = r#"{"items": [{"interface": "wg1", "public_key": "pub1", "selected": true}]}"#;
        let req: ImportPeersReq = serde_json::from_str(json_str).unwrap();
        let items = req.into_items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].interface.as_deref(), Some("wg1"));
        assert_eq!(items[0].public_key, "pub1");
    }
}
