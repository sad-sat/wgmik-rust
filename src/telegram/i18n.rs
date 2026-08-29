use std::collections::HashMap;
use std::sync::OnceLock;

static STRINGS: OnceLock<HashMap<&'static str, (&'static str, &'static str)>> = OnceLock::new();

fn init_strings() -> HashMap<&'static str, (&'static str, &'static str)> {
    let mut m = HashMap::new();
    m.insert("welcome", ("👋 Welcome.\nChoose what you want to check.", "به ربات WGMik خوش آمدید!\nاز منوی زیر برای مشاهده پیرها، مصرف و وضعیت مصرف منصفانه استفاده کنید."));
    m.insert("welcome_signup", ("You're all set. Your account is linked to {count} connection(s).", "خوش آمدید! حساب شما به {count} پیر متصل شد."));
    m.insert("token_invalid", ("This signup link is invalid or has expired.", "این لینک ثبت‌نام نامعتبر است یا منقضی شده."));
    m.insert("token_used", ("This signup link has already been used.", "این لینک ثبت‌نام قبلاً استفاده شده."));
    m.insert("blocked", ("This account is blocked. Please contact your admin if this is unexpected.", "حساب شما مسدود شده. با مدیر تماس بگیرید."));
    m.insert("no_peers", ("No connections are linked to your account yet.", "هیچ پیری به حساب شما متصل نیست."));
    m.insert("btn_my_peers", ("My Connections", "پیرهای من"));
    m.insert("btn_my_connections", ("My Connections", "پیرهای من"));
    m.insert("btn_usage", ("Usage", "مصرف"));
    m.insert("btn_usage_history", ("Usage History", "سابقه مصرف"));
    m.insert("btn_status", ("Status", "وضعیت"));
    m.insert("btn_fair_usage", ("Fair Usage", "مصرف منصفانه"));
    m.insert("btn_limits", ("Limits", "محدودیت‌ها"));
    m.insert("btn_language", ("Language", "زبان"));
    m.insert("btn_notifications", ("Notifications", "اعلان‌ها"));
    m.insert("btn_settings", ("Settings", "تنظیمات"));
    m.insert("btn_back", ("Back", "« بازگشت"));
    m.insert("btn_home", ("Home", "خانه"));
    m.insert("btn_today", ("Today", "امروز"));
    m.insert("btn_monthly", ("This Month", "این ماه"));
    m.insert("btn_alltime", ("All Time", "کل دوره"));
    m.insert("btn_calendar", ("Calendar", "تقویم"));
    m.insert("today_title", ("Today's Usage", "مصرف امروز"));
    m.insert("monthly_title", ("Monthly Usage", "مصرف ماهانه"));
    m.insert("alltime_title", ("All-time Usage", "مصرف کل"));
    m.insert("usage_rx", ("Download: {rx}", "دانلود: {rx}"));
    m.insert("usage_tx", ("Upload: {tx}", "آپلود: {tx}"));
    m.insert("usage_total", ("Total: {total}", "مجموع: {total}"));
    m.insert("status_online", ("Online", "آنلاین"));
    m.insert("status_offline", ("Offline", "آفلاین"));
    m.insert("status_throttled", ("Throttled", "محدود شده"));
    m.insert("status_normal", ("Normal", "عادی"));
    m.insert("lang_changed", ("Language changed to English.", "زبان به فارسی تغییر کرد."));
    m.insert("notif_enabled", ("Notifications enabled.", "اعلان‌ها فعال شدند."));
    m.insert("notif_disabled", ("Notifications disabled.", "اعلان‌ها غیرفعال شدند."));
    m.insert("admin_panel", ("Admin Dashboard", "پنل مدیریت"));
    m.insert("admin_unauthorized", ("Unauthorized. You are not configured as admin.", "دسترسی غیرمجاز. شما به عنوان مدیر تنظیم نشده‌اید."));
    m.insert("please_wait", ("⏳ Please wait, your previous request is still being processed...", "⏳ لطفاً کمی صبر کنید، درخواست قبلی شما هنوز در حال پردازش است..."));
    m.insert("please_wait_short", ("⏳ Please wait...", "⏳ لطفاً کمی صبر کنید..."));
    m
}

pub fn t(key: &str, lang: &str) -> String {
    let map = STRINGS.get_or_init(init_strings);
    if let Some(&(en, fa)) = map.get(key) {
        if lang.starts_with("fa") {
            fa.to_string()
        } else {
            en.to_string()
        }
    } else {
        key.to_string()
    }
}
