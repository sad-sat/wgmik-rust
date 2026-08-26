use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageMaintenanceStatus {
    pub running: bool,
    pub phase: String,
    pub phase_label: String,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub detail: Option<String>,
    pub file_size_before: Option<i64>,
    pub file_size_after: Option<i64>,
    pub backfilled_minutes: i64,
    pub deleted_samples: i64,
    pub deleted_minutes: i64,
    pub deleted_daily: i64,
    pub progress_percent: f64,
    pub phase_progress_percent: f64,
    pub elapsed_seconds: i64,
    pub trigger: String,
}

impl Default for UsageMaintenanceStatus {
    fn default() -> Self {
        Self {
            running: false,
            phase: "idle".to_string(),
            phase_label: "Idle".to_string(),
            started_at: None,
            finished_at: None,
            updated_at: None,
            cancelled_at: None,
            last_error: None,
            detail: None,
            file_size_before: None,
            file_size_after: None,
            backfilled_minutes: 0,
            deleted_samples: 0,
            deleted_minutes: 0,
            deleted_daily: 0,
            progress_percent: 0.0,
            phase_progress_percent: 0.0,
            elapsed_seconds: 0,
            trigger: "manual".to_string(),
        }
    }
}

pub type MaintenanceManager = Arc<Mutex<UsageMaintenanceStatus>>;

pub fn new_maintenance_manager() -> MaintenanceManager {
    Arc::new(Mutex::new(UsageMaintenanceStatus::default()))
}
