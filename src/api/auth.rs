use crate::config::AppSettings;
use crate::crypto::{create_access_token, get_password_hash, verify_password, verify_token};
use crate::db::models::User;
use crate::db::DbPool;
use axum::extract::{Json, State};
use axum::http::header::{HeaderMap, SET_COOKIE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::IntoResponse;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub settings: AppSettings,
    pub gate: crate::ops::ExclusiveOperationGate,
    pub maintenance: crate::accounting::MaintenanceManager,
    pub backup: crate::backup::BackupManager,
    pub tls_setup: crate::routeros::tls_setup::TlsSetupManager,
    pub bot: Arc<tokio::sync::Mutex<Option<Arc<crate::telegram::TelegramBot>>>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginReq {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginResp {
    pub access_token: String,
    pub token_type: String,
    pub ok: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SetupReq {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SetupStateDTO {
    pub needs_initial_setup: bool,
    pub setup_required: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UserDTO {
    pub id: i64,
    pub username: String,
    pub is_admin: bool,
    pub is_active: bool,
    pub must_change_password: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthBootstrapDTO {
    pub user: Option<UserDTO>,
    pub router_count: i64,
    pub enabled_router_count: i64,
    pub peer_count: i64,
    pub selected_peer_count: i64,
    pub needs_onboarding: bool,
    pub needs_peer_import: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChangePasswordReq {
    pub current_password: String,
    pub new_password: String,
}

pub fn extract_token_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(auth_val) = headers.get("authorization") {
        if let Ok(s) = auth_val.to_str() {
            if let Some(tok) = s.strip_prefix("Bearer ").or_else(|| s.strip_prefix("bearer ")) {
                let trimmed = tok.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }

    if let Some(cookie_val) = headers.get("cookie") {
        if let Ok(s) = cookie_val.to_str() {
            for cookie in s.split(';') {
                let mut parts = cookie.trim().splitn(2, '=');
                if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                    if k.trim() == "access_token" {
                        let trimmed = v.trim();
                        if !trimmed.is_empty() {
                            return Some(trimmed.to_string());
                        }
                    }
                }
            }
        }
    }

    None
}

pub async fn get_current_user(headers: &HeaderMap, state: &AppState) -> Option<User> {
    let token = extract_token_from_headers(headers)?;

    let (user_id, session_version) = verify_token(&token, &state.settings.secret_key)?;
    let conn = state.pool.get().ok()?;
    conn.query_row(
        "SELECT id, username, hashed_password, is_admin, is_active, session_version, must_change_password FROM users WHERE id = ?1 AND is_active = 1",
        params![user_id],
        |row| {
            let sv: i64 = row.get(5)?;
            if sv != session_version {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Ok(User {
                id: row.get(0)?,
                username: row.get(1)?,
                hashed_password: row.get(2)?,
                is_admin: row.get(3)?,
                is_active: row.get(4)?,
                session_version: sv,
                password_changed_at: None,
                last_login_at: None,
                failed_login_attempts: 0,
                locked_until: None,
                must_change_password: row.get(6)?,
                created_at: chrono::Utc::now(),
            })
        },
    ).ok()
}

pub async fn auth_login(State(state): State<AppState>, Json(payload): Json<LoginReq>) -> impl IntoResponse {
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let user_res = conn.query_row(
        "SELECT id, username, hashed_password, is_admin, is_active, session_version, must_change_password FROM users WHERE username = ?1",
        params![payload.username],
        |row| {
            Ok(User {
                id: row.get(0)?,
                username: row.get(1)?,
                hashed_password: row.get(2)?,
                is_admin: row.get(3)?,
                is_active: row.get(4)?,
                session_version: row.get(5)?,
                password_changed_at: None,
                last_login_at: None,
                failed_login_attempts: 0,
                locked_until: None,
                must_change_password: row.get(6)?,
                created_at: chrono::Utc::now(),
            })
        },
    );

    let user = match user_res {
        Ok(u) => u,
        Err(_) => return (StatusCode::UNAUTHORIZED, "Invalid username or password").into_response(),
    };

    if !user.is_active {
        return (StatusCode::FORBIDDEN, "Account disabled").into_response();
    }

    if !verify_password(&payload.password, &user.hashed_password) {
        return (StatusCode::UNAUTHORIZED, "Invalid username or password").into_response();
    }

    let token = match create_access_token(user.id, user.session_version, &state.settings.secret_key, Some(7)) {
        Ok(t) => t,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Token generation failed").into_response(),
    };

    let cookie = format!("access_token={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800", token);
    let mut resp = (StatusCode::OK, Json(LoginResp { access_token: token, token_type: "bearer".to_string(), ok: true })).into_response();
    resp.headers_mut().insert(SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    resp
}

pub async fn auth_logout() -> impl IntoResponse {
    let cookie = "access_token=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0";
    let mut resp = (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response();
    resp.headers_mut().insert(SET_COOKIE, HeaderValue::from_static(cookie));
    resp
}

pub async fn auth_setup_state(State(state): State<AppState>) -> impl IntoResponse {
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0)).unwrap_or(0);
    let needs_setup = count == 0;
    (StatusCode::OK, Json(SetupStateDTO {
        needs_initial_setup: needs_setup,
        setup_required: needs_setup,
    })).into_response()
}

pub async fn auth_setup(State(state): State<AppState>, Json(payload): Json<SetupReq>) -> impl IntoResponse {
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0)).unwrap_or(0);
    if count > 0 {
        return (StatusCode::CONFLICT, "Setup already completed").into_response();
    }

    if payload.password.len() < 12 {
        return (StatusCode::BAD_REQUEST, "Password must be at least 12 characters").into_response();
    }

    let hashed = match get_password_hash(&payload.password) {
        Ok(h) => h,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Hashing failed").into_response(),
    };

    let res = conn.execute(
        "INSERT INTO users (username, hashed_password, is_admin, is_active, session_version, must_change_password)
         VALUES (?1, ?2, 1, 1, 1, 0)",
        params![payload.username, hashed],
    );

    if res.is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create admin").into_response();
    }

    let user_id = conn.last_insert_rowid();
    let token = match create_access_token(user_id, 1, &state.settings.secret_key, Some(7)) {
        Ok(t) => t,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Token generation failed").into_response(),
    };

    let cookie = format!("access_token={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800", token);
    let mut resp = (StatusCode::OK, Json(LoginResp { access_token: token, token_type: "bearer".to_string(), ok: true })).into_response();
    resp.headers_mut().insert(SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    resp
}

pub async fn auth_me(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    let user = match get_current_user(&headers, &state).await {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };

    (StatusCode::OK, Json(UserDTO {
        id: user.id,
        username: user.username,
        is_admin: user.is_admin,
        is_active: user.is_active,
        must_change_password: user.must_change_password,
    })).into_response()
}

pub async fn auth_bootstrap(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response(),
    };

    let user = get_current_user(&headers, &state).await.map(|u| UserDTO {
        id: u.id,
        username: u.username,
        is_admin: u.is_admin,
        is_active: u.is_active,
        must_change_password: u.must_change_password,
    });

    let router_count: i64 = conn.query_row("SELECT COUNT(*) FROM routers", [], |r| r.get(0)).unwrap_or(0);
    let enabled_router_count: i64 = conn.query_row("SELECT COUNT(*) FROM routers WHERE enabled = 1", [], |r| r.get(0)).unwrap_or(0);
    let peer_count: i64 = conn.query_row("SELECT COUNT(*) FROM peers", [], |r| r.get(0)).unwrap_or(0);
    let selected_peer_count: i64 = conn.query_row("SELECT COUNT(*) FROM peers WHERE selected = 1", [], |r| r.get(0)).unwrap_or(0);

    (StatusCode::OK, Json(AuthBootstrapDTO {
        user,
        router_count,
        enabled_router_count,
        peer_count,
        selected_peer_count,
        needs_onboarding: router_count == 0,
        needs_peer_import: router_count > 0 && peer_count == 0,
    })).into_response()
}

pub async fn auth_change_password(headers: HeaderMap, State(state): State<AppState>, Json(payload): Json<ChangePasswordReq>) -> impl IntoResponse {
    let user = match get_current_user(&headers, &state).await {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };

    if !verify_password(&payload.current_password, &user.hashed_password) {
        return (StatusCode::BAD_REQUEST, "Current password incorrect").into_response();
    }

    if payload.new_password.len() < 12 {
        return (StatusCode::BAD_REQUEST, "New password must be at least 12 characters").into_response();
    }

    let hashed = match get_password_hash(&payload.new_password) {
        Ok(h) => h,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Hashing failed").into_response(),
    };

    let conn = state.pool.get().unwrap();
    let new_sv = user.session_version + 1;
    let _ = conn.execute(
        "UPDATE users SET hashed_password = ?1, session_version = ?2, must_change_password = 0 WHERE id = ?3",
        params![hashed, new_sv, user.id],
    );

    (StatusCode::OK, Json(serde_json::json!({"status": "password_changed"}))).into_response()
}
