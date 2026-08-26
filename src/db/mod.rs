pub mod models;
pub mod schema;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use schema::run_migrations;
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
                PRAGMA cache_size = -2000;
                PRAGMA temp_store = MEMORY;
                "#,
            )?;
            Ok(())
        });

    let pool = Pool::builder()
        .max_size(4)
        .min_idle(Some(1))
        .idle_timeout(Some(std::time::Duration::from_secs(60)))
        .build(manager)
        .expect("Failed to create SQLite connection pool");

    {
        let conn = pool.get().expect("Failed to obtain DB connection for migration");
        run_migrations(&conn).expect("Database migration failed");
    }

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
