use super::auth::{get_current_user, AppState};
use crate::accounting::deltas::counter_day_key;
use crate::calendar::parse_timezone;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct DashboardLiveStatusDTO {
    pub peer_id: i64,
    pub online: bool,
    pub raw_last_handshake: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DashboardMetricsDTO {
    pub today_rx: i64,
    pub today_tx: i64,
    pub month_rx: i64,
    pub month_tx: i64,
    pub alltime_rx: i64,
    pub alltime_tx: i64,
    pub active_peers: i64,
    pub total_peers: i64,
}

#[derive(Debug, Default, Deserialize)]
pub struct LiveStatusQuery {
    pub router_id: Option<i64>,
    pub router_ids: Option<String>,
}

#[derive(Debug, Clone)]
struct PeerLookup {
    id: i64,
    router_id: i64,
    interface: String,
    public_key: String,
    disabled: bool,
}

pub async fn get_dashboard_live_status(
    headers: HeaderMap,
    Query(query): Query<LiveStatusQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let (db_peers, routers, online_threshold) = {
        let conn = match state.pool.get() {
            Ok(c) => c,
            Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
        };

        let online_threshold: i64 = conn.query_row(
            "SELECT value FROM settings_kv WHERE key = 'online_threshold_seconds'",
            [],
            |row| row.get::<_, String>(0),
        ).ok().and_then(|v| v.parse().ok()).unwrap_or(state.settings.online_threshold_seconds as i64);

        // Build router filter
        let mut router_id_list = Vec::new();
        if let Some(rid) = query.router_id {
            router_id_list.push(rid);
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

        let mut sql = "SELECT id, router_id, interface, public_key, disabled FROM peers WHERE selected = 1".to_string();
        if !router_id_list.is_empty() {
            let in_clause = router_id_list.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
            sql.push_str(&format!(" AND router_id IN ({})", in_clause));
        }

        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
        };

        let peer_rows = stmt.query_map([], |row| {
            Ok(PeerLookup {
                id: row.get(0)?,
                router_id: row.get(1)?,
                interface: row.get(2)?,
                public_key: row.get(3)?,
                disabled: row.get(4)?,
            })
        });

        let mut db_peers = Vec::new();
        if let Ok(rows) = peer_rows {
            for r in rows {
                if let Ok(p) = r {
                    db_peers.push(p);
                }
            }
        }

        // Get unique router IDs involved
        let mut distinct_router_ids: Vec<i64> = db_peers.iter().map(|p| p.router_id).collect();
        distinct_router_ids.sort_unstable();
        distinct_router_ids.dedup();

        let mut routers = Vec::new();
        for rid in distinct_router_ids {
            let r_row = conn.query_row(
                "SELECT id, name, host, proto, port, username, secret_enc, tls_verify, enabled, ros_version, ros_supported FROM routers WHERE id = ?1 AND enabled = 1 AND ros_supported = 1",
                params![rid],
                |row| Ok(crate::db::models::Router {
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
            if let Ok(r) = r_row {
                routers.push(r);
            }
        }

        (db_peers, routers, online_threshold)
    };

    if db_peers.is_empty() {
        return (StatusCode::OK, Json(Vec::<DashboardLiveStatusDTO>::new())).into_response();
    }

    // Fetch live peers from routers concurrently with a 4s timeout
    let mut live_results: Vec<(i64, Vec<crate::routeros::WGPeer>)> = Vec::new();
    let mut tasks = tokio::task::JoinSet::new();

    for router in routers {
        let sec_key = state.settings.secret_key.clone();
        tasks.spawn(async move {
            let client = crate::routeros::factory::make_client(&router, &sec_key, Some(4));
            let peers = client.list_all_wireguard_peers().await.unwrap_or_default();
            (router.id, peers)
        });
    }

    while let Some(res) = tasks.join_next().await {
        if let Ok((router_id, peers)) = res {
            live_results.push((router_id, peers));
        }
    }

    let mut out = Vec::new();
    for peer in db_peers {
        let mut matched_handshake = 0;
        let mut peer_disabled = peer.disabled;

        if let Some((_, live_peers)) = live_results.iter().find(|(rid, _)| *rid == peer.router_id) {
            if let Some(lp) = live_peers.iter().find(|lp| lp.interface == peer.interface && lp.public_key == peer.public_key) {
                matched_handshake = lp.last_handshake.unwrap_or(0);
                peer_disabled = lp.disabled || peer.disabled;
            }
        }

        let is_online = !peer_disabled && matched_handshake > 0 && matched_handshake <= online_threshold;
        out.push(DashboardLiveStatusDTO {
            peer_id: peer.id,
            online: is_online,
            raw_last_handshake: matched_handshake,
        });
    }

    (StatusCode::OK, Json(out)).into_response()
}

pub async fn get_dashboard_metrics(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let now_utc = Utc::now();
    let tz = parse_timezone(&state.settings.timezone);
    let day_key = counter_day_key(now_utc, tz);
    let month_key = now_utc.format("%Y-%m").to_string();

    let today: (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(rx), 0), COALESCE(SUM(tx), 0) FROM usage_daily WHERE day = ?1",
        params![day_key],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap_or((0, 0));

    let month: (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(rx), 0), COALESCE(SUM(tx), 0) FROM usage_monthly WHERE month_key = ?1",
        params![month_key],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap_or((0, 0));

    let alltime: (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(rx), 0), COALESCE(SUM(tx), 0) FROM usage_monthly",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap_or((0, 0));

    let total_peers: i64 = conn.query_row("SELECT COUNT(*) FROM peers WHERE selected = 1", [], |r| r.get(0)).unwrap_or(0);

    (StatusCode::OK, Json(DashboardMetricsDTO {
        today_rx: today.0,
        today_tx: today.1,
        month_rx: month.0,
        month_tx: month.1,
        alltime_rx: alltime.0,
        alltime_tx: alltime.1,
        active_peers: total_peers,
        total_peers,
    })).into_response()
}

pub async fn get_last_actions(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let mut stmt = conn.prepare("SELECT id, peer_id, ts, action, note FROM actions ORDER BY ts DESC LIMIT 50").unwrap();
    let rows = stmt.query_map([], |row| {
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
