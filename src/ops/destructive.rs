use chrono::{DateTime, Utc};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct ExclusiveOperation {
    pub key: String,
    pub label: String,
    pub detail: String,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ExclusiveOperationGate {
    state: Arc<Mutex<Option<ExclusiveOperation>>>,
    activity: Arc<tokio::sync::Mutex<()>>,
}

impl Default for ExclusiveOperationGate {
    fn default() -> Self {
        Self::new()
    }
}

impl ExclusiveOperationGate {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(None)),
            activity: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub fn snapshot(&self) -> Option<ExclusiveOperation> {
        let lock = self.state.lock().unwrap();
        lock.clone()
    }

    pub fn is_active(&self) -> bool {
        self.snapshot().is_some()
    }

    pub async fn begin(&self, key: &str, label: &str, detail: &str) -> Result<ExclusiveOperationGuard, String> {
        let op = ExclusiveOperation {
            key: key.to_string(),
            label: label.to_string(),
            detail: detail.trim().to_string(),
            started_at: Utc::now(),
        };

        {
            let mut lock = self.state.lock().unwrap();
            if let Some(active) = lock.as_ref() {
                return Err(format!("{} is already in progress", active.label));
            }
            *lock = Some(op);
        }

        let activity_guard = self.activity.clone().lock_owned().await;

        Ok(ExclusiveOperationGuard {
            gate: self.clone(),
            _activity_guard: activity_guard,
        })
    }

    pub async fn coordinated_activity<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce() -> R,
    {
        let _guard = self.activity.lock().await;
        if self.is_active() {
            None
        } else {
            Some(f())
        }
    }
}

pub struct ExclusiveOperationGuard {
    gate: ExclusiveOperationGate,
    _activity_guard: tokio::sync::OwnedMutexGuard<()>,
}

impl Drop for ExclusiveOperationGuard {
    fn drop(&mut self) {
        let mut lock = self.gate.state.lock().unwrap();
        *lock = None;
    }
}
