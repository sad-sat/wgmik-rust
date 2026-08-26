use chrono::{DateTime, Utc};
use chrono_tz::Tz;

pub const NEAR_32BIT_COUNTER_BYTES: i64 = (3.5 * 1024.0 * 1024.0 * 1024.0) as i64;
pub const LOW_COUNTER_RESET_BYTES: i64 = 768 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CounterDelta {
    pub delta: i64,
    pub dropped: bool,
    pub near_32bit_drop: bool,
}

pub fn counter_delta(previous: i64, current: i64) -> CounterDelta {
    let dropped = current < previous;
    let near_32bit_drop = dropped
        && previous >= NEAR_32BIT_COUNTER_BYTES
        && current <= LOW_COUNTER_RESET_BYTES;

    CounterDelta {
        delta: if dropped { 0 } else { current - previous },
        dropped,
        near_32bit_drop,
    }
}

pub fn counter_day_key(ts_utc: DateTime<Utc>, tz: Tz) -> String {
    ts_utc.with_timezone(&tz).format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counter_deltas() {
        let normal = counter_delta(1000, 1500);
        assert_eq!(normal.delta, 500);
        assert!(!normal.dropped);
        assert!(!normal.near_32bit_drop);

        let drop_low = counter_delta(1500, 1000);
        assert_eq!(drop_low.delta, 0);
        assert!(drop_low.dropped);
        assert!(!drop_low.near_32bit_drop);

        let roll_32bit = counter_delta(4_000_000_000, 100_000);
        assert_eq!(roll_32bit.delta, 0);
        assert!(roll_32bit.dropped);
        assert!(roll_32bit.near_32bit_drop);
    }
}
