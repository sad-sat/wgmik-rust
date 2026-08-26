use rusqlite::{Connection, Result};

pub const DDL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS routers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name VARCHAR(255) NOT NULL,
    host VARCHAR(255) NOT NULL,
    proto VARCHAR(10) NOT NULL DEFAULT 'rest',
    port INTEGER NOT NULL DEFAULT 443,
    username VARCHAR(255) NOT NULL,
    secret_enc TEXT NOT NULL,
    tls_verify BOOLEAN NOT NULL DEFAULT 1,
    enabled BOOLEAN NOT NULL DEFAULT 1,
    ros_version VARCHAR(64) NOT NULL DEFAULT '',
    ros_version_checked_at DATETIME,
    ros_supported BOOLEAN NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS peers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    router_id INTEGER NOT NULL,
    interface VARCHAR(128) NOT NULL,
    ros_id VARCHAR(64) NOT NULL DEFAULT '',
    name VARCHAR(255) NOT NULL DEFAULT '',
    public_key VARCHAR(255) NOT NULL,
    allowed_address VARCHAR(255) NOT NULL,
    comment VARCHAR(255) NOT NULL DEFAULT '',
    disabled BOOLEAN NOT NULL DEFAULT 0,
    selected BOOLEAN NOT NULL DEFAULT 1,
    router_sync_status VARCHAR(16) NOT NULL DEFAULT 'synced',
    router_sync_first_seen_at DATETIME,
    router_sync_last_seen_at DATETIME,
    FOREIGN KEY(router_id) REFERENCES routers(id) ON DELETE CASCADE,
    UNIQUE(router_id, interface, public_key)
);

