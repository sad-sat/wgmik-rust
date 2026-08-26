pub mod models;
pub mod schema;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::path::PathBuf;
use std::time::Duration;

pub type DbPool = Pool<SqliteConnectionManager>;

#[derive(Debug)]
pub struct CustomCustomizer;

impl r2d2::CustomizeConnection<Connection, rusqlite::Error> for CustomCustomizer {
    fn on_acquire(&self, conn: &mut Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch(r#"
            PRAGMA foreign_keys = ON;
            PRAGMA busy_timeout = 30000;
            PRAGMA synchronous = NORMAL;
        "#)?;
        Ok(())
    }
}

pub fn get_sqlite_path(database_url: &str) -> PathBuf {
    if let Some(path_str) = database_url.strip_prefix("sqlite:////") {
        PathBuf::from(format!("/{}", path_str))
    } else if let Some(path_str) = database_url.strip_prefix("sqlite:///") {
        PathBuf::from(path_str)
    } else if let Some(path_str) = database_url.strip_prefix("sqlite:") {
        PathBuf::from(path_str)
    } else {
        PathBuf::from("./wgmik.db")
    }
}

pub fn create_pool(database_url: &str) -> DbPool {
    let db_path = get_sqlite_path(database_url);
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let manager = SqliteConnectionManager::file(&db_path);
    let pool = Pool::builder()
        .max_size(16)
        .min_idle(Some(2))
        .connection_timeout(Duration::from_secs(30))
        .connection_customizer(Box::new(CustomCustomizer))
        .build(manager)
        .expect("Failed to initialize SQLite connection pool");

    // Initialize schema and migrations on startup
    let conn = pool.get().expect("Failed to acquire connection for schema init");
    schema::initialize_database(&conn).expect("Failed to initialize database schema");

    pool
}
