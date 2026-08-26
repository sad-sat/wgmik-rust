pub use chrono_tz::Tz;
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use std::str::FromStr;

pub const DATE_CALENDAR_GREGORIAN: &str = "gregorian";
pub const DATE_CALENDAR_PERSIAN: &str = "persian";

pub const PERSIAN_MONTH_NAMES: [&str; 12] = [
    "Farvardin",
    "Ordibehesht",
    "Khordad",
    "Tir",
    "Mordad",
    "Shahrivar",
    "Mehr",
    "Aban",
    "Azar",
    "Dey",
    "Bahman",
    "Esfand",
];

pub fn normalize_date_calendar(value: Option<&str>) -> String {
    let v = value.unwrap_or(DATE_CALENDAR_GREGORIAN).trim().to_lowercase();
    if v == DATE_CALENDAR_PERSIAN {
        DATE_CALENDAR_PERSIAN.to_string()
    } else {
        DATE_CALENDAR_GREGORIAN.to_string()
    }
}

pub fn app_date_calendar() -> String {
    DATE_CALENDAR_GREGORIAN.to_string()
}

pub fn gregorian_to_jalali(mut gy: i32, mut gm: i32, mut gd: i32) -> (i32, i32, i32) {
    let g_days_in_month = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let j_days_in_month = [31, 31, 31, 31, 31, 31, 30, 30, 30, 30, 30, 29];

    gy -= 1600;
    gm -= 1;
    gd -= 1;

    let mut g_day_no = 365 * gy + (gy + 3) / 4 - (gy + 99) / 100 + (gy + 399) / 400;
    for i in 0..gm as usize {
        g_day_no += g_days_in_month[i];
    }
    if gm > 1 && ((gy + 1600) % 4 == 0 && ((gy + 1600) % 100 != 0 || (gy + 1600) % 400 == 0)) {
        g_day_no += 1;
    }
    g_day_no += gd;

    let mut j_day_no = g_day_no - 79;
    let j_np = j_day_no / 12053;
    j_day_no %= 12053;

    let mut jy = 979 + 33 * j_np + 4 * (j_day_no / 1461);
    j_day_no %= 1461;
    if j_day_no >= 366 {
        jy += (j_day_no - 1) / 365;
        j_day_no = (j_day_no - 1) % 365;
    }

    let mut jm = 0;
    while jm < 11 && j_day_no >= j_days_in_month[jm as usize] {
        j_day_no -= j_days_in_month[jm as usize];
        jm += 1;
    }
    (jy, jm + 1, j_day_no + 1)
}

pub fn jalali_to_gregorian(mut jy: i32, mut jm: i32, mut jd: i32) -> (i32, i32, i32) {
    let g_days_in_month = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let j_days_in_month = [31, 31, 31, 31, 31, 31, 30, 30, 30, 30, 30, 29];

    jy -= 979;
    jm -= 1;
    jd -= 1;

    let mut j_day_no = 365 * jy + (jy / 33) * 8 + ((jy % 33) + 3) / 4;
    for i in 0..jm as usize {
        j_day_no += j_days_in_month[i];
    }
    j_day_no += jd;

    let mut g_day_no = j_day_no + 79;
    let mut gy = 1600 + 400 * (g_day_no / 146097);
    g_day_no %= 146097;

    let mut leap = true;
    if g_day_no >= 36525 {
        g_day_no -= 1;
        gy += 100 * (g_day_no / 36524);
        g_day_no %= 36524;
        if g_day_no >= 365 {
            g_day_no += 1;
        } else {
            leap = false;
        }
    }

    gy += 4 * (g_day_no / 1461);
    g_day_no %= 1461;

    if g_day_no >= 366 {
        leap = false;
        g_day_no -= 1;
        gy += g_day_no / 365;
        g_day_no %= 365;
    }

    let mut gm = 0;
    while gm < 11 {
        let dim = g_days_in_month[gm as usize] + if gm == 1 && leap { 1 } else { 0 };
        if g_day_no < dim {
            break;
        }
        g_day_no -= dim;
        gm += 1;
    }
    (gy, gm + 1, g_day_no + 1)
}

pub fn is_jalali_leap_year(jy: i32) -> bool {
    let (gy1, gm1, gd1) = jalali_to_gregorian(jy, 1, 1);
    let (gy2, gm2, gd2) = jalali_to_gregorian(jy + 1, 1, 1);
    let d1 = NaiveDate::from_ymd_opt(gy1, gm1 as u32, gd1 as u32).unwrap();
    let d2 = NaiveDate::from_ymd_opt(gy2, gm2 as u32, gd2 as u32).unwrap();
    (d2 - d1).num_days() == 366
}

