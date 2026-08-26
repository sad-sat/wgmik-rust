use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupStatus {
    pub running: bool,
    pub phase: String,
    pub phase_label: String,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub detail: Option<String>,
    pub file_size: Option<i64>,
    pub download_token: Option<String>,
    pub download_filename: Option<String>,
    pub elapsed_seconds: i64,
    pub progress_percent: f64,
}

impl Default for BackupStatus {
    fn default() -> Self {
        Self {
            running: false,
            phase: "idle".to_string(),
            phase_label: "Idle".to_string(),
            started_at: None,
            finished_at: None,
            updated_at: None,
            last_error: None,
            detail: None,
            file_size: None,
            download_token: None,
            download_filename: None,
            elapsed_seconds: 0,
            progress_percent: 0.0,
        }
    }
}

pub type BackupManager = Arc<Mutex<BackupStatus>>;

pub fn new_backup_manager() -> BackupManager {
    Arc::new(Mutex::new(BackupStatus::default()))
}
