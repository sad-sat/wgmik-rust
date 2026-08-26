use chrono::{DateTime, Timelike, Utc};
use rusqlite::{params, Connection, Result};

pub fn floor_to_minute_utc(dt: DateTime<Utc>) -> DateTime<Utc> {
    dt.with_second(0).unwrap().with_nanosecond(0).unwrap()
}

pub fn upsert_usage_minute(
    conn: &Connection,
    peer_id: i64,
    minute_ts: DateTime<Utc>,
    rx: i64,
    tx: i64,
) -> Result<()> {
    if rx == 0 && tx == 0 {
        return Ok(());
    }
    let ts_str = minute_ts.format("%Y-%m-%d %H:%M:%S").to_string();
    conn.execute(
        r#"
        INSERT INTO usage_minute (peer_id, minute_ts, rx, tx)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(peer_id, minute_ts) DO UPDATE SET
            rx = usage_minute.rx + excluded.rx,
            tx = usage_minute.tx + excluded.tx
        "#,
        params![peer_id, ts_str, rx, tx],
    )?;
    Ok(())
}