pub fn days_in_gregorian_month(year: i32, month: i32) -> i32 {
    let d = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(year, month as u32 + 1, 1).unwrap()
    };
    let prev = NaiveDate::from_ymd_opt(year, month as u32, 1).unwrap();
    (d - prev).num_days() as i32
}

pub fn days_in_selected_month(year: i32, month: i32, calendar: &str) -> i32 {
    if calendar == DATE_CALENDAR_PERSIAN {
        if month <= 6 {
            31
        } else if month <= 11 {
            30
        } else if is_jalali_leap_year(year) {
            30
        } else {
            29
        }
    } else {
        days_in_gregorian_month(year, month)
    }
}

pub fn selected_calendar_month_start(local_date: NaiveDate, calendar: &str) -> NaiveDate {
    if calendar == DATE_CALENDAR_PERSIAN {
        let (jy, jm, _) = gregorian_to_jalali(local_date.year(), local_date.month() as i32, local_date.day() as i32);
        let (gy, gm, gd) = jalali_to_gregorian(jy, jm, 1);
        NaiveDate::from_ymd_opt(gy, gm as u32, gd as u32).unwrap()
    } else {
        NaiveDate::from_ymd_opt(local_date.year(), local_date.month(), 1).unwrap()
    }
}

pub fn add_selected_calendar_months(local_month_start: NaiveDate, delta: i32, calendar: &str) -> NaiveDate {
    if calendar == DATE_CALENDAR_PERSIAN {
        let (mut jy, mut jm, _) = gregorian_to_jalali(local_month_start.year(), local_month_start.month() as i32, local_month_start.day() as i32);
        jm += delta;
        jy += (jm - 1).div_euclid(12);
        jm = (jm - 1).rem_euclid(12) + 1;
        let (gy, gm, gd) = jalali_to_gregorian(jy, jm, 1);
        NaiveDate::from_ymd_opt(gy, gm as u32, gd as u32).unwrap()
    } else {
        let mut y = local_month_start.year();
        let mut m = local_month_start.month() as i32 + delta;
        y += (m - 1).div_euclid(12);
        m = (m - 1).rem_euclid(12) + 1;
        NaiveDate::from_ymd_opt(y, m as u32, 1).unwrap()
    }
}

pub fn selected_calendar_date_parts(local_date: NaiveDate, calendar: &str) -> (i32, i32, i32) {
    if calendar == DATE_CALENDAR_PERSIAN {
        gregorian_to_jalali(local_date.year(), local_date.month() as i32, local_date.day() as i32)
    } else {
        (local_date.year(), local_date.month() as i32, local_date.day() as i32)
    }
}

pub fn selected_calendar_to_gregorian_date(year: i32, month: i32, day: i32, calendar: &str) -> NaiveDate {
    let max_days = days_in_selected_month(year, month, calendar);
    let clamped_day = day.max(1).min(max_days);
    if calendar == DATE_CALENDAR_PERSIAN {
        let (gy, gm, gd) = jalali_to_gregorian(year, month, clamped_day);
        NaiveDate::from_ymd_opt(gy, gm as u32, gd as u32).unwrap()
    } else {
        NaiveDate::from_ymd_opt(year, month as u32, clamped_day as u32).unwrap()
    }
}

pub fn add_selected_calendar_months_to_parts(mut year: i32, mut month: i32, delta: i32) -> (i32, i32) {
    month += delta;
    year += (month - 1).div_euclid(12);
    month = (month - 1).rem_euclid(12) + 1;
    (year, month)
}

pub fn parse_timezone(tz_str: &str) -> Tz {
    Tz::from_str(tz_str).unwrap_or(Tz::UTC)
}

pub fn selected_month_cycle_bounds_utc(
    now_utc: DateTime<Utc>,
    tz: Tz,
    calendar: &str,
    reset_day: i32,
    count: i32,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let calendar = normalize_date_calendar(Some(calendar));
    let reset_day = reset_day.max(1).min(31);
    let count = count.max(1);

    let now_local = now_utc.with_timezone(&tz);
    let (sy, sm, _) = selected_calendar_date_parts(now_local.date_naive(), &calendar);

    let current_reset_date = selected_calendar_to_gregorian_date(sy, sm, reset_day, &calendar);
    let (start_y, start_m) = if now_local.date_naive() < current_reset_date {
        add_selected_calendar_months_to_parts(sy, sm, -1)
    } else {
        (sy, sm)
    };

    let (start_y, start_m) = add_selected_calendar_months_to_parts(start_y, start_m, -(count - 1));
    let (end_y, end_m) = add_selected_calendar_months_to_parts(start_y, start_m, count);

    let start_date = selected_calendar_to_gregorian_date(start_y, start_m, reset_day, &calendar);
    let end_date = selected_calendar_to_gregorian_date(end_y, end_m, reset_day, &calendar);

    let start_naive = NaiveDateTime::new(start_date, NaiveTime::MIN);
    let end_naive = NaiveDateTime::new(end_date, NaiveTime::MIN);

    let start_utc = tz.from_local_datetime(&start_naive).single().unwrap_or_else(|| tz.from_utc_datetime(&start_naive)).with_timezone(&Utc);
    let end_utc = tz.from_local_datetime(&end_naive).single().unwrap_or_else(|| tz.from_utc_datetime(&end_naive)).with_timezone(&Utc);

    (start_utc, end_utc)
}

