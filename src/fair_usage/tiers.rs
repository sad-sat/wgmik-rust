use crate::db::models::FairUsageTier;
use rusqlite::{params, Connection, Result};

pub fn ordered_tiers_for_rule(conn: &Connection, rule_id: i64) -> Result<Vec<FairUsageTier>> {
    let mut stmt = conn.prepare(
        "SELECT id, rule_id, sort_order, threshold_bytes, name, throttle_download_kbps, throttle_upload_kbps
         FROM fair_usage_tiers
         WHERE rule_id = ?1
         ORDER BY threshold_bytes ASC, sort_order ASC, id ASC",
    )?;
    let rows = stmt.query_map(params![rule_id], |row| {
        Ok(FairUsageTier {
            id: row.get(0)?,
            rule_id: row.get(1)?,
            sort_order: row.get(2)?,
            threshold_bytes: row.get(3)?,
            name: row.get(4)?,
            throttle_download_kbps: row.get(5)?,
            throttle_upload_kbps: row.get(6)?,
        })
    })?;

    let mut list = Vec::new();
    for r in rows {
        list.push(r?);
    }
    Ok(list)
}

pub fn active_tier_for_combined_usage<'a>(
    tiers: &'a [FairUsageTier],
    combined_bytes: i64,
) -> Option<&'a FairUsageTier> {
    tiers
        .iter()
        .filter(|t| combined_bytes >= t.threshold_bytes)
        .max_by_key(|t| (t.threshold_bytes, t.sort_order, t.id))
}
