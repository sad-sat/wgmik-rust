use super::auth::{get_current_user, AppState};
use axum::extract::{Json, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SettingsDTO {
    pub poll_interval_seconds: u64,
    pub online_threshold_seconds: u64,
    pub monthly_reset_day: u32,
    pub timezone: String,
    pub date_calendar: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MetricsDTO {
    pub routers_count: i64,
    pub peers_count: i64,
    pub users_count: i64,
    pub database_size_bytes: i64,
}

pub async fn get_settings(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let poll_interval_seconds = conn.query_row("SELECT value FROM settings_kv WHERE key = 'poll_interval_seconds'", [], |r| r.get::<_, String>(0))
        .ok().and_then(|v| v.parse().ok()).unwrap_or(state.settings.poll_interval_seconds);
    let online_threshold_seconds = conn.query_row("SELECT value FROM settings_kv WHERE key = 'online_threshold_seconds'", [], |r| r.get::<_, String>(0))
        .ok().and_then(|v| v.parse().ok()).unwrap_or(state.settings.online_threshold_seconds);
    let monthly_reset_day = conn.query_row("SELECT value FROM settings_kv WHERE key = 'monthly_reset_day'", [], |r| r.get::<_, String>(0))
        .ok().and_then(|v| v.parse().ok()).unwrap_or(state.settings.monthly_reset_day);
    let timezone = conn.query_row("SELECT value FROM settings_kv WHERE key = 'timezone'", [], |r| r.get::<_, String>(0))
        .unwrap_or_else(|_| state.settings.timezone.clone());
    let date_calendar = conn.query_row("SELECT value FROM settings_kv WHERE key = 'date_calendar'", [], |r| r.get::<_, String>(0))
        .unwrap_or_else(|_| state.settings.date_calendar.clone());

    (StatusCode::OK, Json(SettingsDTO {
        poll_interval_seconds,
        online_threshold_seconds,
        monthly_reset_day,
        timezone,
        date_calendar,
    })).into_response()
}

pub async fn update_settings(headers: HeaderMap, State(state): State<AppState>, Json(payload): Json<SettingsDTO>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let kvs = [
        ("poll_interval_seconds", payload.poll_interval_seconds.to_string()),
        ("online_threshold_seconds", payload.online_threshold_seconds.to_string()),
        ("monthly_reset_day", payload.monthly_reset_day.to_string()),
        ("timezone", payload.timezone.clone()),
        ("date_calendar", payload.date_calendar.clone()),
    ];

    for (k, v) in kvs {
        let _ = conn.execute(
            "INSERT INTO settings_kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![k, v],
        );
    }

    (StatusCode::OK, Json(payload)).into_response()
}

pub async fn get_metrics(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let conn = state.pool.get().unwrap();
    let routers_count: i64 = conn.query_row("SELECT COUNT(*) FROM routers", [], |r| r.get(0)).unwrap_or(0);
    let peers_count: i64 = conn.query_row("SELECT COUNT(*) FROM peers", [], |r| r.get(0)).unwrap_or(0);
    let users_count: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0)).unwrap_or(0);
    let database_size_bytes: i64 = conn.query_row("SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()", [], |r| r.get(0)).unwrap_or(0);

    (StatusCode::OK, Json(MetricsDTO {
        routers_count,
        peers_count,
        users_count,
        database_size_bytes,
    })).into_response()
}