pub fn selected_month_bounds_utc(
    now_utc: DateTime<Utc>,
    tz: Tz,
    calendar: &str,
    count: i32,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let calendar = normalize_date_calendar(Some(calendar));
    let count = count.max(1);
    let now_local = now_utc.with_timezone(&tz);
    let this_start = selected_calendar_month_start(now_local.date_naive(), &calendar);
    let start_date = add_selected_calendar_months(this_start, -(count - 1), &calendar);
    let end_date = add_selected_calendar_months(start_date, count, &calendar);

    let start_naive = NaiveDateTime::new(start_date, NaiveTime::MIN);
    let end_naive = NaiveDateTime::new(end_date, NaiveTime::MIN);

    let start_utc = tz.from_local_datetime(&start_naive).single().unwrap_or_else(|| tz.from_utc_datetime(&start_naive)).with_timezone(&Utc);
    let end_utc = tz.from_local_datetime(&end_naive).single().unwrap_or_else(|| tz.from_utc_datetime(&end_naive)).with_timezone(&Utc);

    (start_utc, end_utc)
}

pub fn selected_calendar_month_bounds_utc(
    calendar_year: i32,
    calendar_month: i32,
    tz: Tz,
    calendar: &str,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let calendar = normalize_date_calendar(Some(calendar));
    let start_date = selected_calendar_to_gregorian_date(calendar_year, calendar_month, 1, &calendar);
    let end_date = add_selected_calendar_months(start_date, 1, &calendar);

    let start_naive = NaiveDateTime::new(start_date, NaiveTime::MIN);
    let end_naive = NaiveDateTime::new(end_date, NaiveTime::MIN);

    let start_utc = tz.from_local_datetime(&start_naive).single().unwrap_or_else(|| tz.from_utc_datetime(&start_naive)).with_timezone(&Utc);
    let end_utc = tz.from_local_datetime(&end_naive).single().unwrap_or_else(|| tz.from_utc_datetime(&end_naive)).with_timezone(&Utc);

    (start_utc, end_utc)
}

pub fn utc_range_to_local_day_bounds(
    start_utc: Option<DateTime<Utc>>,
    end_utc: Option<DateTime<Utc>>,
    tz: Tz,
) -> (Option<String>, Option<String>) {
    let start_day = start_utc.map(|dt| dt.with_timezone(&tz).format("%Y-%m-%d").to_string());
    let end_day = end_utc.map(|dt| {
        let adjusted = dt - chrono::Duration::microseconds(1);
        adjusted.with_timezone(&tz).format("%Y-%m-%d").to_string()
    });
    (start_day, end_day)
}

pub fn format_app_datetime(dt_utc: DateTime<Utc>, include_time: bool, calendar: &str, tz: Tz) -> String {
    let local = dt_utc.with_timezone(&tz);
    let calendar = normalize_date_calendar(Some(calendar));
    let base = if calendar == DATE_CALENDAR_PERSIAN {
        let (jy, jm, jd) = gregorian_to_jalali(local.year(), local.month() as i32, local.day() as i32);
        let month_name = PERSIAN_MONTH_NAMES.get((jm - 1) as usize).unwrap_or(&"");
        format!("{} {}, {}", month_name, jd, jy)
    } else {
        local.format("%Y-%m-%d").to_string()
    };

    if include_time {
        format!("{} {}", base, local.format("%H:%M"))
    } else {
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calendar_conversions() {
        let (jy, jm, jd) = gregorian_to_jalali(2026, 3, 21);
        assert_eq!((jy, jm, jd), (1405, 1, 1));

        let (gy, gm, gd) = jalali_to_gregorian(1405, 1, 1);
        assert_eq!((gy, gm, gd), (2026, 3, 21));

        let (jy2, jm2, jd2) = gregorian_to_jalali(2024, 1, 1);
        let (gy2, gm2, gd2) = jalali_to_gregorian(jy2, jm2, jd2);
        assert_eq!((gy2, gm2, gd2), (2024, 1, 1));
    }

    #[test]
    fn test_jalali_leap_year() {
        assert!(is_jalali_leap_year(1403));
        assert!(!is_jalali_leap_year(1404));
    }
}
