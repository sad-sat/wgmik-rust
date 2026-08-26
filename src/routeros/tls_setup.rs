use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsSetupStatus {
    pub running: bool,
    pub step: String,
    pub progress_percent: u32,
    pub error: Option<String>,
    pub finished_at: Option<DateTime<Utc>>,
    pub success: bool,
}

impl Default for TlsSetupStatus {
    fn default() -> Self {
        Self {
            running: false,
            step: "idle".to_string(),
            progress_percent: 0,
            error: None,
            finished_at: None,
            success: false,
        }
    }
}

pub type TlsSetupManager = Arc<Mutex<std::collections::HashMap<i64, TlsSetupStatus>>>;

pub fn new_tls_setup_manager() -> TlsSetupManager {
    Arc::new(Mutex::new(std::collections::HashMap::new()))
}
