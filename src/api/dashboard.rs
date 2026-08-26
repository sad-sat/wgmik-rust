use super::auth::{get_current_user, AppState};
use crate::accounting::deltas::counter_day_key;
use crate::calendar::parse_timezone;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct DashboardLiveStatusDTO {
    pub peer_id: i64,
    pub router_id: i64,
    pub name: String,
    pub interface: String,
    pub is_online: bool,
    pub last_handshake_ago: Option<i64>,
    pub rx_bytes: i64,
    pub tx_bytes: i64,
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

pub async fn get_dashboard_live_status(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let mut stmt = conn.prepare("SELECT id, router_id, name, interface FROM peers WHERE selected = 1").unwrap();
    let rows = stmt.query_map([], |row| {
        Ok(DashboardLiveStatusDTO {
            peer_id: row.get(0)?,
            router_id: row.get(1)?,
            name: row.get(2)?,
            interface: row.get(3)?,
            is_online: false,
            last_handshake_ago: None,
            rx_bytes: 0,
            tx_bytes: 0,
        })
    }).unwrap();

    let mut list = Vec::new();
    for r in rows {
        if let Ok(item) = r {
            list.push(item);
        }
    }

    (StatusCode::OK, Json(list)).into_response()
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
