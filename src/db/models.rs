use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Router {
    pub id: i64,
    pub name: String,
    pub host: String,
    pub proto: String, // "rest" | "rest-http" | "api" | "api-plain"
    pub port: u16,
    pub username: String,
    pub secret_enc: String,
    pub tls_verify: bool,
    pub enabled: bool,
    pub ros_version: String,
    pub ros_version_checked_at: Option<DateTime<Utc>>,
    pub ros_supported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    pub id: i64,
    pub router_id: i64,
    pub interface: String,
    pub ros_id: String,
    pub name: String,
    pub public_key: String,
    pub allowed_address: String,
    pub comment: String,
    pub disabled: bool,
    pub selected: bool,
    pub router_sync_status: String, // "synced" | "new" | "missing"
    pub router_sync_first_seen_at: Option<DateTime<Utc>>,
    pub router_sync_last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSample {
    pub id: i64,
    pub peer_id: i64,
    pub ts: DateTime<Utc>,
    pub rx: i64,
    pub tx: i64,
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageDaily {
    pub id: i64,
    pub peer_id: i64,
    pub day: String, // "YYYY-MM-DD"
    pub rx: i64,
    pub tx: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageMinute {
    pub id: i64,
    pub peer_id: i64,
    pub minute_ts: DateTime<Utc>,
    pub rx: i64,
    pub tx: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageMonthly {
    pub id: i64,
    pub peer_id: i64,
    pub month_key: String, // "YYYY-MM"
    pub rx: i64,
    pub tx: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerTotalsMerge {
    pub id: i64,
    pub source_peer_id: i64,
    pub target_peer_id: i64,
    pub source_router_id: i64,
    pub target_router_id: i64,
    pub merge_mode: String,
    pub match_type: String,
    pub usage_minute_rows: i64,
    pub usage_daily_rows: i64,
    pub usage_monthly_rows: i64,
    pub applied_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quota {
    pub id: i64,
    pub peer_id: i64,
    pub monthly_limit_bytes: i64,
    pub reset_day: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FairUsageRule {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub quota_mode: String, // "combined" | "independent"
    pub download_quota_bytes: i64,
    pub upload_quota_bytes: Option<i64>,
    pub throttle_download_kbps: i64,
    pub throttle_upload_kbps: i64,
    pub time_scope: String,
    pub scope_period_count: i64,
    pub scope_period_unit: String, // "hour" | "day" | "week" | "month"
    pub scope_type: String,        // "global" | "router" | "peer"
    pub router_id: Option<i64>,
    pub sort_order: i64,
    pub passthrough: bool,
    pub enabled: bool,
    pub tiered: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FairUsageTier {
    pub id: i64,
    pub rule_id: i64,
    pub sort_order: i64,
    pub threshold_bytes: i64,
    pub name: String,
    pub throttle_download_kbps: i64,
    pub throttle_upload_kbps: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FairUsageAssignment {
    pub id: i64,
    pub rule_id: i64,
    pub peer_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FairUsageState {
    pub id: i64,
    pub peer_id: i64,
    pub rule_id: i64,
    pub tier_id: Option<i64>,
    pub throttled: bool,
    pub ros_queue_id: String,
    pub throttled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub id: i64,
    pub peer_id: Option<i64>,
    pub ts: DateTime<Utc>,
    pub action: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsKV {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub hashed_password: String,
    pub is_admin: bool,
    pub is_active: bool,
    pub session_version: i64,
    pub password_changed_at: Option<DateTime<Utc>>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub failed_login_attempts: i64,
    pub locked_until: Option<DateTime<Utc>>,
    pub must_change_password: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSecurityEvent {
    pub id: i64,
    pub actor_user_id: Option<i64>,
    pub target_user_id: Option<i64>,
    pub event_type: String,
    pub detail: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramUser {
    pub id: i64,
    pub telegram_user_id: i64,
    pub telegram_username: String,
    pub first_name: String,
    pub last_name: String,
    pub language: String,
    pub is_blocked: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramPeerBinding {
    pub id: i64,
    pub telegram_user_id: i64,
    pub peer_id: i64,
    pub visible: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramSignupToken {
    pub id: i64,
    pub token: String,
    pub peer_ids: String, // JSON array "[1,2]"
    pub created_by: Option<i64>,
    pub used_by: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub single_use: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramNotificationConfig {
    pub id: i64,
    pub event_type: String,
    pub notify_clients: bool,
    pub notify_admin: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramUserNotificationPreference {
    pub id: i64,
    pub telegram_user_id: i64,
    pub event_type: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramNotificationLog {
    pub id: i64,
    pub telegram_user_id: i64,
    pub peer_id: Option<i64>,
    pub event_type: String,
    pub sent_at: DateTime<Utc>,
    pub message_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramBroadcast {
    pub id: i64,
    pub created_by_user_id: Option<i64>,
    pub body: String,
    pub photo_path: Option<String>,
    pub photo_filename: String,
    pub photo_mime: String,
    pub photo_size_bytes: i64,
    pub recipient_mode: String,
    pub status: String,
    pub total_count: i64,
    pub sent_count: i64,
    pub failed_count: i64,
    pub acknowledged_count: i64,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramBroadcastRecipient {
    pub id: i64,
    pub broadcast_id: i64,
    pub telegram_user_id: Option<i64>,
    pub chat_id: i64,
    pub display_name: String,
    pub status: String,
    pub telegram_message_id: Option<i64>,
    pub error_code: String,
    pub error_message: String,
    pub sent_at: Option<DateTime<Utc>>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}
