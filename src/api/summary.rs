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
    pub rx: i64,
    pub tx: i64,
    pub total_rx: i64,
    pub total_tx: i64,
    #[serde(default)]
    pub has_fair_usage: bool,
    #[serde(default)]
    pub fair_usage_throttled: bool,
}

#[derive(Debug, Default, Deserialize)]
pub struct PeersSummaryQuery {
    pub days: Option<i64>,
    pub seconds: Option<i64>,
    pub router_id: Option<i64>,
    pub router_ids: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub all_time: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawSummaryQuery {
    pub seconds: Option<i64>,
    pub router_id: Option<i64>,
    pub router_ids: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub interval_seconds: Option<i64>,
}

pub async fn get_summary_month(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };
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

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };
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

pub async fn get_summary_peers(
    headers: HeaderMap,
    Query(query): Query<PeersSummaryQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let now_utc = Utc::now();
    let tz = parse_timezone(&state.settings.timezone);

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

    let router_filter = if !router_id_list.is_empty() {
        let in_clause = router_id_list.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
        format!(" AND p.router_id IN ({})", in_clause)
    } else {
        String::new()
    };

    let mut summary_map: std::collections::HashMap<i64, (i64, i64)> = std::collections::HashMap::new();

    if let Some(secs) = query.seconds {
        if secs > 0 {
            let start_str = (now_utc - chrono::Duration::seconds(secs)).format("%Y-%m-%d %H:%M:%S").to_string();
            let end_str = now_utc.format("%Y-%m-%d %H:%M:%S").to_string();

            let sql = format!(
                r#"
                SELECT m.peer_id, COALESCE(SUM(m.rx), 0), COALESCE(SUM(m.tx), 0)
                FROM usage_minute m
                JOIN peers p ON p.id = m.peer_id
                WHERE p.selected = 1 {}
                  AND m.minute_ts >= ?1 AND m.minute_ts <= ?2
                GROUP BY m.peer_id
                "#,
                router_filter
            );

            if let Ok(mut stmt) = conn.prepare(&sql) {
                if let Ok(rows) = stmt.query_map(params![start_str, end_str], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
                }) {
                    for r in rows.flatten() {
                        summary_map.insert(r.0, (r.1, r.2));
                    }
                }
            }
        }
    } else if query.all_time.unwrap_or(false) {
        let sql = format!(
            r#"
            SELECT m.peer_id, COALESCE(SUM(m.rx), 0), COALESCE(SUM(m.tx), 0)
            FROM usage_monthly m
            JOIN peers p ON p.id = m.peer_id
            WHERE p.selected = 1 {}
            GROUP BY m.peer_id
            "#,
            router_filter
        );

        if let Ok(mut stmt) = conn.prepare(&sql) {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
            }) {
                for r in rows.flatten() {
                    summary_map.insert(r.0, (r.1, r.2));
                }
            }
        }
    } else {
        // Daily window
        let days = query.days.unwrap_or(1).max(1);
        let start_day = (now_utc - chrono::Duration::days(days - 1)).with_timezone(&tz).format("%Y-%m-%d").to_string();

        let sql = format!(
            r#"
            SELECT d.peer_id, COALESCE(SUM(d.rx), 0), COALESCE(SUM(d.tx), 0)
            FROM usage_daily d
            JOIN peers p ON p.id = d.peer_id
            WHERE p.selected = 1 {}
              AND d.day >= ?1
            GROUP BY d.peer_id
            "#,
            router_filter
        );

        if let Ok(mut stmt) = conn.prepare(&sql) {
            if let Ok(rows) = stmt.query_map(params![start_day], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
            }) {
                for r in rows.flatten() {
                    summary_map.insert(r.0, (r.1, r.2));
                }
            }
        }
    }

    // Get all selected peers in scope
    let scope_sql = format!("SELECT id FROM peers p WHERE p.selected = 1 {}", router_filter);
    let mut all_peer_ids = Vec::new();
    if let Ok(mut stmt) = conn.prepare(&scope_sql) {
        if let Ok(rows) = stmt.query_map([], |row| row.get::<_, i64>(0)) {
            for id in rows.flatten() {
                all_peer_ids.push(id);
                summary_map.entry(id).or_insert((0, 0));
            }
        }
    }

    // Fair usage status
    let mut fu_applicable = std::collections::HashSet::new();
    let mut fu_throttled = std::collections::HashMap::new();

    if !all_peer_ids.is_empty() {
        let in_clause = all_peer_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
        let fu_sql = format!(
            "SELECT peer_id, throttled FROM fair_usage_state WHERE peer_id IN ({})",
            in_clause
        );
        if let Ok(mut stmt) = conn.prepare(&fu_sql) {
            if let Ok(rows) = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?))) {
                for r in rows.flatten() {
                    fu_applicable.insert(r.0);
                    fu_throttled.insert(r.0, r.1);
                }
            }
        }

        let fa_sql = format!(
            "SELECT peer_id FROM fair_usage_assignments WHERE peer_id IN ({})",
            in_clause
        );
        if let Ok(mut stmt) = conn.prepare(&fa_sql) {
            if let Ok(rows) = stmt.query_map([], |row| row.get::<_, i64>(0)) {
                for id in rows.flatten() {
                    fu_applicable.insert(id);
                }
            }
        }
    }

    let mut list = Vec::new();
    for (pid, (rx, tx)) in summary_map {
        let has_fu = fu_applicable.contains(&pid);
        let throttled = *fu_throttled.get(&pid).unwrap_or(&false);
        list.push(PeerUsageSummaryDTO {
            peer_id: pid,
            rx,
            tx,
            total_rx: rx,
            total_tx: tx,
            has_fair_usage: has_fu,
            fair_usage_throttled: throttled,
        });
    }

    (StatusCode::OK, Json(list)).into_response()
}

