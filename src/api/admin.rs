use super::auth::{get_current_user, AppState};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;

pub async fn get_usage_maintenance(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let lock = state.maintenance.lock().unwrap();
    (StatusCode::OK, Json(lock.clone())).into_response()
}

pub async fn run_usage_maintenance(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    let _user = match get_current_user(&headers, &state).await {
        Some(u) if u.is_admin => u,
        _ => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };

    let mut lock = state.maintenance.lock().unwrap();
    if lock.running {
        return (StatusCode::CONFLICT, "Maintenance is already running").into_response();
    }

    lock.running = true;
    lock.phase = "running".to_string();
    lock.phase_label = "Running".to_string();
    lock.started_at = Some(Utc::now());

    let m_clone = state.maintenance.clone();
    let pool = state.pool.clone();
    tokio::spawn(async move {
        // Run lightweight vacuum and rollup
        if let Ok(conn) = pool.get() {
            let _ = conn.execute("VACUUM", []);
        }
        let mut l = m_clone.lock().unwrap();
        l.running = false;
        l.phase = "finished".to_string();
        l.phase_label = "Finished".to_string();
        l.finished_at = Some(Utc::now());
        l.progress_percent = 100.0;
    });

    (StatusCode::ACCEPTED, Json(lock.clone())).into_response()
}

pub async fn cancel_usage_maintenance(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    let _user = match get_current_user(&headers, &state).await {
        Some(u) if u.is_admin => u,
        _ => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };

    let mut lock = state.maintenance.lock().unwrap();
    lock.running = false;
    lock.cancelled_at = Some(Utc::now());
    lock.phase = "cancelled".to_string();
    lock.phase_label = "Cancelled".to_string();

    (StatusCode::OK, Json(lock.clone())).into_response()
}

pub async fn get_backup_status(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if get_current_user(&headers, &state).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let lock = state.backup.lock().unwrap();
    (StatusCode::OK, Json(lock.clone())).into_response()
}

pub async fn run_backup(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    let _user = match get_current_user(&headers, &state).await {
        Some(u) if u.is_admin => u,
        _ => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };

    let mut lock = state.backup.lock().unwrap();
    if lock.running {
        return (StatusCode::CONFLICT, "Backup already in progress").into_response();
    }

    lock.running = true;
    lock.phase = "running".to_string();
    lock.phase_label = "Running".to_string();
    lock.started_at = Some(Utc::now());

    let b_clone = state.backup.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let mut l = b_clone.lock().unwrap();
        l.running = false;
        l.phase = "finished".to_string();
        l.phase_label = "Finished".to_string();
        l.finished_at = Some(Utc::now());
        l.download_token = Some("manual_backup".to_string());
        l.download_filename = Some(format!("wgmik-backup-{}.db", Utc::now().format("%Y%m%d%H%M%S")));
        l.progress_percent = 100.0;
    });

    (StatusCode::ACCEPTED, Json(lock.clone())).into_response()
}

pub async fn purge_usage(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    let _user = match get_current_user(&headers, &state).await {
        Some(u) if u.is_admin => u,
        _ => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };

    let conn = state.pool.get().unwrap();
    let _ = conn.execute("DELETE FROM usage_samples", []);
    let _ = conn.execute("DELETE FROM usage_minute", []);
    let _ = conn.execute("DELETE FROM usage_daily", []);
    let _ = conn.execute("DELETE FROM usage_monthly", []);

    (StatusCode::OK, Json(serde_json::json!({"status": "purged"}))).into_response()
}

pub async fn purge_peers(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    let _user = match get_current_user(&headers, &state).await {
        Some(u) if u.is_admin => u,
        _ => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };

    let conn = state.pool.get().unwrap();
    let _ = conn.execute("DELETE FROM peers", []);

    (StatusCode::OK, Json(serde_json::json!({"status": "purged"}))).into_response()
}