CREATE TABLE IF NOT EXISTS usage_samples (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    peer_id INTEGER NOT NULL,
    ts DATETIME NOT NULL,
    rx BIGINT NOT NULL,
    tx BIGINT NOT NULL,
    endpoint VARCHAR(255) NOT NULL DEFAULT '',
    FOREIGN KEY(peer_id) REFERENCES peers(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS usage_daily (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    peer_id INTEGER NOT NULL,
    day VARCHAR(10) NOT NULL,
    rx BIGINT NOT NULL DEFAULT 0,
    tx BIGINT NOT NULL DEFAULT 0,
    FOREIGN KEY(peer_id) REFERENCES peers(id) ON DELETE CASCADE,
    UNIQUE(peer_id, day)
);

CREATE TABLE IF NOT EXISTS usage_minute (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    peer_id INTEGER NOT NULL,
    minute_ts DATETIME NOT NULL,
    rx BIGINT NOT NULL DEFAULT 0,
    tx BIGINT NOT NULL DEFAULT 0,
    FOREIGN KEY(peer_id) REFERENCES peers(id) ON DELETE CASCADE,
    UNIQUE(peer_id, minute_ts)
);

CREATE TABLE IF NOT EXISTS usage_monthly (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    peer_id INTEGER NOT NULL,
    month_key VARCHAR(7) NOT NULL,
    rx BIGINT NOT NULL DEFAULT 0,
    tx BIGINT NOT NULL DEFAULT 0,
    FOREIGN KEY(peer_id) REFERENCES peers(id) ON DELETE CASCADE,
    UNIQUE(peer_id, month_key)
);

CREATE TABLE IF NOT EXISTS peer_totals_merge (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_peer_id INTEGER NOT NULL UNIQUE,
    target_peer_id INTEGER NOT NULL,
    source_router_id INTEGER NOT NULL,
    target_router_id INTEGER NOT NULL,
    merge_mode VARCHAR(32) NOT NULL DEFAULT 'totals_only',
    match_type VARCHAR(64) NOT NULL DEFAULT '',
    usage_minute_rows INTEGER NOT NULL DEFAULT 0,
    usage_daily_rows INTEGER NOT NULL DEFAULT 0,
    usage_monthly_rows INTEGER NOT NULL DEFAULT 0,
    applied_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(source_peer_id) REFERENCES peers(id) ON DELETE CASCADE,
    FOREIGN KEY(target_peer_id) REFERENCES peers(id) ON DELETE CASCADE,
    FOREIGN KEY(source_router_id) REFERENCES routers(id) ON DELETE CASCADE,
    FOREIGN KEY(target_router_id) REFERENCES routers(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS quotas (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    peer_id INTEGER NOT NULL UNIQUE,
    monthly_limit_bytes BIGINT NOT NULL DEFAULT 0,
    reset_day INTEGER NOT NULL DEFAULT 1,
    FOREIGN KEY(peer_id) REFERENCES peers(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS fair_usage_rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name VARCHAR(255) NOT NULL,
    description VARCHAR(512) NOT NULL DEFAULT '',
    quota_mode VARCHAR(16) NOT NULL DEFAULT 'combined',
    download_quota_bytes BIGINT NOT NULL DEFAULT 0,
    upload_quota_bytes BIGINT,
    throttle_download_kbps INTEGER NOT NULL DEFAULT 1000,
    throttle_upload_kbps INTEGER NOT NULL DEFAULT 1000,
    time_scope VARCHAR(16) NOT NULL DEFAULT 'monthly',
    scope_period_count INTEGER NOT NULL DEFAULT 1,
    scope_period_unit VARCHAR(8) NOT NULL DEFAULT 'month',
    scope_type VARCHAR(16) NOT NULL DEFAULT 'global',
    router_id INTEGER,
    sort_order INTEGER NOT NULL DEFAULT 0,
    passthrough BOOLEAN NOT NULL DEFAULT 0,
    enabled BOOLEAN NOT NULL DEFAULT 1,
    tiered BOOLEAN NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(router_id) REFERENCES routers(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS fair_usage_tiers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rule_id INTEGER NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    threshold_bytes BIGINT NOT NULL,
    name VARCHAR(128) NOT NULL DEFAULT '',
    throttle_download_kbps INTEGER NOT NULL,
    throttle_upload_kbps INTEGER NOT NULL,
    FOREIGN KEY(rule_id) REFERENCES fair_usage_rules(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS fair_usage_assignments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rule_id INTEGER NOT NULL,
    peer_id INTEGER NOT NULL,
    FOREIGN KEY(rule_id) REFERENCES fair_usage_rules(id) ON DELETE CASCADE,
    FOREIGN KEY(peer_id) REFERENCES peers(id) ON DELETE CASCADE,
    UNIQUE(rule_id, peer_id)
);

CREATE TABLE IF NOT EXISTS fair_usage_state (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    peer_id INTEGER NOT NULL UNIQUE,
    rule_id INTEGER NOT NULL,
    tier_id INTEGER,
    throttled BOOLEAN NOT NULL DEFAULT 0,
    ros_queue_id VARCHAR(64) DEFAULT '',
    throttled_at DATETIME,
    FOREIGN KEY(peer_id) REFERENCES peers(id) ON DELETE CASCADE,
    FOREIGN KEY(rule_id) REFERENCES fair_usage_rules(id) ON DELETE CASCADE,
    FOREIGN KEY(tier_id) REFERENCES fair_usage_tiers(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS actions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    peer_id INTEGER,
    ts DATETIME NOT NULL,
    action VARCHAR(64) NOT NULL,
    note TEXT NOT NULL,
    FOREIGN KEY(peer_id) REFERENCES peers(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS settings_kv (
    key VARCHAR(64) PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username VARCHAR(64) NOT NULL UNIQUE,
    hashed_password TEXT NOT NULL,
    is_admin BOOLEAN NOT NULL DEFAULT 1,
    is_active BOOLEAN NOT NULL DEFAULT 1,
    session_version INTEGER NOT NULL DEFAULT 1,
    password_changed_at DATETIME,
    last_login_at DATETIME,
    failed_login_attempts INTEGER NOT NULL DEFAULT 0,
    locked_until DATETIME,
    must_change_password BOOLEAN NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS user_security_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_user_id INTEGER,
    target_user_id INTEGER,
    event_type VARCHAR(64) NOT NULL,
    detail TEXT NOT NULL DEFAULT '',
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(actor_user_id) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY(target_user_id) REFERENCES users(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS telegram_users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    telegram_user_id BIGINT NOT NULL UNIQUE,
    telegram_username VARCHAR(255) NOT NULL DEFAULT '',
    first_name VARCHAR(255) NOT NULL DEFAULT '',
    last_name VARCHAR(255) NOT NULL DEFAULT '',
    language VARCHAR(4) NOT NULL DEFAULT 'en',
    is_blocked BOOLEAN NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS telegram_peer_bindings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    telegram_user_id INTEGER NOT NULL,
    peer_id INTEGER NOT NULL,
    visible BOOLEAN NOT NULL DEFAULT 1,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(telegram_user_id) REFERENCES telegram_users(id) ON DELETE CASCADE,
    FOREIGN KEY(peer_id) REFERENCES peers(id) ON DELETE CASCADE,
    UNIQUE(telegram_user_id, peer_id)
);

CREATE TABLE IF NOT EXISTS telegram_signup_tokens (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    token VARCHAR(64) NOT NULL UNIQUE,
    peer_ids TEXT NOT NULL DEFAULT '[]',
    created_by INTEGER,
    used_by INTEGER,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    used_at DATETIME,
    expires_at DATETIME,
    single_use BOOLEAN NOT NULL DEFAULT 1,
    FOREIGN KEY(created_by) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY(used_by) REFERENCES telegram_users(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS telegram_notification_config (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_type VARCHAR(32) NOT NULL UNIQUE,
    notify_clients BOOLEAN NOT NULL DEFAULT 1,
    notify_admin BOOLEAN NOT NULL DEFAULT 1,
    enabled BOOLEAN NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS telegram_user_notification_preferences (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    telegram_user_id INTEGER NOT NULL,
    event_type VARCHAR(32) NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT 1,
    FOREIGN KEY(telegram_user_id) REFERENCES telegram_users(id) ON DELETE CASCADE,
    UNIQUE(telegram_user_id, event_type)
);

CREATE TABLE IF NOT EXISTS telegram_notification_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    telegram_user_id INTEGER NOT NULL,
    peer_id INTEGER,
    event_type VARCHAR(32) NOT NULL,
    sent_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    message_hash VARCHAR(64) NOT NULL DEFAULT '',
    FOREIGN KEY(telegram_user_id) REFERENCES telegram_users(id) ON DELETE CASCADE,
    FOREIGN KEY(peer_id) REFERENCES peers(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS telegram_broadcasts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_by_user_id INTEGER,
    body TEXT NOT NULL DEFAULT '',
    photo_path VARCHAR(512),
    photo_filename VARCHAR(255) NOT NULL DEFAULT '',
    photo_mime VARCHAR(64) NOT NULL DEFAULT '',
    photo_size_bytes INTEGER NOT NULL DEFAULT 0,
    recipient_mode VARCHAR(16) NOT NULL DEFAULT 'all',
    status VARCHAR(24) NOT NULL DEFAULT 'queued',
    total_count INTEGER NOT NULL DEFAULT 0,
    sent_count INTEGER NOT NULL DEFAULT 0,
    failed_count INTEGER NOT NULL DEFAULT 0,
    acknowledged_count INTEGER NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    started_at DATETIME,
    finished_at DATETIME,
    FOREIGN KEY(created_by_user_id) REFERENCES users(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS telegram_broadcast_recipients (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    broadcast_id INTEGER NOT NULL,
    telegram_user_id INTEGER,
    chat_id BIGINT NOT NULL,
    display_name VARCHAR(255) NOT NULL DEFAULT '',
    status VARCHAR(24) NOT NULL DEFAULT 'pending',
    telegram_message_id INTEGER,
    error_code VARCHAR(64) NOT NULL DEFAULT '',
    error_message TEXT NOT NULL DEFAULT '',
    sent_at DATETIME,
    acknowledged_at DATETIME,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(broadcast_id) REFERENCES telegram_broadcasts(id) ON DELETE CASCADE,
    FOREIGN KEY(telegram_user_id) REFERENCES telegram_users(id) ON DELETE SET NULL,
    UNIQUE(broadcast_id, telegram_user_id)
);
"#;

pub fn initialize_database(conn: &Connection) -> Result<()> {
    conn.execute_batch(DDL_SCHEMA)?;

    // Ensure Indexes
    conn.execute_batch(r#"
        CREATE INDEX IF NOT EXISTS ix_usage_samples_peer_id_ts ON usage_samples (peer_id, ts);
        CREATE INDEX IF NOT EXISTS ix_usage_minute_peer_id_minute_ts ON usage_minute (peer_id, minute_ts);
        CREATE INDEX IF NOT EXISTS ix_peers_selected_router_id_id ON peers (selected, router_id, id);
        CREATE INDEX IF NOT EXISTS ix_peers_router_id_interface ON peers (router_id, interface);
        CREATE INDEX IF NOT EXISTS ix_fair_usage_tiers_rule_id ON fair_usage_tiers (rule_id);
        CREATE INDEX IF NOT EXISTS ix_user_security_events_actor_user_id ON user_security_events (actor_user_id);
        CREATE INDEX IF NOT EXISTS ix_user_security_events_target_user_id ON user_security_events (target_user_id);
        CREATE INDEX IF NOT EXISTS ix_user_security_events_event_type ON user_security_events (event_type);
        CREATE INDEX IF NOT EXISTS ix_user_security_events_created_at ON user_security_events (created_at);
        CREATE INDEX IF NOT EXISTS ix_telegram_broadcast_recipients_broadcast_id ON telegram_broadcast_recipients (broadcast_id);
        CREATE INDEX IF NOT EXISTS ix_telegram_broadcast_recipients_chat_id ON telegram_broadcast_recipients (chat_id);
    "#)?;

    // Run Column Backfill Migrations if old DB
    run_migrations(conn)?;

    Ok(())
}

fn table_columns(conn: &Connection, table: &str) -> Result<std::collections::HashSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut cols = std::collections::HashSet::new();
    for col in rows {
        if let Ok(c) = col {
            cols.insert(c);
        }
    }
    Ok(cols)
}

pub fn run_migrations(conn: &Connection) -> Result<()> {
    // 1. fair_usage_rules
    if let Ok(cols) = table_columns(conn, "fair_usage_rules") {
        if !cols.contains("scope_period_count") {
            let _ = conn.execute("ALTER TABLE fair_usage_rules ADD COLUMN scope_period_count INTEGER DEFAULT 1", []);
        }
        if !cols.contains("scope_period_unit") {
            let _ = conn.execute("ALTER TABLE fair_usage_rules ADD COLUMN scope_period_unit VARCHAR(8) DEFAULT 'month'", []);
        }
        if !cols.contains("tiered") {
            let _ = conn.execute("ALTER TABLE fair_usage_rules ADD COLUMN tiered BOOLEAN DEFAULT 0", []);
        }
        if !cols.contains("sort_order") {
            let _ = conn.execute("ALTER TABLE fair_usage_rules ADD COLUMN sort_order INTEGER DEFAULT 0", []);
        }
        if !cols.contains("passthrough") {
            let _ = conn.execute("ALTER TABLE fair_usage_rules ADD COLUMN passthrough BOOLEAN DEFAULT 0", []);
        }
    }

    // 2. routers
    if let Ok(cols) = table_columns(conn, "routers") {
        if !cols.contains("enabled") {
            let _ = conn.execute("ALTER TABLE routers ADD COLUMN enabled BOOLEAN DEFAULT 1 NOT NULL", []);
        }
        if !cols.contains("ros_version") {
            let _ = conn.execute("ALTER TABLE routers ADD COLUMN ros_version VARCHAR(64) DEFAULT '' NOT NULL", []);
        }
        if !cols.contains("ros_version_checked_at") {
            let _ = conn.execute("ALTER TABLE routers ADD COLUMN ros_version_checked_at DATETIME", []);
        }
        if !cols.contains("ros_supported") {
            let _ = conn.execute("ALTER TABLE routers ADD COLUMN ros_supported BOOLEAN DEFAULT 0 NOT NULL", []);
        }
    }

    // 3. peers
    if let Ok(cols) = table_columns(conn, "peers") {
        if !cols.contains("router_sync_status") {
            let _ = conn.execute("ALTER TABLE peers ADD COLUMN router_sync_status VARCHAR(16) DEFAULT 'synced' NOT NULL", []);
        }
        if !cols.contains("router_sync_first_seen_at") {
            let _ = conn.execute("ALTER TABLE peers ADD COLUMN router_sync_first_seen_at DATETIME", []);
        }
        if !cols.contains("router_sync_last_seen_at") {
            let _ = conn.execute("ALTER TABLE peers ADD COLUMN router_sync_last_seen_at DATETIME", []);
        }
    }

    // 4. users
    if let Ok(cols) = table_columns(conn, "users") {
        if !cols.contains("is_active") {
            let _ = conn.execute("ALTER TABLE users ADD COLUMN is_active BOOLEAN DEFAULT 1 NOT NULL", []);
        }
        if !cols.contains("session_version") {
            let _ = conn.execute("ALTER TABLE users ADD COLUMN session_version INTEGER DEFAULT 1 NOT NULL", []);
        }
        if !cols.contains("password_changed_at") {
            let _ = conn.execute("ALTER TABLE users ADD COLUMN password_changed_at DATETIME", []);
        }
        if !cols.contains("last_login_at") {
            let _ = conn.execute("ALTER TABLE users ADD COLUMN last_login_at DATETIME", []);
        }
        if !cols.contains("failed_login_attempts") {
            let _ = conn.execute("ALTER TABLE users ADD COLUMN failed_login_attempts INTEGER DEFAULT 0 NOT NULL", []);
        }
        if !cols.contains("locked_until") {
            let _ = conn.execute("ALTER TABLE users ADD COLUMN locked_until DATETIME", []);
        }
        if !cols.contains("must_change_password") {
            let _ = conn.execute("ALTER TABLE users ADD COLUMN must_change_password BOOLEAN DEFAULT 0 NOT NULL", []);
        }
    }

    // 5. fair_usage_state
    if let Ok(cols) = table_columns(conn, "fair_usage_state") {
        if !cols.contains("tier_id") {
            let _ = conn.execute("ALTER TABLE fair_usage_state ADD COLUMN tier_id INTEGER REFERENCES fair_usage_tiers (id) ON DELETE SET NULL", []);
        }
    }

    Ok(())
}
