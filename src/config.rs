use std::env;
use std::fs;
use std::path::PathBuf;
use rand::distributions::Alphanumeric;
use rand::Rng;

#[derive(Clone, Debug)]
pub struct AppSettings {
    pub app_name: String,
    pub secret_key: String,
    pub database_url: String,
    pub debug: bool,
    pub poll_interval_seconds: u64,
    pub online_threshold_seconds: u64,
    pub monthly_reset_day: u32,
    pub timezone: String,
    pub date_calendar: String,
    pub host: String,
    pub port: u16,
}

impl AppSettings {
    pub fn from_env() -> Self {
        let app_name = env::var("APP_NAME").unwrap_or_else(|_| "wgmik-server".to_string());
        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:///./wgmik.db".to_string());
        let debug = env::var("DEBUG").map(|v| v == "1" || v.to_lowercase() == "true").unwrap_or(false);
        let poll_interval_seconds = env::var("POLL_INTERVAL_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(30);
        let online_threshold_seconds = env::var("ONLINE_THRESHOLD_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(15);
        let monthly_reset_day = env::var("MONTHLY_RESET_DAY").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
        let timezone = env::var("TIMEZONE").unwrap_or_else(|_| "UTC".to_string());
        let date_calendar = env::var("DATE_CALENDAR").unwrap_or_else(|_| "gregorian".to_string());
        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(6574);

        let secret_key = resolve_secret_key(&database_url);

        Self {
            app_name,
            secret_key,
            database_url,
            debug,
            poll_interval_seconds,
            online_threshold_seconds,
            monthly_reset_day,
            timezone,
            date_calendar,
            host,
            port,
        }
    }
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

fn default_secret_key_file(database_url: &str) -> PathBuf {
    if let Ok(explicit) = env::var("SECRET_KEY_FILE") {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Some(db_path) = sqlite_path_from_url(database_url) {
        if let Some(parent) = db_path.parent() {
            return parent.join("secret_key");
        }
    }
    PathBuf::from("./secret_key")
}

fn load_or_create_secret_key(database_url: &str) -> String {
    let key_file = default_secret_key_file(database_url);
    if key_file.exists() {
        if let Ok(content) = fs::read_to_string(&key_file) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }

    let new_key: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect();

    if let Some(parent) = key_file.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&key_file, &new_key);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&key_file, fs::Permissions::from_mode(0o600));
    }
    new_key
}

fn resolve_secret_key(database_url: &str) -> String {
    if let Ok(val) = env::var("SECRET_KEY") {
        let trimmed = val.trim();
        if !trimmed.is_empty() && trimmed != "change-me" {
            return trimmed.to_string();
        }
    }
    load_or_create_secret_key(database_url)
}
