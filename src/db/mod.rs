pub mod models;
pub mod schema;

use r2d2::{ManageConnection, Pool};
use r2d2_sqlite::SqliteConnectionManager;
use schema::initialize_database;
use std::fs;
use std::path::PathBuf;

pub type DbPool = Pool<SqliteConnectionManager>;

pub fn create_pool(database_url: &str) -> DbPool {
    let path = sqlite_path_from_url(database_url).unwrap_or_else(|| PathBuf::from("./wgmik.db"));
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let manager = SqliteConnectionManager::file(&path)
        .with_init(|c| {
            c.execute_batch(
                r#"
                PRAGMA foreign_keys = ON;
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;
                PRAGMA busy_timeout = 30000;
                PRAGMA cache_size = -4000;
                PRAGMA temp_store = MEMORY;
                PRAGMA mmap_size = 134217728;
                "#,
            )?;
            Ok(())
        });

    // Initialize database schema first on a direct connection before pooling
    if let Ok(init_conn) = manager.connect() {
        let _ = initialize_database(&init_conn);
    }

    let pool = Pool::builder()
        .max_size(6)
        .min_idle(Some(1))
        .idle_timeout(Some(std::time::Duration::from_secs(60)))
        .build(manager)
        .expect("Failed to create SQLite connection pool");

    pool
}

fn sqlite_path_from_url(database_url: &str) -> Option<PathBuf> {
    if let Some(path_str) = database_url.strip_prefix("sqlite:////") {
        Some(PathBuf::from(format!("/{}", path_str)))
    } else if let Some(path_str) = database_url.strip_prefix("sqlite:///") {
        Some(PathBuf::from(path_str))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fresh_database_initialization() {
        let temp_dir = std::env::temp_dir().join(format!("wgmik_test_{}", rand::random::<u64>()));
        let db_path = temp_dir.join("test.db");
        let db_url = format!("sqlite:///{}", db_path.display());

        let pool = create_pool(&db_url);
        let conn = pool.get().expect("get conn");

        // Verify users table exists and count query works
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0)).expect("query users count");
        assert_eq!(count, 0);

        // Verify insert into users table works
        let res = conn.execute(
            "INSERT INTO users (username, hashed_password, is_admin, is_active, session_version, must_change_password)
             VALUES (?1, ?2, 1, 1, 1, 0)",
            rusqlite::params!["admin", "some_hash"],
        );
        assert!(res.is_ok());

        let count_after: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0)).expect("query users count after");
        assert_eq!(count_after, 1);

        // Cleanup
        drop(conn);
        drop(pool);
        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
