use chrono::{Duration, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use std::collections::BTreeMap;

pub fn local_bucket_start_utc_naive(ts_utc_naive: NaiveDateTime, interval: i64, tz: Tz) -> NaiveDateTime {
    if interval <= 0 {
        return ts_utc_naive;
    }
    let dt_utc = Utc.from_utc_datetime(&ts_utc_naive);
    let dt_local = dt_utc.with_timezone(&tz);
    let day_start = dt_local.date_naive().and_time(NaiveTime::MIN);
    let dt_local_naive = dt_local.naive_local();

    let sec = (dt_local_naive - day_start).num_seconds();
    let bucket_sec = (sec / interval) * interval;
    let start_local_naive = day_start + Duration::seconds(bucket_sec);

    let start_local = tz.from_local_datetime(&start_local_naive)
        .single()
        .unwrap_or_else(|| tz.from_utc_datetime(&start_local_naive));

    start_local.with_timezone(&Utc).naive_utc()
}

pub fn aggregate_rows_to_local_buckets(
    rows: &[(NaiveDateTime, i64, i64)],
    interval: i64,
    tz: Tz,
) -> Vec<(NaiveDateTime, i64, i64)> {
    let mut buckets: BTreeMap<NaiveDateTime, (i64, i64)> = BTreeMap::new();
    for &(ts, rx, tx) in rows {
        let b = local_bucket_start_utc_naive(ts, interval, tz);
        let entry = buckets.entry(b).or_insert((0, 0));
        entry.0 += rx;
        entry.1 += tx;
    }
    buckets.into_iter().map(|(b, (rx, tx))| (b, rx, tx)).collect()
}

pub fn aggregate_router_rows_to_local_buckets(
    rows: &[(i64, NaiveDateTime, i64, i64)],
    interval: i64,
    tz: Tz,
) -> Vec<(i64, NaiveDateTime, i64, i64)> {
    let mut buckets: BTreeMap<(i64, NaiveDateTime), (i64, i64)> = BTreeMap::new();
    for &(router_id, ts, rx, tx) in rows {
        let b = local_bucket_start_utc_naive(ts, interval, tz);
        let entry = buckets.entry((router_id, b)).or_insert((0, 0));
        entry.0 += rx;
        entry.1 += tx;
    }
    buckets.into_iter().map(|((rid, b), (rx, tx))| (rid, b, rx, tx)).collect()
}
