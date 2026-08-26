use super::sync::{evaluate_fair_usage_chain, get_applicable_fair_usage_rules, is_rule_over_quota};
use super::tiers::{active_tier_for_combined_usage, ordered_tiers_for_rule};
use super::usage::{compute_next_reset_utc_for_rule, format_scope_label, normalize_scope_period, peer_scope_usage_for_rule};
use crate::calendar::Tz;
use crate::db::models::Peer;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FairUsageTierStatusDTO {
    pub tier_id: i64,
    pub sort_order: i64,
    pub threshold_bytes: i64,
    pub name: String,
    pub throttle_download_kbps: i64,
    pub throttle_upload_kbps: i64,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FairUsageRuleStatusItemDTO {
    pub rule_id: i64,
    pub rule_name: String,
    pub quota_mode: String,
    pub download_quota_bytes: i64,
    pub upload_quota_bytes: Option<i64>,
    pub throttle_download_kbps: i64,
    pub throttle_upload_kbps: i64,
    pub time_scope: Option<String>,
    pub scope_period_count: i64,
    pub scope_period_unit: String,
    pub scope_label: String,
    pub scope_type: Option<String>,
    pub sort_order: i64,
    pub passthrough: bool,
    pub used_rx: i64,
    pub used_tx: i64,
    pub over_quota: bool,
    pub is_effective: bool,
    pub next_reset: Option<String>,
    pub tiered: bool,
    pub tiers: Vec<FairUsageTierStatusDTO>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FairUsagePeerStatusDTO {
    pub peer_id: i64,
    pub rules: Vec<FairUsageRuleStatusItemDTO>,
    pub rule_id: Option<i64>,
    pub rule_name: Option<String>,
    pub quota_mode: Option<String>,
    pub download_quota_bytes: i64,
    pub upload_quota_bytes: Option<i64>,
    pub throttle_download_kbps: i64,
    pub throttle_upload_kbps: i64,
    pub time_scope: Option<String>,
    pub scope_period_count: i64,
    pub scope_period_unit: String,
    pub scope_label: String,
    pub scope_type: Option<String>,
    pub sort_order: i64,
    pub passthrough: bool,
    pub used_rx: i64,
    pub used_tx: i64,
    pub throttled: bool,
    pub throttled_at: Option<String>,
    pub next_reset: Option<String>,
}

pub fn build_fair_usage_peer_status_dto(
    conn: &Connection,
    peer: &Peer,
    now_utc: DateTime<Utc>,
    tz: Tz,
    calendar: &str,
    reset_day: i32,
) -> FairUsagePeerStatusDTO {
    let applicable = get_applicable_fair_usage_rules(conn, peer).unwrap_or_default();
    if applicable.is_empty() {
        return FairUsagePeerStatusDTO {
            peer_id: peer.id,
            rules: Vec::new(),
            rule_id: None,
            rule_name: None,
            quota_mode: None,
            download_quota_bytes: 0,
            upload_quota_bytes: None,
            throttle_download_kbps: 0,
            throttle_upload_kbps: 0,
            time_scope: None,
            scope_period_count: 1,
            scope_period_unit: "month".to_string(),
            scope_label: "Monthly".to_string(),
            scope_type: None,
            sort_order: 0,
            passthrough: false,
            used_rx: 0,
            used_tx: 0,
            throttled: false,
            throttled_at: None,
            next_reset: None,
        };
    }

    let state_row = conn.query_row(
        "SELECT throttled, throttled_at FROM fair_usage_state WHERE peer_id = ?1",
        params![peer.id],
        |row| Ok((row.get::<_, bool>(0).unwrap_or(false), row.get::<_, Option<String>>(1).unwrap_or(None))),
    ).ok();

    let (throttled, throttled_at) = state_row.unwrap_or((false, None));

    let winner = evaluate_fair_usage_chain(conn, peer, now_utc, tz, calendar, reset_day);
    let winning_rule_id = winner.as_ref().map(|(r, _)| r.id);

    let mut items = Vec::new();
    for rule in &applicable {
        let (urx, utx) = peer_scope_usage_for_rule(conn, peer.id, rule, now_utc, tz, calendar, reset_day);
        let (cnt, unit) = normalize_scope_period(rule);
        let nxt = compute_next_reset_utc_for_rule(rule, now_utc, tz, calendar, reset_day);
        let oq = is_rule_over_quota(conn, rule, urx, utx);

        let mut tier_status = Vec::new();
        if rule.tiered {
            let tiers = ordered_tiers_for_rule(conn, rule.id).unwrap_or_default();
            let combined = urx + utx;
            let active_id = active_tier_for_combined_usage(&tiers, combined).map(|a| a.id);
            for t in tiers {
                tier_status.push(FairUsageTierStatusDTO {
                    tier_id: t.id,
                    sort_order: t.sort_order,
                    threshold_bytes: t.threshold_bytes,
                    name: t.name,
                    throttle_download_kbps: t.throttle_download_kbps,
                    throttle_upload_kbps: t.throttle_upload_kbps,
                    is_active: active_id == Some(t.id),
                });
            }
        }

        items.push(FairUsageRuleStatusItemDTO {
            rule_id: rule.id,
            rule_name: rule.name.clone(),
            quota_mode: rule.quota_mode.clone(),
            download_quota_bytes: rule.download_quota_bytes,
            upload_quota_bytes: rule.upload_quota_bytes,
            throttle_download_kbps: rule.throttle_download_kbps,
            throttle_upload_kbps: rule.throttle_upload_kbps,
            time_scope: Some(rule.time_scope.clone()),
            scope_period_count: cnt,
            scope_period_unit: unit.clone(),
            scope_label: format_scope_label(cnt, &unit),
            scope_type: Some(rule.scope_type.clone()),
            sort_order: rule.sort_order,
            passthrough: rule.passthrough,
            used_rx: urx,
            used_tx: utx,
            over_quota: oq,
            is_effective: winning_rule_id == Some(rule.id),
            next_reset: Some(nxt.to_rfc3339()),
            tiered: rule.tiered,
            tiers: tier_status,
        });
    }

    let (primary_rule, primary_tier) = if let Some((r, t)) = &winner {
        (r.clone(), t.clone())
    } else {
        (applicable[0].clone(), None)
    };

    let (ur0, ut0) = peer_scope_usage_for_rule(conn, peer.id, &primary_rule, now_utc, tz, calendar, reset_day);
    let (c0, u0) = normalize_scope_period(&primary_rule);
    let n0 = compute_next_reset_utc_for_rule(&primary_rule, now_utc, tz, calendar, reset_day);
    let (td0, tu0) = if let Some(t) = &primary_tier {
        (t.throttle_download_kbps, t.throttle_upload_kbps)
    } else {
        (primary_rule.throttle_download_kbps, primary_rule.throttle_upload_kbps)
    };

    FairUsagePeerStatusDTO {
        peer_id: peer.id,
        rules: items,
        rule_id: Some(primary_rule.id),
        rule_name: Some(primary_rule.name),
        quota_mode: Some(primary_rule.quota_mode),
        download_quota_bytes: primary_rule.download_quota_bytes,
        upload_quota_bytes: primary_rule.upload_quota_bytes,
        throttle_download_kbps: td0,
        throttle_upload_kbps: tu0,
        time_scope: Some(primary_rule.time_scope),
        scope_period_count: c0,
        scope_period_unit: u0.clone(),
        scope_label: format_scope_label(c0, &u0),
        scope_type: Some(primary_rule.scope_type),
        sort_order: primary_rule.sort_order,
        passthrough: primary_rule.passthrough,
        used_rx: ur0,
        used_tx: ut0,
        throttled,
        throttled_at,
        next_reset: Some(n0.to_rfc3339()),
    }
}