pub async fn get_summary_raw(
    headers: HeaderMap,
    Query(q): Query<RawSummaryQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let seconds = q.seconds.unwrap_or(86400).max(60);
    let start_str = q.start.unwrap_or_else(|| (Utc::now() - chrono::Duration::seconds(seconds)).format("%Y-%m-%d %H:%M:%S").to_string());
    let end_str = q.end.unwrap_or_else(|| Utc::now().format("%Y-%m-%d %H:%M:%S").to_string());

    let mut router_id_list = Vec::new();
    if let Some(rid) = q.router_id {
        router_id_list.push(rid);
    }
    if let Some(ref rids_str) = q.router_ids {
        for part in rids_str.split(',') {
            if let Ok(id) = part.trim().parse::<i64>() {
                if !router_id_list.contains(&id) {
                    router_id_list.push(id);
                }
            }
        }
    }

    let router_filter = if !router_id_list.is_empty() {
        let in_clause = router_id_list.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
        format!(" AND p.router_id IN ({})", in_clause)
    } else {
        String::new()
    };

    let sql = format!(
        r#"
        SELECT u.minute_ts, COALESCE(SUM(u.rx), 0), COALESCE(SUM(u.tx), 0)
        FROM usage_minute u
        JOIN peers p ON u.peer_id = p.id
        WHERE p.selected = 1 {}
          AND u.minute_ts >= ?1 AND u.minute_ts <= ?2
        GROUP BY u.minute_ts
        ORDER BY u.minute_ts ASC
        "#,
        router_filter
    );

    let mut list = Vec::new();
    if let Ok(mut stmt) = conn.prepare(&sql) {
        if let Ok(rows) = stmt.query_map(params![start_str, end_str], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
        }) {
            for r in rows.flatten() {
                list.push(serde_json::json!({
                    "ts": r.0,
                    "timestamp": r.0,
                    "rx": r.1,
                    "tx": r.2,
                }));
            }
        }
    }

    (StatusCode::OK, Json(list)).into_response()
}

