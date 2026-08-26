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
    pub cpu_percent: Option<f64>,
    pub load_1: Option<f64>,
    pub load_5: Option<f64>,
    pub load_15: Option<f64>,
    pub mem_percent: Option<f64>,
    pub mem_used: Option<i64>,
    pub mem_total: Option<i64>,
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

    let mut load_1 = None;
    let mut load_5 = None;
    let mut load_15 = None;
    if let Ok(loadavg_str) = std::fs::read_to_string("/proc/loadavg") {
        let parts: Vec<&str> = loadavg_str.split_whitespace().collect();
        if parts.len() >= 3 {
            load_1 = parts[0].parse::<f64>().ok();
            load_5 = parts[1].parse::<f64>().ok();
            load_15 = parts[2].parse::<f64>().ok();
        }
    }

    let mut mem_total = None;
    let mut mem_used = None;
    let mut mem_percent = None;
    if let Ok(meminfo_str) = std::fs::read_to_string("/proc/meminfo") {
        let mut total_kb: Option<i64> = None;
        let mut avail_kb: Option<i64> = None;
        let mut free_kb: Option<i64> = None;

        for line in meminfo_str.lines() {
            if line.starts_with("MemTotal:") {
                total_kb = line.split_whitespace().nth(1).and_then(|v| v.parse().ok());
            } else if line.starts_with("MemAvailable:") {
                avail_kb = line.split_whitespace().nth(1).and_then(|v| v.parse().ok());
            } else if line.starts_with("MemFree:") {
                free_kb = line.split_whitespace().nth(1).and_then(|v| v.parse().ok());
            }
        }

        if let Some(tot) = total_kb {
            let tot_bytes = tot.saturating_mul(1024);
            mem_total = Some(tot_bytes);
            let free = avail_kb.or(free_kb).unwrap_or(0);
            let used_bytes = tot.saturating_sub(free).saturating_mul(1024);
            mem_used = Some(used_bytes);
            if tot > 0 {
                mem_percent = Some(((tot.saturating_sub(free)) as f64 / tot as f64) * 100.0);
            }
        }
    }

    let cpu_percent = load_1.map(|l| (l * 10.0).min(100.0));

    (StatusCode::OK, Json(MetricsDTO {
        cpu_percent,
        load_1,
        load_5,
        load_15,
        mem_percent,
        mem_used,
        mem_total,
    })).into_response()
}
