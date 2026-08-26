use super::tiers::{active_tier_for_combined_usage, ordered_tiers_for_rule};
use super::usage::peer_scope_usage_for_rule;
use crate::calendar::Tz;
use crate::db::models::{FairUsageRule, FairUsageState, FairUsageTier, Peer};
use crate::db::DbPool;
use crate::routeros::AnyRouterOSClient;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Result};

pub const FU_QUEUE_PREFIX: &str = "wgmik-fu-";

pub fn get_applicable_fair_usage_rules(conn: &Connection, peer: &Peer) -> Result<Vec<FairUsageRule>> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.name, r.description, r.quota_mode, r.download_quota_bytes, r.upload_quota_bytes,
                r.throttle_download_kbps, r.throttle_upload_kbps, r.time_scope, r.scope_period_count,
                r.scope_period_unit, r.scope_type, r.router_id, r.sort_order, r.passthrough, r.enabled,
                r.tiered, r.created_at, r.updated_at
         FROM fair_usage_assignments a
         JOIN fair_usage_rules r ON a.rule_id = r.id
         WHERE a.peer_id = ?1 AND r.enabled = 1
         ORDER BY r.sort_order ASC, r.id ASC",
    )?;
    let rows = stmt.query_map(params![peer.id], |row| map_fair_usage_rule(row))?;
    let mut assigned = Vec::new();
    for r in rows {
        assigned.push(r?);
    }
    if !assigned.is_empty() {
        return Ok(assigned);
    }

    let mut stmt2 = conn.prepare(
        "SELECT id, name, description, quota_mode, download_quota_bytes, upload_quota_bytes,
                throttle_download_kbps, throttle_upload_kbps, time_scope, scope_period_count,
                scope_period_unit, scope_type, router_id, sort_order, passthrough, enabled,
                tiered, created_at, updated_at
         FROM fair_usage_rules
         WHERE enabled = 1 AND (scope_type = 'global' OR (scope_type = 'router' AND router_id = ?1))
         ORDER BY sort_order ASC, id ASC",
    )?;
    let rows2 = stmt2.query_map(params![peer.router_id], |row| map_fair_usage_rule(row))?;
    let mut list = Vec::new();
    for r in rows2 {
        list.push(r?);
    }
    Ok(list)
}

fn map_fair_usage_rule(row: &rusqlite::Row) -> rusqlite::Result<FairUsageRule> {
    Ok(FairUsageRule {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        quota_mode: row.get(3)?,
        download_quota_bytes: row.get(4)?,
        upload_quota_bytes: row.get(5)?,
        throttle_download_kbps: row.get(6)?,
        throttle_upload_kbps: row.get(7)?,
        time_scope: row.get(8)?,
        scope_period_count: row.get(9)?,
        scope_period_unit: row.get(10)?,
        scope_type: row.get(11)?,
        router_id: row.get(12)?,
        sort_order: row.get(13)?,
        passthrough: row.get(14)?,
        enabled: row.get(15)?,
        tiered: row.get(16)?,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    })
}

pub fn is_rule_over_quota(
    conn: &Connection,
    rule: &FairUsageRule,
    used_rx: i64,
    used_tx: i64,
) -> bool {
    if rule.tiered {
        let tiers = ordered_tiers_for_rule(conn, rule.id).unwrap_or_default();
        if tiers.is_empty() {
            return false;
        }
        let combined = used_rx + used_tx;
        return active_tier_for_combined_usage(&tiers, combined).is_some();
    }

    if rule.quota_mode == "combined" {
        (used_rx + used_tx) >= rule.download_quota_bytes
    } else {
        let over_dl = if rule.download_quota_bytes > 0 { used_rx >= rule.download_quota_bytes } else { false };
        let over_ul = if let Some(ul) = rule.upload_quota_bytes { ul > 0 && used_tx >= ul } else { false };
        over_dl || over_ul
    }
}

