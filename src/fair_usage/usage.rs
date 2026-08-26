use crate::accounting::bucketing::local_bucket_start_utc_naive;
use crate::accounting::storage::floor_to_minute_utc;
use crate::calendar::{selected_month_cycle_bounds_utc, Tz};
use crate::db::models::FairUsageRule;
use chrono::{DateTime, Datelike, Duration, NaiveDateTime, NaiveTime, TimeZone, Utc};
use rusqlite::{params, Connection};

pub fn normalize_scope_period(rule: &FairUsageRule) -> (i64, String) {
    let cnt = rule.scope_period_count.max(1);
    let mut unit = rule.scope_period_unit.trim().to_lowercase();
    if unit != "hour" && unit != "day" && unit != "week" && unit != "month" {
        unit = "month".to_string();
    }
    let cap = match unit.as_str() {
        "hour" => 168,
        "day" => 90,
        "week" => 52,
        _ => 24,
    };
    (cnt.min(cap), unit)
}

pub fn format_scope_label(count: i64, unit: &str) -> String {
    let u = unit.to_lowercase();
    if count == 1 {
        match u.as_str() {
            "hour" => "Hourly".to_string(),
            "day" => "Daily".to_string(),
            "week" => "Weekly".to_string(),
            _ => "Monthly".to_string(),
        }
    } else {
        let plural = match u.as_str() {
            "hour" => "hours",
            "day" => "days",
            "week" => "weeks",
            _ => "months",
        };
        format!("{} {}", count, plural)
    }
}

pub fn sum_usage_minute_range(
    conn: &Connection,
    peer_id: i64,
    start_naive: NaiveDateTime,
    end_naive: NaiveDateTime,
) -> (i64, i64) {
    let start_str = start_naive.format("%Y-%m-%d %H:%M:%S").to_string();
    let end_str = end_naive.format("%Y-%m-%d %H:%M:%S").to_string();

    let mut stmt = match conn.prepare(
        "SELECT COALESCE(SUM(rx), 0), COALESCE(SUM(tx), 0)
         FROM usage_minute
         WHERE peer_id = ?1 AND minute_ts >= ?2 AND minute_ts <= ?3",
    ) {
        Ok(s) => s,
        Err(_) => return (0, 0),
    };

    let res = stmt.query_row(params![peer_id, start_str, end_str], |row| {
        Ok((row.get::<_, i64>(0).unwrap_or(0), row.get::<_, i64>(1).unwrap_or(0)))
    });

    res.unwrap_or((0, 0))
}

pub fn peer_scope_usage_for_rule(
    conn: &Connection,
    peer_id: i64,
    rule: &FairUsageRule,
    now_utc: DateTime<Utc>,
    tz: Tz,
    calendar: &str,
    reset_day: i32,
) -> (i64, i64) {
    let (cnt, unit) = normalize_scope_period(rule);
    let end_naive = floor_to_minute_utc(now_utc).naive_utc();

    if unit == "hour" {
        let interval = cnt * 3600;
        let start_naive = local_bucket_start_utc_naive(now_utc.naive_utc(), interval, tz);
        return sum_usage_minute_range(conn, peer_id, start_naive, end_naive);
    }

    if unit == "day" {
        let now_local = now_utc.with_timezone(&tz);
        let today = now_local.date_naive();
        let start_date = today - Duration::days(cnt - 1);
        let start_local = tz
            .from_local_datetime(&start_date.and_time(NaiveTime::MIN))
            .single()
            .unwrap_or_else(|| tz.from_utc_datetime(&start_date.and_time(NaiveTime::MIN)));
        let start_naive = start_local.with_timezone(&Utc).naive_utc();
        return sum_usage_minute_range(conn, peer_id, start_naive, end_naive);
    }

    if unit == "week" {
        let now_local = now_utc.with_timezone(&tz);
        let today = now_local.date_naive();
        let dow = today.weekday().num_days_from_monday() as i64;
        let week_start = today - Duration::days(dow);
        let start_date = week_start - Duration::days(7 * (cnt - 1));
        let start_local = tz
            .from_local_datetime(&start_date.and_time(NaiveTime::MIN))
            .single()
            .unwrap_or_else(|| tz.from_utc_datetime(&start_date.and_time(NaiveTime::MIN)));
        let start_naive = start_local.with_timezone(&Utc).naive_utc();
        return sum_usage_minute_range(conn, peer_id, start_naive, end_naive);
    }

    let (start_utc, _) = selected_month_cycle_bounds_utc(now_utc, tz, calendar, reset_day, cnt as i32);
    sum_usage_minute_range(conn, peer_id, start_utc.naive_utc(), end_naive)
}

pub fn compute_next_reset_utc_for_rule(
    rule: &FairUsageRule,
    now_utc: DateTime<Utc>,
    tz: Tz,
    calendar: &str,
    reset_day: i32,
) -> DateTime<Utc> {
    let (cnt, unit) = normalize_scope_period(rule);

    if unit == "hour" {
        let interval = cnt * 3600;
        let b_naive = local_bucket_start_utc_naive(now_utc.naive_utc(), interval, tz);
        let b_utc = Utc.from_utc_datetime(&b_naive);
        let b_loc = b_utc.with_timezone(&tz);
        let nxt = b_loc + Duration::hours(cnt);
        return nxt.with_timezone(&Utc);
    }

    if unit == "day" {
        let now_local = now_utc.with_timezone(&tz);
        let today = now_local.date_naive();
        let start_date = today - Duration::days(cnt - 1);
        let start_local = tz
            .from_local_datetime(&start_date.and_time(NaiveTime::MIN))
            .single()
            .unwrap_or_else(|| tz.from_utc_datetime(&start_date.and_time(NaiveTime::MIN)));
        let nxt = start_local + Duration::days(cnt);
        return nxt.with_timezone(&Utc);
    }

    if unit == "week" {
        let now_local = now_utc.with_timezone(&tz);
        let today = now_local.date_naive();
        let dow = today.weekday().num_days_from_monday() as i64;
        let week_start = today - Duration::days(dow);
        let start_date = week_start - Duration::days(7 * (cnt - 1));
        let start_local = tz
            .from_local_datetime(&start_date.and_time(NaiveTime::MIN))
            .single()
            .unwrap_or_else(|| tz.from_utc_datetime(&start_date.and_time(NaiveTime::MIN)));
        let nxt = start_local + Duration::weeks(cnt);
        return nxt.with_timezone(&Utc);
    }

    let (_, end_utc) = selected_month_cycle_bounds_utc(now_utc, tz, calendar, reset_day, cnt as i32);
    end_utc
}