pub async fn get_summary_raw_by_router(
    headers: HeaderMap,
    Query(q): Query<RawSummaryQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let seconds = q.seconds.unwrap_or(86400).max(60);
    let start_str = q.start.unwrap_or_else(|| (Utc::now() - chrono::Duration::seconds(seconds)).format("%Y-%m-%d %H:%M:%S").to_string());
    let end_str = q.end.unwrap_or_else(|| Utc::now().format("%Y-%m-%d %H:%M:%S").to_string());

    let mut router_id_list = Vec::new();
    if let Some(rid) = q.router_id {
        router_id_list.push(rid);
    }
    if let Some(ref rids_str) = q.router_ids {
        for part in rids_str.split(',') {
            if let Ok(id) = part.trim().parse::<i64>() {
                if !router_id_list.contains(&id) {
                    router_id_list.push(id);
                }
            }
        }
    }

    let router_filter = if !router_id_list.is_empty() {
        let in_clause = router_id_list.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
        format!(" AND p.router_id IN ({})", in_clause)
    } else {
        String::new()
    };

    let sql = format!(
        r#"
        SELECT p.router_id, u.minute_ts, COALESCE(SUM(u.rx), 0), COALESCE(SUM(u.tx), 0)
        FROM usage_minute u
        JOIN peers p ON u.peer_id = p.id
        WHERE p.selected = 1 {}
          AND u.minute_ts >= ?1 AND u.minute_ts <= ?2
        GROUP BY p.router_id, u.minute_ts
        ORDER BY p.router_id ASC, u.minute_ts ASC
        "#,
        router_filter
    );

    let mut list = Vec::new();
    if let Ok(mut stmt) = conn.prepare(&sql) {
        if let Ok(rows) = stmt.query_map(params![start_str, end_str], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?))
        }) {
            for r in rows.flatten() {
                list.push(serde_json::json!({
                    "router_id": r.0,
                    "ts": r.1,
                    "timestamp": r.1,
                    "rx": r.2,
                    "tx": r.3,
                }));
            }
        }
    }

    (StatusCode::OK, Json(list)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::initialize_database;

    #[test]
    fn test_peer_usage_daily_and_minute_aggregation() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let _ = initialize_database(&conn);

        // Insert router
        conn.execute(
            "INSERT INTO routers (name, host, proto, port, username, secret_enc, tls_verify, enabled, ros_version, ros_supported) VALUES ('R1', '192.168.88.1', 'rest', 443, 'admin', 'enc', 0, 1, '7.12', 1)",
            [],
        ).unwrap();

        // Insert peers
        conn.execute(
            "INSERT INTO peers (router_id, interface, ros_id, name, public_key, allowed_address, disabled, selected, router_sync_status) VALUES (1, 'wg0', '*1', 'Client1', 'pub1', '10.0.0.2/32', 0, 1, 'synced')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO peers (router_id, interface, ros_id, name, public_key, allowed_address, disabled, selected, router_sync_status) VALUES (1, 'wg0', '*2', 'Client2', 'pub2', '10.0.0.3/32', 0, 1, 'synced')",
            [],
        ).unwrap();

        // Insert usage daily
        let day_str = Utc::now().format("%Y-%m-%d").to_string();
        conn.execute(
            "INSERT INTO usage_daily (peer_id, day, rx, tx) VALUES (1, ?1, 1048576, 2097152)",
            params![day_str],
        ).unwrap();

        // Insert usage minute
        let min_str = Utc::now().format("%Y-%m-%d %H:%M:00").to_string();
        conn.execute(
            "INSERT INTO usage_minute (peer_id, minute_ts, rx, tx) VALUES (1, ?1, 524288, 1048576)",
            params![min_str],
        ).unwrap();

        // Verify daily query
        let daily_sum: (i64, i64) = conn.query_row(
            "SELECT COALESCE(SUM(rx), 0), COALESCE(SUM(tx), 0) FROM usage_daily WHERE peer_id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(daily_sum.0, 1048576);
        assert_eq!(daily_sum.1, 2097152);

        // Verify minute query
        let min_sum: (i64, i64) = conn.query_row(
            "SELECT COALESCE(SUM(rx), 0), COALESCE(SUM(tx), 0) FROM usage_minute WHERE peer_id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(min_sum.0, 524288);
        assert_eq!(min_sum.1, 1048576);
    }
}
