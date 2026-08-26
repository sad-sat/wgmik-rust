use super::auth::{get_current_user, AppState};
use crate::calendar::parse_timezone;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SummaryPointDTO {
    pub date: String,
    pub rx: i64,
    pub tx: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RouterSummaryPointDTO {
    pub router_id: i64,
    pub date: String,
    pub rx: i64,
    pub tx: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PeerUsageSummaryDTO {
    pub peer_id: i64,
    pub name: String,
    pub interface: String,
    pub router_id: i64,
    pub today_rx: i64,
    pub today_tx: i64,
    pub month_rx: i64,
    pub month_tx: i64,
    pub alltime_rx: i64,
    pub alltime_tx: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TimeRangeQuery {
    pub start: Option<String>,
    pub end: Option<String>,
    pub interval_seconds: Option<i64>,
}

pub async fn get_summary_month(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let now_utc = Utc::now();
    let month_prefix = now_utc.format("%Y-%m").to_string();

    let mut stmt = conn.prepare(
        "SELECT day, COALESCE(SUM(rx), 0), COALESCE(SUM(tx), 0)
         FROM usage_daily
         WHERE day LIKE ?1 || '%'
         GROUP BY day
         ORDER BY day ASC",
    ).unwrap();

    let rows = stmt.query_map(params![month_prefix], |row| {
        Ok(SummaryPointDTO {
            date: row.get(0)?,
            rx: row.get(1)?,
            tx: row.get(2)?,
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

pub async fn get_summary_month_by_router(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let now_utc = Utc::now();
    let month_prefix = now_utc.format("%Y-%m").to_string();

    let mut stmt = conn.prepare(
        "SELECT p.router_id, d.day, COALESCE(SUM(d.rx), 0), COALESCE(SUM(d.tx), 0)
         FROM usage_daily d
         JOIN peers p ON d.peer_id = p.id
         WHERE d.day LIKE ?1 || '%'
         GROUP BY p.router_id, d.day
         ORDER BY p.router_id ASC, d.day ASC",
    ).unwrap();

    let rows = stmt.query_map(params![month_prefix], |row| {
        Ok(RouterSummaryPointDTO {
            router_id: row.get(0)?,
            date: row.get(1)?,
            rx: row.get(2)?,
            tx: row.get(3)?,
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

pub async fn get_summary_peers(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let now_utc = Utc::now();
    let tz = parse_timezone(&state.settings.timezone);
    let day_key = crate::accounting::deltas::counter_day_key(now_utc, tz);
    let month_key = now_utc.format("%Y-%m").to_string();

    let mut stmt = conn.prepare(
        r#"
        SELECT p.id, p.name, p.interface, p.router_id,
               COALESCE(d.rx, 0), COALESCE(d.tx, 0),
               COALESCE(m.rx, 0), COALESCE(m.tx, 0),
               (SELECT COALESCE(SUM(rx), 0) FROM usage_monthly WHERE peer_id = p.id),
               (SELECT COALESCE(SUM(tx), 0) FROM usage_monthly WHERE peer_id = p.id)
        FROM peers p
        LEFT JOIN usage_daily d ON p.id = d.peer_id AND d.day = ?1
        LEFT JOIN usage_monthly m ON p.id = m.peer_id AND m.month_key = ?2
        WHERE p.selected = 1
        ORDER BY p.id ASC
        "#,
    ).unwrap();

    let rows = stmt.query_map(params![day_key, month_key], |row| {
        Ok(PeerUsageSummaryDTO {
            peer_id: row.get(0)?,
            name: row.get(1)?,
            interface: row.get(2)?,
            router_id: row.get(3)?,
            today_rx: row.get(4)?,
            today_tx: row.get(5)?,
            month_rx: row.get(6)?,
            month_tx: row.get(7)?,
            alltime_rx: row.get(8)?,
            alltime_tx: row.get(9)?,
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

pub async fn get_summary_raw(headers: HeaderMap, State(state): State<AppState>, Query(q): Query<TimeRangeQuery>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let start_str = q.start.unwrap_or_else(|| (Utc::now() - chrono::Duration::hours(24)).format("%Y-%m-%d %H:%M:%S").to_string());
    let end_str = q.end.unwrap_or_else(|| Utc::now().format("%Y-%m-%d %H:%M:%S").to_string());

    let mut stmt = conn.prepare(
        "SELECT minute_ts, COALESCE(SUM(rx), 0), COALESCE(SUM(tx), 0)
         FROM usage_minute
         WHERE minute_ts >= ?1 AND minute_ts <= ?2
         GROUP BY minute_ts
         ORDER BY minute_ts ASC",
    ).unwrap();

    let rows = stmt.query_map(params![start_str, end_str], |row| {
        let ts_str: String = row.get(0)?;
        let rx: i64 = row.get(1)?;
        let tx: i64 = row.get(2)?;
        Ok((ts_str, rx, tx))
    }).unwrap();

    let mut list = Vec::new();
    for r in rows {
        if let Ok((ts, rx, tx)) = r {
            list.push(serde_json::json!({
                "timestamp": ts,
                "rx": rx,
                "tx": tx,
            }));
        }
    }

    (StatusCode::OK, Json(list)).into_response()
}

pub async fn get_summary_raw_by_router(headers: HeaderMap, State(state): State<AppState>, Query(q): Query<TimeRangeQuery>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let start_str = q.start.unwrap_or_else(|| (Utc::now() - chrono::Duration::hours(24)).format("%Y-%m-%d %H:%M:%S").to_string());
    let end_str = q.end.unwrap_or_else(|| Utc::now().format("%Y-%m-%d %H:%M:%S").to_string());

    let mut stmt = conn.prepare(
        "SELECT p.router_id, u.minute_ts, COALESCE(SUM(u.rx), 0), COALESCE(SUM(u.tx), 0)
         FROM usage_minute u
         JOIN peers p ON u.peer_id = p.id
         WHERE u.minute_ts >= ?1 AND u.minute_ts <= ?2
         GROUP BY p.router_id, u.minute_ts
         ORDER BY p.router_id ASC, u.minute_ts ASC",
    ).unwrap();

    let rows = stmt.query_map(params![start_str, end_str], |row| {
        Ok(serde_json::json!({
            "router_id": row.get::<_, i64>(0)?,
            "timestamp": row.get::<_, String>(1)?,
            "rx": row.get::<_, i64>(2)?,
            "tx": row.get::<_, i64>(3)?,
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