pub fn evaluate_fair_usage_chain(
    conn: &Connection,
    peer: &Peer,
    now_utc: DateTime<Utc>,
    tz: Tz,
    calendar: &str,
    reset_day: i32,
) -> Option<(FairUsageRule, Option<FairUsageTier>)> {
    let rules = get_applicable_fair_usage_rules(conn, peer).ok()?;
    if rules.is_empty() {
        return None;
    }

    let mut winner: Option<(FairUsageRule, Option<FairUsageTier>)> = None;
    for rule in rules {
        let (urx, utx) = peer_scope_usage_for_rule(conn, peer.id, &rule, now_utc, tz, calendar, reset_day);
        let tier = if rule.tiered {
            let tiers = ordered_tiers_for_rule(conn, rule.id).unwrap_or_default();
            active_tier_for_combined_usage(&tiers, urx + utx).cloned()
        } else {
            None
        };

        let matched = if rule.tiered {
            tier.is_some()
        } else {
            is_rule_over_quota(conn, &rule, urx, utx)
        };

        if !matched {
            continue;
        }

        winner = Some((rule.clone(), tier));
        if !rule.passthrough {
            break;
        }
    }

    winner
}

pub async fn apply_fair_usage_policy(
    pool: &DbPool,
    peer: &Peer,
    client: Option<&AnyRouterOSClient>,
    now_utc: DateTime<Utc>,
    tz: Tz,
    calendar: &str,
    reset_day: i32,
) {
    let (rules_empty, state, winner) = {
        let conn = match pool.get() {
            Ok(c) => c,
            Err(_) => return,
        };
        let rules = get_applicable_fair_usage_rules(&conn, peer).unwrap_or_default();
        let state_res = conn.query_row(
            "SELECT id, peer_id, rule_id, tier_id, throttled, ros_queue_id FROM fair_usage_state WHERE peer_id = ?1",
            params![peer.id],
            |row| {
                Ok(FairUsageState {
                    id: row.get(0)?,
                    peer_id: row.get(1)?,
                    rule_id: row.get(2)?,
                    tier_id: row.get(3)?,
                    throttled: row.get(4)?,
                    ros_queue_id: row.get(5)?,
                    throttled_at: None,
                })
            },
        );
        let win = evaluate_fair_usage_chain(&conn, peer, now_utc, tz, calendar, reset_day);
        (rules.is_empty(), state_res.ok(), win)
    };

    if rules_empty {
        if let Some(st) = state {
            if st.throttled {
                if let Some(c) = client {
                    if !st.ros_queue_id.is_empty() && !peer.ros_id.is_empty() {
                        let _ = c.remove_simple_queue(&st.ros_queue_id).await;
                    }
                }
                if let Ok(conn) = pool.get() {
                    let _ = conn.execute("DELETE FROM fair_usage_state WHERE peer_id = ?1", params![peer.id]);
                    let ts_str = now_utc.format("%Y-%m-%d %H:%M:%S").to_string();
                    let _ = conn.execute(
                        "INSERT INTO actions (peer_id, ts, action, note) VALUES (?1, ?2, ?3, ?4)",
                        params![peer.id, ts_str, "fu_reset", "Rule removed or disabled; throttle lifted"],
                    );
                }
            } else if let Ok(conn) = pool.get() {
                let _ = conn.execute("DELETE FROM fair_usage_state WHERE peer_id = ?1", params![peer.id]);
            }
        }
        return;
    }

    if winner.is_none() {
        if let Some(st) = state {
            if st.throttled {
                if let Some(c) = client {
                    if !st.ros_queue_id.is_empty() && !peer.ros_id.is_empty() {
                        let _ = c.remove_simple_queue(&st.ros_queue_id).await;
                    }
                }
                if let Ok(conn) = pool.get() {
                    let _ = conn.execute("DELETE FROM fair_usage_state WHERE peer_id = ?1", params![peer.id]);
                    let ts_str = now_utc.format("%Y-%m-%d %H:%M:%S").to_string();
                    let _ = conn.execute(
                        "INSERT INTO actions (peer_id, ts, action, note) VALUES (?1, ?2, ?3, ?4)",
                        params![peer.id, ts_str, "fu_reset", "Auto-reset: usage below quota for all applicable fair-usage rules"],
                    );
                }
            }
        }
        return;
    }

    let (winning_rule, winning_tier) = winner.unwrap();
    let (down_kbps, up_kbps) = if let Some(t) = &winning_tier {
        (t.throttle_download_kbps, t.throttle_upload_kbps)
    } else {
        (winning_rule.throttle_download_kbps, winning_rule.throttle_upload_kbps)
    };

    let up_limit = format!("{}k", up_kbps);
    let down_limit = format!("{}k", down_kbps);
    let log_label = if let Some(t) = &winning_tier {
        if !t.name.trim().is_empty() {
            format!("{} ({})", winning_rule.name, t.name.trim())
        } else {
            winning_rule.name.clone()
        }
    } else {
        winning_rule.name.clone()
    };

    let name_or_id = if !peer.name.is_empty() { peer.name.clone() } else { peer.id.to_string() };
    let queue_name = format!("{}{}", FU_QUEUE_PREFIX, name_or_id);
    let target = &peer.allowed_address;

    if let Some(c) = client {
        if !peer.ros_id.is_empty() {
            let mut ros_queue_id = state.as_ref().map(|s| s.ros_queue_id.clone()).unwrap_or_default();
            let mut synced = false;

            if !ros_queue_id.is_empty() {
                if c.update_simple_queue(&ros_queue_id, &up_limit, &down_limit).await.is_ok() {
                    synced = true;
                } else {
                    let _ = c.remove_simple_queue(&ros_queue_id).await;
                    ros_queue_id.clear();
                }
            }

            if !synced {
                if let Ok(queues) = c.list_simple_queues(FU_QUEUE_PREFIX).await {
                    for q in queues {
                        if q.name == queue_name && !q.ros_id.is_empty() {
                            if c.update_simple_queue(&q.ros_id, &up_limit, &down_limit).await.is_ok() {
                                ros_queue_id = q.ros_id;
                                synced = true;
                                break;
                            }
                        }
                    }
                }
            }

            if !synced {
                if let Ok(new_id) = c.add_simple_queue(&queue_name, target, &up_limit, &down_limit, "wgmik fair-usage auto").await {
                    ros_queue_id = new_id;
                }
            }

            if let Ok(conn) = pool.get() {
                let ts_str = now_utc.format("%Y-%m-%d %H:%M:%S").to_string();
                let tier_id = winning_tier.as_ref().map(|t| t.id);

                let _ = conn.execute(
                    r#"
                    INSERT INTO fair_usage_state (peer_id, rule_id, tier_id, throttled, ros_queue_id, throttled_at)
                    VALUES (?1, ?2, ?3, 1, ?4, ?5)
                    ON CONFLICT(peer_id) DO UPDATE SET
                        rule_id = excluded.rule_id,
                        tier_id = excluded.tier_id,
                        throttled = 1,
                        ros_queue_id = excluded.ros_queue_id,
                        throttled_at = COALESCE(fair_usage_state.throttled_at, excluded.throttled_at)
                    "#,
                    params![peer.id, winning_rule.id, tier_id, ros_queue_id, ts_str],
                );

                let entered_unthrottled = state.as_ref().map(|s| !s.throttled).unwrap_or(true);
                if entered_unthrottled {
                    let note = format!("Throttled: {} ({}/{})", log_label, up_limit, down_limit);
                    let _ = conn.execute(
                        "INSERT INTO actions (peer_id, ts, action, note) VALUES (?1, ?2, ?3, ?4)",
                        params![peer.id, ts_str, "fu_throttle", note],
                    );
                }
            }
        }
    }
}
