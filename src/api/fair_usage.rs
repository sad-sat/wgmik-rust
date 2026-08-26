use super::auth::{get_current_user, AppState};
use crate::calendar::parse_timezone;
use crate::db::models::Peer;
use crate::fair_usage::build_fair_usage_peer_status_dto;
use axum::extract::{Json, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct FairUsageRuleDTO {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub quota_mode: String,
    pub download_quota_bytes: i64,
    pub upload_quota_bytes: Option<i64>,
    pub throttle_download_kbps: i64,
    pub throttle_upload_kbps: i64,
    pub time_scope: String,
    pub scope_period_count: i64,
    pub scope_period_unit: String,
    pub scope_type: String,
    pub router_id: Option<i64>,
    pub sort_order: i64,
    pub passthrough: bool,
    pub enabled: bool,
    pub tiered: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AssignRuleReq {
    pub peer_ids: Vec<i64>,
}

pub async fn list_fair_usage_rules(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, name, description, quota_mode, download_quota_bytes, upload_quota_bytes,
                throttle_download_kbps, throttle_upload_kbps, time_scope, scope_period_count,
                scope_period_unit, scope_type, router_id, sort_order, passthrough, enabled, tiered
         FROM fair_usage_rules
         ORDER BY sort_order ASC, id ASC",
    ).unwrap();

    let rows = stmt.query_map([], |row| {
        Ok(FairUsageRuleDTO {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            quota_mode: row.get(3)?,
            download_quota_bytes: row.get(4)?,
            upload_quota_bytes: row.get(5)?,
            throttle_download_kbps: row.get(6)?,
            throttle_upload_kbps: row.get(7)?,
            time_scope: row.get(8)?,
            scope_period_count: row.get(9)?,
            scope_period_unit: row.get(10)?,
            scope_type: row.get(11)?,
            router_id: row.get(12)?,
            sort_order: row.get(13)?,
            passthrough: row.get(14)?,
            enabled: row.get(15)?,
            tiered: row.get(16)?,
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

pub async fn create_fair_usage_rule(headers: HeaderMap, State(state): State<AppState>, Json(payload): Json<FairUsageRuleDTO>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let now_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let res = conn.execute(
        r#"
        INSERT INTO fair_usage_rules (
            name, description, quota_mode, download_quota_bytes, upload_quota_bytes,
            throttle_download_kbps, throttle_upload_kbps, time_scope, scope_period_count,
            scope_period_unit, scope_type, router_id, sort_order, passthrough, enabled,
            tiered, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17)
        "#,
        params![
            payload.name, payload.description, payload.quota_mode, payload.download_quota_bytes, payload.upload_quota_bytes,
            payload.throttle_download_kbps, payload.throttle_upload_kbps, payload.time_scope, payload.scope_period_count,
            payload.scope_period_unit, payload.scope_type, payload.router_id, payload.sort_order, payload.passthrough,
            payload.enabled, payload.tiered, now_str
        ],
    );

    if res.is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create rule").into_response();
    }

    let id = conn.last_insert_rowid();
    let mut created = payload;
    created.id = id;
    (StatusCode::OK, Json(created)).into_response()
}

pub async fn get_fair_usage_rule(headers: HeaderMap, State(state): State<AppState>, Path(rule_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let row = conn.query_row(
        "SELECT id, name, description, quota_mode, download_quota_bytes, upload_quota_bytes,
                throttle_download_kbps, throttle_upload_kbps, time_scope, scope_period_count,
                scope_period_unit, scope_type, router_id, sort_order, passthrough, enabled, tiered
         FROM fair_usage_rules WHERE id = ?1",
        params![rule_id],
        |row| Ok(FairUsageRuleDTO {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            quota_mode: row.get(3)?,
            download_quota_bytes: row.get(4)?,
            upload_quota_bytes: row.get(5)?,
            throttle_download_kbps: row.get(6)?,
            throttle_upload_kbps: row.get(7)?,
            time_scope: row.get(8)?,
            scope_period_count: row.get(9)?,
            scope_period_unit: row.get(10)?,
            scope_type: row.get(11)?,
            router_id: row.get(12)?,
            sort_order: row.get(13)?,
            passthrough: row.get(14)?,
            enabled: row.get(15)?,
            tiered: row.get(16)?,
        }),
    );

    match row {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "Rule not found").into_response(),
    }
}

pub async fn update_fair_usage_rule(headers: HeaderMap, State(state): State<AppState>, Path(rule_id): Path<i64>, Json(payload): Json<FairUsageRuleDTO>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let now_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let _ = conn.execute(
        r#"
        UPDATE fair_usage_rules SET
            name = ?1, description = ?2, quota_mode = ?3, download_quota_bytes = ?4, upload_quota_bytes = ?5,
            throttle_download_kbps = ?6, throttle_upload_kbps = ?7, time_scope = ?8, scope_period_count = ?9,
            scope_period_unit = ?10, scope_type = ?11, router_id = ?12, sort_order = ?13, passthrough = ?14,
            enabled = ?15, tiered = ?16, updated_at = ?17
        WHERE id = ?18
        "#,
        params![
            payload.name, payload.description, payload.quota_mode, payload.download_quota_bytes, payload.upload_quota_bytes,
            payload.throttle_download_kbps, payload.throttle_upload_kbps, payload.time_scope, payload.scope_period_count,
            payload.scope_period_unit, payload.scope_type, payload.router_id, payload.sort_order, payload.passthrough,
            payload.enabled, payload.tiered, now_str, rule_id
        ],
    );

    (StatusCode::OK, Json(payload)).into_response()
}

pub async fn delete_fair_usage_rule(headers: HeaderMap, State(state): State<AppState>, Path(rule_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let _ = conn.execute("DELETE FROM fair_usage_rules WHERE id = ?1", params![rule_id]);
    (StatusCode::OK, Json(serde_json::json!({"status": "deleted"}))).into_response()
}

pub async fn assign_fair_usage_rule(headers: HeaderMap, State(state): State<AppState>, Path(rule_id): Path<i64>, Json(payload): Json<AssignRuleReq>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    for peer_id in payload.peer_ids {
        let _ = conn.execute(
            "INSERT INTO fair_usage_assignments (rule_id, peer_id) VALUES (?1, ?2) ON CONFLICT(rule_id, peer_id) DO NOTHING",
            params![rule_id, peer_id],
        );
    }

    get_fair_usage_rule(headers, State(state), Path(rule_id)).await.into_response()
}

pub async fn unassign_fair_usage_rule(headers: HeaderMap, State(state): State<AppState>, Path((rule_id, peer_id)): Path<(i64, i64)>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let _ = conn.execute("DELETE FROM fair_usage_assignments WHERE rule_id = ?1 AND peer_id = ?2", params![rule_id, peer_id]);
    (StatusCode::OK, Json(serde_json::json!({"status": "unassigned"}))).into_response()
}

pub async fn get_peer_fair_usage_status(headers: HeaderMap, State(state): State<AppState>, Path(peer_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let peer = match conn.query_row(
        "SELECT id, router_id, interface, ros_id, name, public_key, allowed_address, comment, disabled, selected, router_sync_status FROM peers WHERE id = ?1",
        params![peer_id],
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
    ) {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "Peer not found").into_response(),
    };

    let now_utc = Utc::now();
    let tz = parse_timezone(&state.settings.timezone);
    let calendar = "gregorian";
    let dto = build_fair_usage_peer_status_dto(&conn, &peer, now_utc, tz, calendar, state.settings.monthly_reset_day as i32);

    (StatusCode::OK, Json(dto)).into_response()
}

pub async fn reset_peer_fair_usage(headers: HeaderMap, State(state): State<AppState>, Path(peer_id): Path<i64>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let _ = conn.execute("DELETE FROM fair_usage_state WHERE peer_id = ?1", params![peer_id]);
    let ts_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let _ = conn.execute(
        "INSERT INTO actions (peer_id, ts, action, note) VALUES (?1, ?2, ?3, ?4)",
        params![peer_id, ts_str, "fu_reset", "Manual reset by admin"],
    );

    (StatusCode::OK, Json(serde_json::json!({"status": "reset"}))).into_response()
}
