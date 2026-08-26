use super::auth::{get_current_user, AppState, UserDTO};
use crate::crypto::get_password_hash;
use axum::extract::{Json, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateUserReq {
    pub username: String,
    pub password: String,
    pub is_admin: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateUserReq {
    pub is_active: Option<bool>,
    pub is_admin: Option<bool>,
    pub must_change_password: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResetPasswordReq {
    pub new_password: String,
}

pub async fn list_users(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    let _user = match get_current_user(&headers, &state).await {
        Some(u) if u.is_admin => u,
        _ => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };

    let conn = state.pool.get().unwrap();
    let mut stmt = conn.prepare("SELECT id, username, is_admin, is_active, must_change_password FROM users").unwrap();
    let rows = stmt.query_map([], |row| {
        Ok(UserDTO {
            id: row.get(0)?,
            username: row.get(1)?,
            is_admin: row.get(2)?,
            is_active: row.get(3)?,
            must_change_password: row.get(4)?,
        })
    }).unwrap();

    let mut users = Vec::new();
    for r in rows {
        if let Ok(u) = r {
            users.push(u);
        }
    }

    (StatusCode::OK, Json(users)).into_response()
}

pub async fn create_user(headers: HeaderMap, State(state): State<AppState>, Json(payload): Json<CreateUserReq>) -> impl IntoResponse {
    let _user = match get_current_user(&headers, &state).await {
        Some(u) if u.is_admin => u,
        _ => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };

    if payload.password.len() < 8 {
        return (StatusCode::BAD_REQUEST, "Password must be at least 8 characters").into_response();
    }

    let hashed = match get_password_hash(&payload.password) {
        Ok(h) => h,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Hashing failed").into_response(),
    };

    let conn = state.pool.get().unwrap();
    let is_admin = payload.is_admin.unwrap_or(false);
    let res = conn.execute(
        "INSERT INTO users (username, hashed_password, is_admin, is_active, session_version, must_change_password)
         VALUES (?1, ?2, ?3, 1, 1, 0)",
        params![payload.username, hashed, is_admin],
    );

    if res.is_err() {
        return (StatusCode::BAD_REQUEST, "Username already exists").into_response();
    }

    let id = conn.last_insert_rowid();
    (StatusCode::OK, Json(UserDTO {
        id,
        username: payload.username,
        is_admin,
        is_active: true,
        must_change_password: false,
    })).into_response()
}

pub async fn update_user(headers: HeaderMap, State(state): State<AppState>, Path(user_id): Path<i64>, Json(payload): Json<UpdateUserReq>) -> impl IntoResponse {
    let _current_user = match get_current_user(&headers, &state).await {
        Some(u) if u.is_admin => u,
        _ => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };

    let conn = state.pool.get().unwrap();
    if let Some(active) = payload.is_active {
        let _ = conn.execute("UPDATE users SET is_active = ?1 WHERE id = ?2", params![active, user_id]);
    }
    if let Some(admin) = payload.is_admin {
        let _ = conn.execute("UPDATE users SET is_admin = ?1 WHERE id = ?2", params![admin, user_id]);
    }
    if let Some(mcp) = payload.must_change_password {
        let _ = conn.execute("UPDATE users SET must_change_password = ?1 WHERE id = ?2", params![mcp, user_id]);
    }

    let user_row = conn.query_row(
        "SELECT id, username, is_admin, is_active, must_change_password FROM users WHERE id = ?1",
        params![user_id],
        |row| Ok(UserDTO {
            id: row.get(0)?,
            username: row.get(1)?,
            is_admin: row.get(2)?,
            is_active: row.get(3)?,
            must_change_password: row.get(4)?,
        }),
    );

    match user_row {
        Ok(u) => (StatusCode::OK, Json(u)).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "User not found").into_response(),
    }
}

pub async fn reset_user_password(headers: HeaderMap, State(state): State<AppState>, Path(user_id): Path<i64>, Json(payload): Json<ResetPasswordReq>) -> impl IntoResponse {
    let _current_user = match get_current_user(&headers, &state).await {
        Some(u) if u.is_admin => u,
        _ => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };

    let hashed = match get_password_hash(&payload.new_password) {
        Ok(h) => h,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Hashing failed").into_response(),
    };

    let conn = state.pool.get().unwrap();
    let _ = conn.execute(
        "UPDATE users SET hashed_password = ?1, session_version = session_version + 1, must_change_password = 1 WHERE id = ?2",
        params![hashed, user_id],
    );

    (StatusCode::OK, Json(serde_json::json!({"status": "password_reset"}))).into_response()
}

pub async fn delete_user(headers: HeaderMap, State(state): State<AppState>, Path(user_id): Path<i64>) -> impl IntoResponse {
    let current_user = match get_current_user(&headers, &state).await {
        Some(u) if u.is_admin => u,
        _ => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };

    if current_user.id == user_id {
        return (StatusCode::BAD_REQUEST, "Cannot delete yourself").into_response();
    }

    let conn = state.pool.get().unwrap();
    let _ = conn.execute("DELETE FROM users WHERE id = ?1", params![user_id]);
    (StatusCode::OK, Json(serde_json::json!({"status": "deleted"}))).into_response()
}
