use crate::accounting::deltas::{counter_day_key, counter_delta};
use crate::accounting::storage::{floor_to_minute_utc, upsert_usage_minute};
use crate::calendar::parse_timezone;
use crate::config::AppSettings;
use crate::db::models::{Peer, Router};
use crate::db::DbPool;
use crate::fair_usage::apply_fair_usage_policy;
use crate::ops::ExclusiveOperationGate;
use crate::routeros::factory::make_client;
use crate::routeros::version::is_routeros_supported;
use chrono::Utc;
use rusqlite::params;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};

pub struct Scheduler {
    pool: DbPool,
    settings: AppSettings,
    gate: ExclusiveOperationGate,
    running: Arc<AtomicBool>,
}

impl Scheduler {
    pub fn new(pool: DbPool, settings: AppSettings, gate: ExclusiveOperationGate) -> Self {
        Self {
            pool,
            settings,
            gate,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn start(self: Arc<Self>) {
        self.running.store(true, Ordering::SeqCst);
        let s = self.clone();
        tokio::spawn(async move {
            s.run_polling_loop().await;
        });
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    async fn run_polling_loop(&self) {
        info!("Scheduler polling loop started");
        while self.running.load(Ordering::SeqCst) {
            let interval = self.get_poll_interval();
            self.poll_once().await;
            sleep(Duration::from_secs(interval)).await;
        }
    }

    fn get_poll_interval(&self) -> u64 {
        let conn = match self.pool.get() {
            Ok(c) => c,
            Err(_) => return self.settings.poll_interval_seconds,
        };
        conn.query_row(
            "SELECT value FROM settings_kv WHERE key = 'poll_interval_seconds'",
            [],
            |row| row.get::<_, String>(0),
        ).ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(self.settings.poll_interval_seconds)
    }

    pub async fn poll_once(&self) {
        if self.gate.is_active() {
            return;
        }

        let now_utc = Utc::now();
        let tz_str = self.get_setting("timezone").unwrap_or_else(|| self.settings.timezone.clone());
        let tz = parse_timezone(&tz_str);
        let calendar = self.get_setting("date_calendar").unwrap_or_else(|| self.settings.date_calendar.clone());
        let reset_day = self.get_setting("monthly_reset_day").and_then(|v| v.parse::<i32>().ok()).unwrap_or(self.settings.monthly_reset_day as i32);
        let day_key = counter_day_key(now_utc, tz);
        let month_key = now_utc.format("%Y-%m").to_string();

        let routers = {
            let conn = match self.pool.get() {
                Ok(c) => c,
                Err(_) => return,
            };
            let mut stmt = match conn.prepare("SELECT id, name, host, proto, port, username, secret_enc, tls_verify, enabled, ros_version, ros_supported FROM routers WHERE enabled = 1 AND ros_supported = 1") {
                Ok(s) => s,
                Err(_) => return,
            };
            let rows = stmt.query_map([], |row| {
                Ok(Router {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    host: row.get(2)?,
                    proto: row.get(3)?,
                    port: row.get(4)?,
                    username: row.get(5)?,
                    secret_enc: row.get(6)?,
                    tls_verify: row.get(7)?,
                    enabled: row.get(8)?,
                    ros_version: row.get(9)?,
                    ros_version_checked_at: None,
                    ros_supported: row.get(10)?,
                })
            });
            let mut list = Vec::new();
            if let Ok(r) = rows {
                for item in r {
                    if let Ok(rt) = item {
                        list.push(rt);
                    }
                }
            }
            list
        };

        for router in routers {
            if !is_routeros_supported(Some(&router.ros_version)) {
                continue;
            }

            let client = make_client(&router, &self.settings.secret_key, Some(10));
            let live_peers = match client.list_all_wireguard_peers().await {
                Ok(p) => p,
                Err(e) => {
                    warn!("Failed to poll router {} ({}): {}", router.name, router.host, e);
                    continue;
                }
            };

            let conn = match self.pool.get() {
                Ok(c) => c,
                Err(_) => continue,
            };

            let db_peers = {
                let mut stmt = match conn.prepare(
                    "SELECT id, router_id, interface, ros_id, name, public_key, allowed_address,
                            comment, disabled, selected, router_sync_status
                     FROM peers WHERE router_id = ?1",
                ) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let rows = stmt.query_map(params![router.id], |row| {
                    Ok(Peer {
                        id: row.get(0)?,
                        router_id: row.get(1)?,
                        interface: row.get(2)?,
                        ros_id: row.get(3)?,
                        name: row.get(4)?,
                        public_key: row.get(5)?,
                        allowed_address: row.get(6)?,
                        comment: row.get(7)?,
                        disabled: row.get(8)?,
                        selected: row.get(9)?,
                        router_sync_status: row.get(10)?,
                        router_sync_first_seen_at: None,
                        router_sync_last_seen_at: None,
                    })
                });
                let mut list = Vec::new();
                if let Ok(r) = rows {
                    for item in r {
                        if let Ok(p) = item {
                            list.push(p);
                        }
                    }
                }
                list
            };

            let now_str = now_utc.format("%Y-%m-%d %H:%M:%S").to_string();

            // Match live peers with DB
            for lp in &live_peers {
                if lp.interface.is_empty() || lp.public_key.is_empty() {
                    continue;
                }
                let existing = db_peers.iter().find(|p| p.interface == lp.interface && p.public_key == lp.public_key);
                if let Some(peer) = existing {
                    if peer.router_sync_status == "missing" {
                        let _ = conn.execute(
                            "UPDATE peers SET router_sync_status = 'synced', router_sync_first_seen_at = NULL, router_sync_last_seen_at = NULL WHERE id = ?1",
                            params![peer.id],
                        );
                    }
                    if !lp.ros_id.is_empty() && peer.ros_id != lp.ros_id {
                        let _ = conn.execute("UPDATE peers SET ros_id = ?1 WHERE id = ?2", params![lp.ros_id, peer.id]);
                    }
                    if peer.name != lp.name {
                        let _ = conn.execute("UPDATE peers SET name = ?1 WHERE id = ?2", params![lp.name, peer.id]);
                    }
                    if peer.allowed_address != lp.allowed_address {
                        let _ = conn.execute("UPDATE peers SET allowed_address = ?1 WHERE id = ?2", params![lp.allowed_address, peer.id]);
                    }
                    if peer.disabled != lp.disabled {
                        let _ = conn.execute("UPDATE peers SET disabled = ?1 WHERE id = ?2", params![lp.disabled, peer.id]);
                    }
                } else {
                    // New peer discovered on RouterOS
                    let _ = conn.execute(
                        r#"
                        INSERT INTO peers (router_id, interface, ros_id, name, public_key, allowed_address, disabled, selected, router_sync_status, router_sync_first_seen_at, router_sync_last_seen_at)
                        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 'new', ?8, ?8)
                        "#,
                        params![router.id, lp.interface, lp.ros_id, lp.name, lp.public_key, lp.allowed_address, lp.disabled, now_str],
                    );
                }
            }

            // Mark missing peers
            for peer in &db_peers {
                let live_found = live_peers.iter().any(|lp| lp.interface == peer.interface && lp.public_key == peer.public_key);
                if !live_found {
                    if peer.router_sync_status == "new" {
                        let _ = conn.execute("DELETE FROM peers WHERE id = ?1", params![peer.id]);
                    } else if peer.selected && peer.router_sync_status != "missing" {
                        let _ = conn.execute(
                            "UPDATE peers SET router_sync_status = 'missing', router_sync_first_seen_at = COALESCE(router_sync_first_seen_at, ?1), router_sync_last_seen_at = ?1 WHERE id = ?2",
                            params![now_str, peer.id],
                        );
                    }
                }
            }

            // Usage accounting for selected & synced peers
            let selected_peers: Vec<Peer> = db_peers.into_iter().filter(|p| p.selected && p.router_sync_status == "synced").collect();
            for peer in selected_peers {
                let lp = match live_peers.iter().find(|lp| lp.interface == peer.interface && lp.public_key == peer.public_key) {
                    Some(p) => p,
                    None => continue,
                };

                let last_sample = conn.query_row(
                    "SELECT rx, tx FROM usage_samples WHERE peer_id = ?1 ORDER BY ts DESC LIMIT 1",
                    params![peer.id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                ).ok();

                // Save raw sample
                let _ = conn.execute(
                    "INSERT INTO usage_samples (peer_id, ts, rx, tx, endpoint) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![peer.id, now_str, lp.rx_bytes, lp.tx_bytes, lp.endpoint],
                );

                let mut delta_rx = 0;
                let mut delta_tx = 0;
                if let Some((prev_rx, prev_tx)) = last_sample {
                    let rx_res = counter_delta(prev_rx, lp.rx_bytes);
                    let tx_res = counter_delta(prev_tx, lp.tx_bytes);
                    delta_rx = rx_res.delta;
                    delta_tx = tx_res.delta;
                }

                if delta_rx > 0 || delta_tx > 0 {
                    let minute_ts = floor_to_minute_utc(now_utc);
                    let _ = upsert_usage_minute(&conn, peer.id, minute_ts, delta_rx, delta_tx);

                    // Daily upsert
                    let _ = conn.execute(
                        r#"
                        INSERT INTO usage_daily (peer_id, day, rx, tx)
                        VALUES (?1, ?2, ?3, ?4)
                        ON CONFLICT(peer_id, day) DO UPDATE SET
                            rx = usage_daily.rx + excluded.rx,
                            tx = usage_daily.tx + excluded.tx
                        "#,
                        params![peer.id, day_key, delta_rx, delta_tx],
                    );

                    // Monthly upsert
                    let _ = conn.execute(
                        r#"
                        INSERT INTO usage_monthly (peer_id, month_key, rx, tx)
                        VALUES (?1, ?2, ?3, ?4)
                        ON CONFLICT(peer_id, month_key) DO UPDATE SET
                            rx = usage_monthly.rx + excluded.rx,
                            tx = usage_monthly.tx + excluded.tx
                        "#,
                        params![peer.id, month_key, delta_rx, delta_tx],
                    );
                }

                // Fair usage policy enforcement
                apply_fair_usage_policy(&self.pool, &peer, Some(&client), now_utc, tz, &calendar, reset_day).await;
            }
        }
    }

    fn get_setting(&self, key: &str) -> Option<String> {
        let conn = self.pool.get().ok()?;
        conn.query_row(
            "SELECT value FROM settings_kv WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        ).ok()
    }
}
