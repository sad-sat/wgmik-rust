use super::fair_usage_card::generate_fair_usage_card_svg;
use super::i18n::t;
use super::svg_render::{fmt_bytes, render_svg_to_png_async};
use super::usage_chart::generate_usage_chart_svg;
use crate::calendar::parse_timezone;
use crate::db::models::Peer;
use crate::db::DbPool;
use crate::fair_usage::build_fair_usage_peer_status_dto;
use chrono::{Datelike, Timelike, Utc};
use reqwest::multipart::{Form, Part};
use reqwest::Client;
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::{error, info, warn};

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}

#[derive(Clone, Debug)]
struct InFlightEntry {
    started_at: Instant,
    last_notified_at: Option<Instant>,
}

pub struct InFlightGuard {
    chat_id: i64,
    tracker: Arc<Mutex<HashMap<i64, InFlightEntry>>>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = self.tracker.lock() {
            map.remove(&self.chat_id);
        }
    }
}

pub enum InFlightStatus {
    Acquired(InFlightGuard),
    AlreadyRunning { should_notify: bool },
}

#[derive(Clone, Debug)]
struct CachedUser {
    language: String,
    is_blocked: bool,
    username: String,
    first_name: String,
    last_name: String,
    last_synced: Instant,
}

#[derive(Clone, Debug)]
struct CachedAdmin {
    admin_id: Option<i64>,
    admin_username: Option<String>,
    last_checked: Instant,
}

#[derive(Clone)]
pub struct TelegramBot {
    client: Client,
    pub token: String,
    pool: DbPool,
    running: Arc<AtomicBool>,
    started_at: chrono::DateTime<Utc>,
    in_flight_requests: Arc<Mutex<HashMap<i64, InFlightEntry>>>,
    user_cache: Arc<Mutex<HashMap<i64, CachedUser>>>,
    admin_cache: Arc<Mutex<Option<CachedAdmin>>>,
    heavy_ops_semaphore: Arc<tokio::sync::Semaphore>,
}

impl TelegramBot {
    pub fn new(token: String, pool: DbPool, _secret_key: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(35))
                .pool_idle_timeout(Duration::from_secs(90))
                .pool_max_idle_per_host(10)
                .tcp_nodelay(true)
                .build()
                .unwrap(),
            token,
            pool,
            running: Arc::new(AtomicBool::new(false)),
            started_at: Utc::now(),
            in_flight_requests: Arc::new(Mutex::new(HashMap::new())),
            user_cache: Arc::new(Mutex::new(HashMap::new())),
            admin_cache: Arc::new(Mutex::new(None)),
            heavy_ops_semaphore: Arc::new(tokio::sync::Semaphore::new(6)),
        }
    }

    pub fn try_acquire_in_flight(&self, chat_id: i64) -> InFlightStatus {
        let mut map = match self.in_flight_requests.lock() {
            Ok(m) => m,
            Err(poisoned) => poisoned.into_inner(),
        };

        // Prune stale entries older than 45 seconds to prevent memory leaks or deadlocks
        map.retain(|_, entry| entry.started_at.elapsed() < Duration::from_secs(45));

        let now = Instant::now();
        if let Some(entry) = map.get_mut(&chat_id) {
            if entry.started_at.elapsed() < Duration::from_secs(40) {
                let should_notify = match entry.last_notified_at {
                    None => true,
                    Some(last) => now.duration_since(last) >= Duration::from_millis(1500),
                };
                if should_notify {
                    entry.last_notified_at = Some(now);
                }
                return InFlightStatus::AlreadyRunning { should_notify };
            }
        }

        map.insert(
            chat_id,
            InFlightEntry {
                started_at: now,
                last_notified_at: None,
            },
        );

        InFlightStatus::Acquired(InFlightGuard {
            chat_id,
            tracker: self.in_flight_requests.clone(),
        })
    }

    pub fn uptime_seconds(&self) -> i64 {
        (Utc::now() - self.started_at).num_seconds().max(0)
    }

    pub fn started_at_str(&self) -> String {
        self.started_at.to_rfc3339()
    }

    pub async fn get_me(&self) -> Result<String, String> {
        let resp = self.client.get(&self.api_url("getMe"))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let val = resp.json::<Value>().await.map_err(|e| e.to_string())?;
        if val.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            let username = val.get("result")
                .and_then(|r| r.get("username"))
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string();
            Ok(username)
        } else {
            let desc = val.get("description").and_then(|d| d.as_str()).unwrap_or("Telegram API error");
            Err(desc.to_string())
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.token, method)
    }

    pub async fn send_message(&self, chat_id: i64, text: &str, reply_markup: Option<Value>) -> Result<Value, String> {
        let mut body = json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "HTML",
        });
        if let Some(rm) = reply_markup.clone() {
            body["reply_markup"] = rm;
        }

        let resp = self.client.post(&self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let val = resp.json::<Value>().await.map_err(|e| e.to_string())?;
        if !val.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            let desc = val.get("description").and_then(|d| d.as_str()).unwrap_or("Telegram API error");
            warn!("Telegram sendMessage HTML parse failed: {}. Retrying without HTML formatting...", desc);

            let mut fallback_body = json!({
                "chat_id": chat_id,
                "text": text,
            });
            if let Some(rm) = reply_markup {
                fallback_body["reply_markup"] = rm;
            }
            let resp2 = self.client.post(&self.api_url("sendMessage"))
                .json(&fallback_body)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let val2 = resp2.json::<Value>().await.map_err(|e| e.to_string())?;
            if !val2.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                let desc2 = val2.get("description").and_then(|d| d.as_str()).unwrap_or("Telegram API error");
                error!("Telegram sendMessage failed: {}", desc2);
                return Err(desc2.to_string());
            }
            return Ok(val2);
        }

        Ok(val)
    }

    pub async fn send_photo(&self, chat_id: i64, photo_bytes: Vec<u8>, caption: &str, reply_markup: Option<Value>) -> Result<Value, String> {
        let part = Part::bytes(photo_bytes)
            .file_name("chart.png")
            .mime_str("image/png")
            .map_err(|e| e.to_string())?;

        let mut form = Form::new()
            .text("chat_id", chat_id.to_string())
            .text("caption", caption.to_string())
            .text("parse_mode", "HTML")
            .part("photo", part);

        if let Some(rm) = reply_markup.clone() {
            form = form.text("reply_markup", rm.to_string());
        }

        let resp = self.client.post(&self.api_url("sendPhoto"))
            .multipart(form)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let val = resp.json::<Value>().await.map_err(|e| e.to_string())?;
        if !val.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            let desc = val.get("description").and_then(|d| d.as_str()).unwrap_or("Telegram sendPhoto error");
            warn!("Telegram sendPhoto failed: {}. Falling back to text sendMessage.", desc);
            return self.send_message(chat_id, caption, reply_markup).await;
        }

        Ok(val)
    }

    pub async fn answer_callback_query(&self, callback_query_id: &str, text: Option<&str>) -> Result<(), String> {
        let mut body = json!({ "callback_query_id": callback_query_id });
        if let Some(t) = text {
            body["text"] = json!(t);
        }
        let _ = self.client.post(&self.api_url("answerCallbackQuery")).json(&body).send().await;
        Ok(())
    }

    pub async fn sync_bot_commands(&self) {
        let cmds = json!({
            "commands": [
                { "command": "start", "description": "Start the bot / Home" },
                { "command": "home", "description": "Main menu" },
                { "command": "today", "description": "Today's bandwidth usage" },
                { "command": "monthly", "description": "Monthly usage" },
                { "command": "alltime", "description": "All-time usage" },
                { "command": "fair", "description": "Fair usage policy & status" },
                { "command": "settings", "description": "Language & preferences" }
            ]
        });
        let _ = self.client.post(&self.api_url("setMyCommands")).json(&cmds).send().await;

        let pool = self.pool.clone();
        let admin_id_opt = tokio::task::spawn_blocking(move || {
            let conn = pool.get().ok()?;
            let raw: String = conn.query_row(
                "SELECT value FROM settings_kv WHERE key = 'tg_admin_chat_id'",
                [],
                |r| r.get(0),
            ).unwrap_or_default();
            let clean = raw.trim().trim_start_matches('@');
            clean.parse::<i64>().ok()
        }).await.unwrap_or(None);

        if let Some(admin_id) = admin_id_opt {
            let admin_cmds = json!({
                "commands": [
                    { "command": "start", "description": "Start the bot / Home" },
                    { "command": "home", "description": "Main menu" },
                    { "command": "today", "description": "Today's bandwidth usage" },
                    { "command": "monthly", "description": "Monthly usage" },
                    { "command": "alltime", "description": "All-time usage" },
                    { "command": "fair", "description": "Fair usage policy & status" },
                    { "command": "settings", "description": "Language & preferences" },
                    { "command": "admin", "description": "🛡️ Admin dashboard" }
                ],
                "scope": {
                    "type": "chat",
                    "chat_id": admin_id
                }
            });
            let _ = self.client.post(&self.api_url("setMyCommands")).json(&admin_cmds).send().await;
        }
    }

    pub fn is_admin(&self, tg_user_id: i64, username: Option<&str>) -> bool {
        let now = Instant::now();
        let cached = {
            let cache = match self.admin_cache.lock() {
                Ok(c) => c,
                Err(p) => p.into_inner(),
            };
            if let Some(ref ca) = *cache {
                if ca.last_checked.elapsed() < Duration::from_secs(30) {
                    Some(ca.clone())
                } else {
                    None
                }
            } else {
                None
            }
        };

        let admin_data = match cached {
            Some(ca) => ca,
            None => {
                let conn = match self.pool.get() {
                    Ok(c) => c,
                    Err(_) => return false,
                };
                let raw: String = conn.query_row(
                    "SELECT value FROM settings_kv WHERE key = 'tg_admin_chat_id'",
                    [],
                    |r| r.get(0),
                ).unwrap_or_default();
                let clean = raw.trim().trim_start_matches('@').trim_matches('"').trim_matches('\'').to_string();
                let aid = clean.parse::<i64>().ok();
                let aname = if !clean.is_empty() && aid.is_none() { Some(clean.clone()) } else { None };

                let ca = CachedAdmin {
                    admin_id: aid,
                    admin_username: aname,
                    last_checked: now,
                };

                if let Ok(mut lock) = self.admin_cache.lock() {
                    *lock = Some(ca.clone());
                }
                ca
            }
        };

        if let Some(aid) = admin_data.admin_id {
            if aid == tg_user_id {
                return true;
            }
        }

        if let Some(ref aname) = admin_data.admin_username {
            if let Some(uname) = username {
                if !uname.is_empty() && uname.trim_start_matches('@').eq_ignore_ascii_case(aname) {
                    return true;
                }
            }
        }

        false
    }

    pub async fn start_polling(self: Arc<Self>) {
        self.running.store(true, Ordering::SeqCst);
        info!("Telegram bot polling started");

        // Clear any existing webhook so getUpdates polling works cleanly
        let _ = self.client.post(&self.api_url("deleteWebhook"))
            .json(&json!({ "drop_pending_updates": false }))
            .send()
            .await;

        self.sync_bot_commands().await;

        let mut offset: i64 = 0;
        while self.running.load(Ordering::SeqCst) {
            let url = format!("{}?offset={}&timeout=20", self.api_url("getUpdates"), offset);
            match self.client.get(&url).send().await {
                Ok(resp) => {
                    if let Ok(data) = resp.json::<Value>().await {
                        if data.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                            if let Some(results) = data.get("result").and_then(|v| v.as_array()) {
                                for update in results {
                                    if let Some(update_id) = update.get("update_id").and_then(|v| v.as_i64()) {
                                        offset = update_id + 1;
                                    }
                                    let bot = self.clone();
                                    let update_clone = update.clone();
                                    tokio::spawn(async move {
                                        bot.handle_update(update_clone).await;
                                    });
                                }
                            }
                        } else {
                            let desc = data.get("description").and_then(|d| d.as_str()).unwrap_or("Unknown Telegram error");
                            warn!("Telegram getUpdates rejected: {}", desc);
                            if desc.contains("webhook is active") {
                                let _ = self.client.post(&self.api_url("deleteWebhook"))
                                    .json(&json!({ "drop_pending_updates": false }))
                                    .send()
                                    .await;
                            }
                            sleep(Duration::from_secs(3)).await;
                        }
                    }
                }
                Err(e) => {
                    warn!("Telegram polling network error: {}", e);
                    sleep(Duration::from_secs(3)).await;
                }
            }
        }
        info!("Telegram bot polling stopped");
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    async fn handle_update(&self, update: Value) {
        if let Some(message) = update.get("message") {
            self.handle_message(message).await;
        } else if let Some(cb) = update.get("callback_query") {
            self.handle_callback_query(cb).await;
        }
    }

    pub async fn get_or_sync_user(&self, tg_user_id: i64, username: &str, first_name: &str, last_name: &str) -> (String, bool) {
        // Fast in-memory lookup to prevent SQLite write lock contention
        {
            let cache = match self.user_cache.lock() {
                Ok(c) => c,
                Err(p) => p.into_inner(),
            };
            if let Some(u) = cache.get(&tg_user_id) {
                if u.last_synced.elapsed() < Duration::from_secs(300)
                    && u.username == username
                    && u.first_name == first_name
                    && u.last_name == last_name
                {
                    return (u.language.clone(), u.is_blocked);
                }
            }
        }

        let pool = self.pool.clone();
        let uname = username.to_string();
        let fname = first_name.to_string();
        let lname = last_name.to_string();

        let res = tokio::task::spawn_blocking(move || {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(_) => return ("en".to_string(), false),
            };
            let existing: rusqlite::Result<(String, bool)> = conn.query_row(
                "SELECT language, is_blocked FROM telegram_users WHERE telegram_user_id = ?1",
                params![tg_user_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
            );

            if let Ok((lang, is_blocked)) = existing {
                let _ = conn.execute(
                    "UPDATE telegram_users SET telegram_username = ?1, first_name = ?2, last_name = ?3 WHERE telegram_user_id = ?4",
                    params![uname, fname, lname, tg_user_id],
                );
                (lang, is_blocked)
            } else {
                let _ = conn.execute(
                    "INSERT INTO telegram_users (telegram_user_id, telegram_username, first_name, last_name, language, is_blocked)
                     VALUES (?1, ?2, ?3, ?4, 'en', 0)",
                    params![tg_user_id, uname, fname, lname],
                );
                ("en".to_string(), false)
            }
        }).await.unwrap_or(("en".to_string(), false));

        // Update in-memory user cache
        {
            let mut cache = match self.user_cache.lock() {
                Ok(c) => c,
                Err(p) => p.into_inner(),
            };
            cache.insert(tg_user_id, CachedUser {
                language: res.0.clone(),
                is_blocked: res.1,
                username: username.to_string(),
                first_name: first_name.to_string(),
                last_name: last_name.to_string(),
                last_synced: Instant::now(),
            });
        }

        res
    }

    pub async fn set_user_language(&self, tg_user_id: i64, lang: &str) {
        // Fast in-memory cache update
        {
            let mut cache = match self.user_cache.lock() {
                Ok(c) => c,
                Err(p) => p.into_inner(),
            };
            if let Some(u) = cache.get_mut(&tg_user_id) {
                u.language = lang.to_string();
                u.last_synced = Instant::now();
            }
        }

        let pool = self.pool.clone();
        let lang_str = lang.to_string();
        tokio::task::spawn_blocking(move || {
            if let Ok(conn) = pool.get() {
                let _ = conn.execute(
                    "UPDATE telegram_users SET language = ?1 WHERE telegram_user_id = ?2",
                    params![lang_str, tg_user_id],
                );
            }
        }).await.ok();
    }

    async fn handle_message(&self, message: &Value) {
        let chat_id = match message.get("chat").and_then(|c| c.get("id")).and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return,
        };
        let text = message.get("text").and_then(|t| t.as_str()).unwrap_or("").trim();
        let from = message.get("from");
        let tg_user_id = from.and_then(|f| f.get("id")).and_then(|v| v.as_i64()).unwrap_or(chat_id);
        let username = from.and_then(|f| f.get("username")).and_then(|v| v.as_str()).unwrap_or("");
        let first_name = from.and_then(|f| f.get("first_name")).and_then(|v| v.as_str()).unwrap_or("");
        let last_name = from.and_then(|f| f.get("last_name")).and_then(|v| v.as_str()).unwrap_or("");

        let (lang, is_blocked) = self.get_or_sync_user(tg_user_id, username, first_name, last_name).await;
        if is_blocked {
            let _ = self.send_message(chat_id, &t("blocked", &lang), None).await;
            return;
        }

        let _guard = match self.try_acquire_in_flight(chat_id) {
            InFlightStatus::Acquired(guard) => guard,
            InFlightStatus::AlreadyRunning { should_notify } => {
                if should_notify {
                    let _ = self.send_message(chat_id, &t("please_wait", &lang), None).await;
                }
                info!("Telegram message from chat {} ignored (operation in progress)", chat_id);
                return;
            }
        };

        let is_heavy = text == "/today" || text == "/monthly" || text == "/alltime" || text == "/fair";
        let _heavy_permit = if is_heavy {
            match self.heavy_ops_semaphore.clone().try_acquire_owned() {
                Ok(permit) => Some(permit),
                Err(_) => {
                    info!("Server worker capacity busy, dropping heavy message request from chat {}", chat_id);
                    let _ = self.send_message(chat_id, &t("please_wait", &lang), None).await;
                    return;
                }
            }
        } else {
            None
        };

        if text.starts_with("/start") {
            let parts: Vec<&str> = text.split_whitespace().collect();
            if parts.len() > 1 {
                self.handle_signup_token(chat_id, tg_user_id, parts[1], &lang).await;
            } else {
                self.send_home_menu(chat_id, tg_user_id, &lang).await;
            }
        } else if text == "/home" {
            self.send_home_menu(chat_id, tg_user_id, &lang).await;
        } else if text == "/today" {
            self.send_today_usage(chat_id, tg_user_id, &lang).await;
        } else if text == "/monthly" {
            self.send_monthly_usage(chat_id, tg_user_id, &lang).await;
        } else if text == "/alltime" {
            self.send_alltime_usage(chat_id, tg_user_id, &lang).await;
        } else if text == "/fair" {
            self.send_fair_usage(chat_id, tg_user_id, &lang).await;
        } else if text == "/settings" {
            self.send_settings_menu(chat_id, &lang).await;
        } else if text == "/admin" || text.starts_with("/admin") {
            self.send_admin_menu(chat_id, tg_user_id, &lang).await;
        } else {
            self.send_home_menu(chat_id, tg_user_id, &lang).await;
        }
    }

    async fn handle_callback_query(&self, cb: &Value) {
        let id = cb.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let data = cb.get("data").and_then(|v| v.as_str()).unwrap_or("");
        let message = cb.get("message");
        let from = cb.get("from");
        let tg_user_id = from.and_then(|f| f.get("id")).and_then(|v| v.as_i64()).unwrap_or(0);
        let username = from.and_then(|f| f.get("username")).and_then(|v| v.as_str()).unwrap_or("");
        let first_name = from.and_then(|f| f.get("first_name")).and_then(|v| v.as_str()).unwrap_or("");
        let last_name = from.and_then(|f| f.get("last_name")).and_then(|v| v.as_str()).unwrap_or("");

        let raw_chat_id = message.and_then(|m| m.get("chat")).and_then(|c| c.get("id")).and_then(|v| v.as_i64()).unwrap_or(0);
        let chat_id = if raw_chat_id != 0 { raw_chat_id } else { tg_user_id };

        if chat_id == 0 {
            warn!("Telegram callback_query has no valid chat_id: data={}", data);
            return;
        }

        let (lang, is_blocked) = self.get_or_sync_user(tg_user_id, username, first_name, last_name).await;
        if is_blocked {
            let _ = self.answer_callback_query(id, Some(&t("blocked", &lang))).await;
            return;
        }

        let _guard = match self.try_acquire_in_flight(chat_id) {
            InFlightStatus::Acquired(guard) => guard,
            InFlightStatus::AlreadyRunning { .. } => {
                let _ = self.answer_callback_query(id, Some(&t("please_wait_short", &lang))).await;
                info!("Telegram callback from chat {} (data={}) ignored (operation in progress)", chat_id, data);
                return;
            }
        };

        let is_heavy = data == "menu:today" || data == "menu:monthly" || data == "menu:alltime" || data == "menu:fair" || data.starts_with("adm:peers:");
        let _heavy_permit = if is_heavy {
            match self.heavy_ops_semaphore.clone().try_acquire_owned() {
                Ok(permit) => Some(permit),
                Err(_) => {
                    info!("Server worker capacity busy, dropping heavy callback request from chat {} (data={})", chat_id, data);
                    let _ = self.answer_callback_query(id, Some(&t("please_wait_short", &lang))).await;
                    let _ = self.send_message(chat_id, &t("please_wait", &lang), None).await;
                    return;
                }
            }
        } else {
            None
        };

        let _ = self.answer_callback_query(id, None).await;

        info!("Telegram callback: user={}, chat={}, data={}", tg_user_id, chat_id, data);

        if data == "menu:home" {
            self.send_home_menu(chat_id, tg_user_id, &lang).await;
        } else if data == "menu:today" {
            self.send_today_usage(chat_id, tg_user_id, &lang).await;
        } else if data == "menu:monthly" {
            self.send_monthly_usage(chat_id, tg_user_id, &lang).await;
        } else if data == "menu:alltime" {
            self.send_alltime_usage(chat_id, tg_user_id, &lang).await;
        } else if data == "menu:fair" {
            self.send_fair_usage(chat_id, tg_user_id, &lang).await;
        } else if data == "menu:settings" {
            self.send_settings_menu(chat_id, &lang).await;
        } else if data == "menu:admin" || data == "adm:menu" {
            self.send_admin_menu(chat_id, tg_user_id, &lang).await;
        } else if data.starts_with("adm:users:") {
            let page: i64 = data.strip_prefix("adm:users:").unwrap_or("0").parse().unwrap_or(0);
            self.send_admin_users(chat_id, tg_user_id, page, &lang).await;
        } else if data.starts_with("adm:user:") {
            let uid: i64 = data.strip_prefix("adm:user:").unwrap_or("0").parse().unwrap_or(0);
            self.send_admin_user_detail(chat_id, tg_user_id, uid, &lang).await;
        } else if data.starts_with("adm:toggle_block:") {
            let uid: i64 = data.strip_prefix("adm:toggle_block:").unwrap_or("0").parse().unwrap_or(0);
            self.toggle_user_block(chat_id, tg_user_id, uid, &lang).await;
        } else if data.starts_with("adm:peers:") {
            let scope = data.strip_prefix("adm:peers:").unwrap_or("alltime");
            self.send_admin_peers_usage(chat_id, tg_user_id, scope, &lang).await;
        } else if data.starts_with("adm:outbox:") {
            let page: i64 = data.strip_prefix("adm:outbox:").unwrap_or("0").parse().unwrap_or(0);
            self.send_admin_outbox(chat_id, tg_user_id, page, &lang).await;
        } else if data == "lang:en" {
            self.set_user_language(tg_user_id, "en").await;
            let _ = self.send_message(chat_id, &t("lang_changed", "en"), None).await;
            self.send_home_menu(chat_id, tg_user_id, "en").await;
        } else if data == "lang:fa" {
            self.set_user_language(tg_user_id, "fa").await;
            let _ = self.send_message(chat_id, &t("lang_changed", "fa"), None).await;
            self.send_home_menu(chat_id, tg_user_id, "fa").await;
        }
    }

    async fn send_home_menu(&self, chat_id: i64, tg_user_id: i64, lang: &str) {
        let is_adm = self.is_admin(tg_user_id, None);
        let text = t("welcome", lang);
        let mut keyboard_rows = vec![
            vec![
                json!({ "text": format!("📊 {}", t("btn_today", lang)), "callback_data": "menu:today" }),
                json!({ "text": format!("📅 {}", t("btn_monthly", lang)), "callback_data": "menu:monthly" }),
            ],
            vec![
                json!({ "text": format!("📈 {}", t("btn_alltime", lang)), "callback_data": "menu:alltime" }),
                json!({ "text": format!("⚖️ {}", t("btn_fair_usage", lang)), "callback_data": "menu:fair" }),
            ],
        ];

        if is_adm {
            keyboard_rows.push(vec![
                json!({ "text": "🛡️ Admin Panel", "callback_data": "adm:menu" }),
                json!({ "text": format!("⚙️ {}", t("btn_settings", lang)), "callback_data": "menu:settings" }),
            ]);
        } else {
            keyboard_rows.push(vec![
                json!({ "text": format!("⚙️ {}", t("btn_settings", lang)), "callback_data": "menu:settings" }),
            ]);
        }

        let keyboard = json!({ "inline_keyboard": keyboard_rows });
        let _ = self.send_message(chat_id, &text, Some(keyboard)).await;
    }

    async fn send_settings_menu(&self, chat_id: i64, lang: &str) {
        let text = format!("⚙️ <b>{}</b>", t("btn_settings", lang));
        let keyboard = json!({
            "inline_keyboard": [
                [
                    { "text": "🇺🇸 English", "callback_data": "lang:en" },
                    { "text": "🇮🇷 فارسی", "callback_data": "lang:fa" }
                ],
                [
                    { "text": t("btn_back", lang), "callback_data": "menu:home" }
                ]
            ]
        });
        let _ = self.send_message(chat_id, &text, Some(keyboard)).await;
    }

    async fn handle_signup_token(&self, chat_id: i64, tg_user_id: i64, token: &str, lang: &str) {
        enum SignupResult {
            Used,
            Success(usize),
            Invalid,
        }

        let pool = self.pool.clone();
        let token_str = token.to_string();

        let result = match tokio::task::spawn_blocking(move || {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(_) => return SignupResult::Invalid,
            };
            let token_row = conn.query_row(
                "SELECT id, peer_ids, used_at, single_use FROM telegram_signup_tokens WHERE token = ?1",
                params![token_str],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, bool>(3)?)),
            );

            match token_row {
                Ok((token_id, peer_ids_json, used_at, single_use)) => {
                    if used_at.is_some() && single_use {
                        SignupResult::Used
                    } else {
                        let db_user_id = conn.query_row(
                            "SELECT id FROM telegram_users WHERE telegram_user_id = ?1",
                            params![tg_user_id],
                            |row| row.get::<_, i64>(0),
                        ).unwrap_or(0);

                        let peer_ids: Vec<i64> = serde_json::from_str(&peer_ids_json).unwrap_or_default();
                        for pid in &peer_ids {
                            let _ = conn.execute(
                                "INSERT INTO telegram_peer_bindings (telegram_user_id, peer_id, visible)
                                 VALUES (?1, ?2, 1) ON CONFLICT(telegram_user_id, peer_id) DO UPDATE SET visible = 1",
                                params![db_user_id, pid],
                            );
                        }

                        let now_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
                        let _ = conn.execute(
                            "UPDATE telegram_signup_tokens SET used_by = ?1, used_at = ?2 WHERE id = ?3",
                            params![db_user_id, now_str, token_id],
                        );

                        SignupResult::Success(peer_ids.len())
                    }
                }
                Err(_) => SignupResult::Invalid,
            }
        }).await {
            Ok(r) => r,
            Err(_) => SignupResult::Invalid,
        };

        match result {
            SignupResult::Used => {
                let _ = self.send_message(chat_id, &t("token_used", lang), None).await;
            }
            SignupResult::Success(count) => {
                let welcome_msg = t("welcome_signup", lang).replace("{count}", &count.to_string());
                let _ = self.send_message(chat_id, &welcome_msg, None).await;
                self.send_home_menu(chat_id, tg_user_id, lang).await;
            }
            SignupResult::Invalid => {
                let _ = self.send_message(chat_id, &t("token_invalid", lang), None).await;
            }
        }
    }

    async fn send_today_usage(&self, chat_id: i64, tg_user_id: i64, lang: &str) {
        let pool = self.pool.clone();
        let title = t("today_title", lang);
        let is_adm = self.is_admin(tg_user_id, None);

        let items: Vec<(String, String, String)> = match tokio::task::spawn_blocking(move || {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(_) => return Vec::new(),
            };
            let peers = Self::get_user_peers_sync(&conn, tg_user_id);
            if peers.is_empty() {
                return Vec::new();
            }

            let now = Utc::now();
            let today_start = now.date_naive().format("%Y-%m-%d 00:00:00").to_string();
            let now_str = now.format("%Y-%m-%d %H:%M:%S").to_string();
            let current_hour = now.hour() as usize;

            peers.into_iter().map(|peer| {
                let mut hourly_rx = vec![0i64; current_hour + 1];
                let mut hourly_tx = vec![0i64; current_hour + 1];

                if let Ok(mut stmt) = conn.prepare(
                    "SELECT minute_ts, rx, tx FROM usage_minute
                     WHERE peer_id = ?1 AND minute_ts >= ?2 AND minute_ts <= ?3
                     ORDER BY minute_ts ASC"
                ) {
                    if let Ok(rows) = stmt.query_map(params![peer.id, today_start, now_str], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
                    }) {
                        for row in rows.flatten() {
                            if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&row.0, "%Y-%m-%d %H:%M:%S") {
                                let h = dt.hour() as usize;
                                if h <= current_hour {
                                    hourly_rx[h] += row.1;
                                    hourly_tx[h] += row.2;
                                }
                            }
                        }
                    }
                }

                let total_rx: i64 = hourly_rx.iter().sum();
                let total_tx: i64 = hourly_tx.iter().sum();

                let (final_rx, final_tx) = if total_rx == 0 && total_tx == 0 {
                    let today_date = now.date_naive().format("%Y-%m-%d").to_string();
                    let (drx, dtx) = conn.query_row(
                        "SELECT rx, tx FROM usage_daily WHERE peer_id = ?1 AND day = ?2",
                        params![peer.id, today_date],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                    ).unwrap_or((0, 0));
                    if drx > 0 || dtx > 0 {
                        hourly_rx[current_hour] = drx;
                        hourly_tx[current_hour] = dtx;
                    }
                    (drx, dtx)
                } else {
                    (total_rx, total_tx)
                };

                let points: Vec<(String, i64, i64)> = (0..=current_hour)
                    .map(|h| (format!("{:02}:00", h), hourly_rx[h], hourly_tx[h]))
                    .collect();

                let peer_name = if !peer.name.is_empty() { peer.name } else { peer.interface };
                let escaped_name = escape_html(&peer_name);
                let caption = format!("📊 <b>{}</b> - {}\n⬇️ {}\n⬆️ {}\n📈 <b>Total:</b> {}",
                    title, escaped_name, fmt_bytes(final_rx), fmt_bytes(final_tx), fmt_bytes(final_rx + final_tx));

                let svg = generate_usage_chart_svg(&title, &peer_name, final_rx, final_tx, &points);
                (peer_name, svg, caption)
            }).collect()
        }).await {
            Ok(res) => res,
            Err(_) => Vec::new(),
        };

        if items.is_empty() {
            if is_adm {
                let msg = "ℹ️ <b>No personal connections linked.</b>\nAs an administrator, you can view usage for all peers in the 🛡️ <b>Admin Panel</b>.";
                let kb = json!({
                    "inline_keyboard": [
                        [
                            { "text": "📊 All Peers Today", "callback_data": "adm:peers:today" },
                            { "text": "🛡️ Admin Panel", "callback_data": "adm:menu" }
                        ],
                        [
                            { "text": "🏠 Home", "callback_data": "menu:home" }
                        ]
                    ]
                });
                let _ = self.send_message(chat_id, msg, Some(kb)).await;
            } else {
                let _ = self.send_message(chat_id, &t("no_peers", lang), None).await;
            }
            return;
        }

        for (_peer_name, svg, caption) in items {
            if let Ok(png) = render_svg_to_png_async(svg, 1.5).await {
                let _ = self.send_photo(chat_id, png, &caption, None).await;
            } else {
                let _ = self.send_message(chat_id, &caption, None).await;
            }
        }
    }

    async fn send_monthly_usage(&self, chat_id: i64, tg_user_id: i64, lang: &str) {
        let pool = self.pool.clone();
        let title = t("monthly_title", lang);
        let is_adm = self.is_admin(tg_user_id, None);

        let items: Vec<(String, String, String)> = match tokio::task::spawn_blocking(move || {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(_) => return Vec::new(),
            };
            let peers = Self::get_user_peers_sync(&conn, tg_user_id);
            if peers.is_empty() {
                return Vec::new();
            }

            let now = Utc::now().date_naive();
            let month_prefix = now.format("%Y-%m").to_string();
            let current_day = now.day() as usize;

            peers.into_iter().map(|peer| {
                let mut daily_map = std::collections::HashMap::new();
                if let Ok(mut stmt) = conn.prepare(
                    "SELECT day, rx, tx FROM usage_daily
                     WHERE peer_id = ?1 AND day LIKE ?2
                     ORDER BY day ASC"
                ) {
                    if let Ok(rows) = stmt.query_map(params![peer.id, format!("{}%", month_prefix)], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
                    }) {
                        for row in rows.flatten() {
                            if let Ok(d) = chrono::NaiveDate::parse_from_str(&row.0, "%Y-%m-%d") {
                                daily_map.insert(d.day() as usize, (row.1, row.2));
                            }
                        }
                    }
                }

                let mut total_rx = 0i64;
                let mut total_tx = 0i64;
                let mut points = Vec::with_capacity(current_day);

                for d in 1..=current_day {
                    let (rx, tx) = daily_map.get(&d).copied().unwrap_or((0, 0));
                    total_rx += rx;
                    total_tx += tx;
                    points.push((format!("{}/{}", now.month(), d), rx, tx));
                }

                let peer_name = if !peer.name.is_empty() { peer.name } else { peer.interface };
                let escaped_name = escape_html(&peer_name);
                let caption = format!("📅 <b>{}</b> - {}\n⬇️ {}\n⬆️ {}\n📈 <b>Total:</b> {}",
                    title, escaped_name, fmt_bytes(total_rx), fmt_bytes(total_tx), fmt_bytes(total_rx + total_tx));

                let svg = generate_usage_chart_svg(&title, &peer_name, total_rx, total_tx, &points);
                (peer_name, svg, caption)
            }).collect()
        }).await {
            Ok(res) => res,
            Err(_) => Vec::new(),
        };

        if items.is_empty() {
            if is_adm {
                let msg = "ℹ️ <b>No personal connections linked.</b>\nAs an administrator, you can view monthly usage for all peers in the 🛡️ <b>Admin Panel</b>.";
                let kb = json!({
                    "inline_keyboard": [
                        [
                            { "text": "📅 All Peers Month", "callback_data": "adm:peers:monthly" },
                            { "text": "🛡️ Admin Panel", "callback_data": "adm:menu" }
                        ],
                        [
                            { "text": "🏠 Home", "callback_data": "menu:home" }
                        ]
                    ]
                });
                let _ = self.send_message(chat_id, msg, Some(kb)).await;
            } else {
                let _ = self.send_message(chat_id, &t("no_peers", lang), None).await;
            }
            return;
        }

        for (_peer_name, svg, caption) in items {
            if let Ok(png) = render_svg_to_png_async(svg, 1.5).await {
                let _ = self.send_photo(chat_id, png, &caption, None).await;
            } else {
                let _ = self.send_message(chat_id, &caption, None).await;
            }
        }
    }

    async fn send_alltime_usage(&self, chat_id: i64, tg_user_id: i64, lang: &str) {
        let pool = self.pool.clone();
        let title = t("alltime_title", lang);
        let is_adm = self.is_admin(tg_user_id, None);

        let items: Vec<(String, String, String)> = match tokio::task::spawn_blocking(move || {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(_) => return Vec::new(),
            };
            let peers = Self::get_user_peers_sync(&conn, tg_user_id);
            if peers.is_empty() {
                return Vec::new();
            }

            peers.into_iter().map(|peer| {
                let mut points: Vec<(String, i64, i64)> = match conn.prepare(
                    "SELECT day, rx, tx FROM usage_daily WHERE peer_id = ?1 ORDER BY day ASC"
                ) {
                    Ok(mut stmt) => {
                        stmt.query_map(params![peer.id], |r| {
                            let day_str: String = r.get(0)?;
                            let rx: i64 = r.get(1)?;
                            let tx: i64 = r.get(2)?;
                            let short_day = if let Ok(d) = chrono::NaiveDate::parse_from_str(&day_str, "%Y-%m-%d") {
                                format!("{}/{}", d.month(), d.day())
                            } else {
                                day_str
                            };
                            Ok((short_day, rx, tx))
                        }).unwrap().flatten().collect()
                    }
                    Err(_) => Vec::new(),
                };

                let total_rx: i64 = points.iter().map(|p| p.1).sum();
                let total_tx: i64 = points.iter().map(|p| p.2).sum();

                if points.len() == 1 {
                    points.insert(0, ("Start".to_string(), 0, 0));
                }

                let peer_name = if !peer.name.is_empty() { peer.name } else { peer.interface };
                let escaped_name = escape_html(&peer_name);
                let caption = format!("📈 <b>{}</b> - {}\n⬇️ {}\n⬆️ {}\n📈 <b>Total:</b> {}",
                    title, escaped_name, fmt_bytes(total_rx), fmt_bytes(total_tx), fmt_bytes(total_rx + total_tx));

                let svg = generate_usage_chart_svg(&title, &peer_name, total_rx, total_tx, &points);
                (peer_name, svg, caption)
            }).collect()
        }).await {
            Ok(res) => res,
            Err(_) => Vec::new(),
        };

        if items.is_empty() {
            if is_adm {
                let msg = "ℹ️ <b>No personal connections linked.</b>\nAs an administrator, you can view all-time bandwidth for all peers in the 🛡️ <b>Admin Panel</b>.";
                let kb = json!({
                    "inline_keyboard": [
                        [
                            { "text": "📈 All Peers All Time", "callback_data": "adm:peers:alltime" },
                            { "text": "🛡️ Admin Panel", "callback_data": "adm:menu" }
                        ],
                        [
                            { "text": "🏠 Home", "callback_data": "menu:home" }
                        ]
                    ]
                });
                let _ = self.send_message(chat_id, msg, Some(kb)).await;
            } else {
                let _ = self.send_message(chat_id, &t("no_peers", lang), None).await;
            }
            return;
        }

        for (_peer_name, svg, caption) in items {
            if let Ok(png) = render_svg_to_png_async(svg, 1.5).await {
                let _ = self.send_photo(chat_id, png, &caption, None).await;
            } else {
                let _ = self.send_message(chat_id, &caption, None).await;
            }
        }
    }

    async fn send_fair_usage(&self, chat_id: i64, tg_user_id: i64, lang: &str) {
        let pool = self.pool.clone();
        let is_adm = self.is_admin(tg_user_id, None);

        let items: Vec<(String, String)> = match tokio::task::spawn_blocking(move || {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(_) => return Vec::new(),
            };
            let peers = Self::get_user_peers_sync(&conn, tg_user_id);
            if peers.is_empty() {
                return Vec::new();
            }

            let now_utc = Utc::now();
            let tz = parse_timezone("UTC");
            let calendar = "gregorian";

            peers.into_iter().map(|peer| {
                let dto = build_fair_usage_peer_status_dto(&conn, &peer, now_utc, tz, calendar, 1);
                let peer_name = if !peer.name.is_empty() { peer.name } else { peer.interface };
                let escaped_name = escape_html(&peer_name);
                let svg = generate_fair_usage_card_svg(&dto, &peer_name);
                let status_icon = if dto.throttled { "🔴" } else { "🟢" };
                let caption = format!("{} <b>Fair Usage Policy</b> - {}\nState: {}\nDown/Up: {} / {} Kbps",
                    status_icon, escaped_name, if dto.throttled { "Throttled" } else { "Normal" },
                    dto.throttle_download_kbps, dto.throttle_upload_kbps);
                (svg, caption)
            }).collect::<Vec<_>>()
        }).await {
            Ok(res) => res,
            Err(_) => Vec::new(),
        };

        if items.is_empty() {
            if is_adm {
                let msg = "ℹ️ <b>No personal connections linked.</b>\nAs an administrator, you can check system peers and status in the 🛡️ <b>Admin Panel</b>.";
                let kb = json!({
                    "inline_keyboard": [
                        [
                            { "text": "🛡️ Admin Panel", "callback_data": "adm:menu" },
                            { "text": "🏠 Home", "callback_data": "menu:home" }
                        ]
                    ]
                });
                let _ = self.send_message(chat_id, msg, Some(kb)).await;
            } else {
                let _ = self.send_message(chat_id, &t("no_peers", lang), None).await;
            }
            return;
        }

        for (svg, caption) in items {
            if let Ok(png) = render_svg_to_png_async(svg, 1.5).await {
                let _ = self.send_photo(chat_id, png, &caption, None).await;
            } else {
                let _ = self.send_message(chat_id, &caption, None).await;
            }
        }
    }

    async fn send_admin_menu(&self, chat_id: i64, tg_user_id: i64, lang: &str) {
        if !self.is_admin(tg_user_id, None) {
            let _ = self.send_message(chat_id, &t("admin_unauthorized", lang), None).await;
            return;
        }

        let pool = self.pool.clone();
        let (routers_count, peers_count, users_count, broadcasts_count) = match tokio::task::spawn_blocking(move || {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(_) => return (0, 0, 0, 0),
            };
            let rc: i64 = conn.query_row("SELECT COUNT(*) FROM routers", [], |r| r.get(0)).unwrap_or(0);
            let pc: i64 = conn.query_row("SELECT COUNT(*) FROM peers", [], |r| r.get(0)).unwrap_or(0);
            let uc: i64 = conn.query_row("SELECT COUNT(*) FROM telegram_users", [], |r| r.get(0)).unwrap_or(0);
            let bc: i64 = conn.query_row("SELECT COUNT(*) FROM telegram_broadcasts", [], |r| r.get(0)).unwrap_or(0);
            (rc, pc, uc, bc)
        }).await {
            Ok(res) => res,
            Err(_) => (0, 0, 0, 0),
        };

        let msg = format!(
            "🛡️ <b>Admin Dashboard</b>\n\n📡 <b>Routers:</b> {}\n👥 <b>Total Peers:</b> {}\n📱 <b>Telegram Users:</b> {}\n📢 <b>Broadcasts:</b> {}",
            routers_count, peers_count, users_count, broadcasts_count
        );

        let keyboard = json!({
            "inline_keyboard": [
                [
                    { "text": format!("👥 Users ({})", users_count), "callback_data": "adm:users:0" },
                    { "text": format!("📢 Outbox ({})", broadcasts_count), "callback_data": "adm:outbox:0" }
                ],
                [
                    { "text": "📊 All Peers: Today", "callback_data": "adm:peers:today" },
                    { "text": "📅 All Peers: Month", "callback_data": "adm:peers:monthly" }
                ],
                [
                    { "text": "📈 All Peers: All Time", "callback_data": "adm:peers:alltime" }
                ],
                [
                    { "text": "🏠 Home", "callback_data": "menu:home" }
                ]
            ]
        });

        let _ = self.send_message(chat_id, &msg, Some(keyboard)).await;
    }

    async fn send_admin_users(&self, chat_id: i64, tg_user_id: i64, page: i64, lang: &str) {
        if !self.is_admin(tg_user_id, None) {
            let _ = self.send_message(chat_id, &t("admin_unauthorized", lang), None).await;
            return;
        }

        let page_size = 6;
        let offset = page * page_size;
        let pool = self.pool.clone();

        let (total_users, user_items) = match tokio::task::spawn_blocking(move || {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(_) => return (0, Vec::new()),
            };
            let tu: i64 = conn.query_row("SELECT COUNT(*) FROM telegram_users", [], |r| r.get(0)).unwrap_or(0);
            let items: Vec<(i64, String, String, bool, i64)> = match conn.prepare(
                "SELECT u.id, u.telegram_username, u.first_name, u.is_blocked,
                        (SELECT COUNT(*) FROM telegram_peer_bindings b WHERE b.telegram_user_id = u.id) as peer_count
                 FROM telegram_users u ORDER BY u.id DESC LIMIT ?1 OFFSET ?2"
            ) {
                Ok(mut stmt) => {
                    stmt.query_map(params![page_size, offset], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, bool>(3)?,
                            row.get::<_, i64>(4)?,
                        ))
                    }).unwrap().flatten().collect()
                }
                Err(_) => Vec::new(),
            };
            (tu, items)
        }).await {
            Ok(res) => res,
            Err(_) => (0, Vec::new()),
        };

        let total_pages = (total_users + page_size - 1) / page_size;
        let mut user_buttons = Vec::new();
        for (id, uname, fname, is_blocked, pcount) in user_items {
            let label = if !uname.is_empty() {
                format!("@{}", uname)
            } else if !fname.is_empty() {
                fname
            } else {
                format!("User #{}", id)
            };
            let status_icon = if is_blocked { "🔴" } else { "🟢" };
            user_buttons.push(vec![
                json!({
                    "text": format!("{} {} ({} peers)", status_icon, label, pcount),
                    "callback_data": format!("adm:user:{}", id)
                })
            ]);
        }

        let mut nav_row = Vec::new();
        if page > 0 {
            nav_row.push(json!({ "text": "⬅️ Prev", "callback_data": format!("adm:users:{}", page - 1) }));
        }
        if page + 1 < total_pages {
            nav_row.push(json!({ "text": "Next ➡️", "callback_data": format!("adm:users:{}", page + 1) }));
        }
        if !nav_row.is_empty() {
            user_buttons.push(nav_row);
        }
        user_buttons.push(vec![
            json!({ "text": "« Back to Admin", "callback_data": "adm:menu" })
        ]);

        let text = format!("👥 <b>Telegram Users</b> (Page {} of {})\nTotal registered: {}", page + 1, std::cmp::max(1, total_pages), total_users);
        let keyboard = json!({ "inline_keyboard": user_buttons });
        let _ = self.send_message(chat_id, &text, Some(keyboard)).await;
    }

    async fn send_admin_user_detail(&self, chat_id: i64, tg_user_id: i64, user_db_id: i64, lang: &str) {
        if !self.is_admin(tg_user_id, None) {
            let _ = self.send_message(chat_id, &t("admin_unauthorized", lang), None).await;
            return;
        }

        let pool = self.pool.clone();
        let user_data = match tokio::task::spawn_blocking(move || {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(_) => return None,
            };

            let user_info = conn.query_row(
                "SELECT telegram_user_id, telegram_username, first_name, last_name, language, is_blocked, created_at FROM telegram_users WHERE id = ?1",
                params![user_db_id],
                |r| Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, bool>(5)?,
                    r.get::<_, String>(6)?,
                )),
            );

            match user_info {
                Ok((target_tg_id, uname, fname, lname, user_lang, is_blocked, created_at)) => {
                    let peers: Vec<String> = match conn.prepare(
                        "SELECT p.id, p.name, p.public_key, p.interface FROM telegram_peer_bindings b
                         JOIN peers p ON b.peer_id = p.id WHERE b.telegram_user_id = ?1"
                    ) {
                        Ok(mut stmt) => {
                            stmt.query_map(params![user_db_id], |r| {
                                let name: String = r.get(1)?;
                                let pub_key: String = r.get(2)?;
                                let iface: String = r.get(3)?;
                                let display = if !name.is_empty() { name } else { pub_key.chars().take(8).collect::<String>() };
                                Ok(format!("• {} ({})", escape_html(&display), escape_html(&iface)))
                            }).unwrap().flatten().collect()
                        }
                        Err(_) => Vec::new(),
                    };
                    Some((target_tg_id, uname, fname, lname, user_lang, is_blocked, created_at, peers))
                }
                Err(_) => None,
            }
        }).await {
            Ok(res) => res,
            Err(_) => None,
        };

        let (target_tg_id, uname, fname, lname, user_lang, is_blocked, created_at, peers) = match user_data {
            Some(d) => d,
            None => {
                let _ = self.send_message(chat_id, "User not found", None).await;
                return;
            }
        };

        let peers_list = if peers.is_empty() { "No peers linked".to_string() } else { peers.join("\n") };
        let block_label = if is_blocked { "🟢 Unblock User" } else { "🚫 Block User" };

        let text = format!(
            "👤 <b>User Details</b>\n\n<b>ID:</b> <code>{}</code>\n<b>Username:</b> @{}\n<b>Name:</b> {} {}\n<b>Language:</b> {}\n<b>Status:</b> {}\n<b>Created:</b> {}\n\n<b>Linked Peers:</b>\n{}",
            target_tg_id, escape_html(&uname), escape_html(&fname), escape_html(&lname), escape_html(&user_lang), if is_blocked { "🔴 Blocked" } else { "🟢 Active" }, created_at, peers_list
        );

        let keyboard = json!({
            "inline_keyboard": [
                [
                    { "text": block_label, "callback_data": format!("adm:toggle_block:{}", user_db_id) }
                ],
                [
                    { "text": "« Back to Users", "callback_data": "adm:users:0" },
                    { "text": "🛡️ Admin Menu", "callback_data": "adm:menu" }
                ]
            ]
        });

        let _ = self.send_message(chat_id, &text, Some(keyboard)).await;
    }

    async fn toggle_user_block(&self, chat_id: i64, tg_user_id: i64, user_db_id: i64, lang: &str) {
        if !self.is_admin(tg_user_id, None) {
            return;
        }

        let pool = self.pool.clone();
        let target_tg_id = tokio::task::spawn_blocking(move || {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(_) => return None,
            };
            let _ = conn.execute("UPDATE telegram_users SET is_blocked = NOT is_blocked WHERE id = ?1", params![user_db_id]);
            conn.query_row("SELECT telegram_user_id FROM telegram_users WHERE id = ?1", params![user_db_id], |r| r.get::<_, i64>(0)).ok()
        }).await.unwrap_or(None);

        if let Some(uid) = target_tg_id {
            if let Ok(mut cache) = self.user_cache.lock() {
                cache.remove(&uid);
            }
        }

        self.send_admin_user_detail(chat_id, tg_user_id, user_db_id, lang).await;
    }

    async fn send_admin_peers_usage(&self, chat_id: i64, tg_user_id: i64, scope: &str, lang: &str) {
        if !self.is_admin(tg_user_id, None) {
            let _ = self.send_message(chat_id, &t("admin_unauthorized", lang), None).await;
            return;
        }

        let pool = self.pool.clone();
        let scope_str = scope.to_string();

        let (total_rx, total_tx, lines) = match tokio::task::spawn_blocking(move || {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(_) => return (0, 0, Vec::new()),
            };

            let today_utc = Utc::now().date_naive().format("%Y-%m-%d").to_string();
            let month_prefix = format!("{}%", Utc::now().date_naive().format("%Y-%m"));

            // Optimized single SQL queries with JOIN
            let results: Vec<(String, String, i64, i64)> = match scope_str.as_str() {
                "today" => {
                    let mut stmt = match conn.prepare(
                        "SELECT p.name, p.public_key, p.interface, COALESCE(d.rx, 0), COALESCE(d.tx, 0)
                         FROM peers p
                         LEFT JOIN usage_daily d ON p.id = d.peer_id AND d.day = ?1
                         ORDER BY p.name ASC"
                    ) {
                        Ok(s) => s,
                        Err(_) => return (0, 0, Vec::new()),
                    };
                    stmt.query_map(params![today_utc], |r| {
                        let name: String = r.get(0)?;
                        let pubkey: String = r.get(1)?;
                        let iface: String = r.get(2)?;
                        let display = if !name.is_empty() { name } else { pubkey.chars().take(8).collect() };
                        Ok((display, iface, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?))
                    }).unwrap().flatten().collect()
                }
                "monthly" => {
                    let mut stmt = match conn.prepare(
                        "SELECT p.name, p.public_key, p.interface, COALESCE(SUM(d.rx), 0), COALESCE(SUM(d.tx), 0)
                         FROM peers p
                         LEFT JOIN usage_daily d ON p.id = d.peer_id AND d.day LIKE ?1
                         GROUP BY p.id
                         ORDER BY p.name ASC"
                    ) {
                        Ok(s) => s,
                        Err(_) => return (0, 0, Vec::new()),
                    };
                    stmt.query_map(params![month_prefix], |r| {
                        let name: String = r.get(0)?;
                        let pubkey: String = r.get(1)?;
                        let iface: String = r.get(2)?;
                        let display = if !name.is_empty() { name } else { pubkey.chars().take(8).collect() };
                        Ok((display, iface, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?))
                    }).unwrap().flatten().collect()
                }
                _ => {
                    let mut stmt = match conn.prepare(
                        "SELECT p.name, p.public_key, p.interface, COALESCE(SUM(d.rx), 0), COALESCE(SUM(d.tx), 0)
                         FROM peers p
                         LEFT JOIN usage_daily d ON p.id = d.peer_id
                         GROUP BY p.id
                         ORDER BY p.name ASC"
                    ) {
                        Ok(s) => s,
                        Err(_) => return (0, 0, Vec::new()),
                    };
                    stmt.query_map([], |r| {
                        let name: String = r.get(0)?;
                        let pubkey: String = r.get(1)?;
                        let iface: String = r.get(2)?;
                        let display = if !name.is_empty() { name } else { pubkey.chars().take(8).collect() };
                        Ok((display, iface, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?))
                    }).unwrap().flatten().collect()
                }
            };

            let mut trx: i64 = 0;
            let mut ttx: i64 = 0;
            let mut lns = Vec::new();

            for (display, iface, rx, tx) in results {
                trx += rx;
                ttx += tx;
                if rx > 0 || tx > 0 {
                    lns.push(format!("• <b>{}</b> ({})\n  ⬇️ {} | ⬆️ {} | Total: {}",
                        escape_html(&display), escape_html(&iface), fmt_bytes(rx), fmt_bytes(tx), fmt_bytes(rx + tx)));
                }
            }

            (trx, ttx, lns)
        }).await {
            Ok(res) => res,
            Err(_) => (0, 0, Vec::new()),
        };

        let scope_title = match scope {
            "today" => "Today's Bandwidth",
            "monthly" => "This Month's Bandwidth",
            _ => "All-time Bandwidth",
        };

        let summary_header = format!(
            "📊 <b>All Peers Summary ({})</b>\n⬇️ {}\n⬆️ {}\n📈 <b>Total: {}</b>\n\n",
            scope_title, fmt_bytes(total_rx), fmt_bytes(total_tx), fmt_bytes(total_rx + total_tx)
        );

        let body = if lines.is_empty() {
            format!("{}<i>No bandwidth recorded for this period.</i>", summary_header)
        } else {
            let combined = lines.join("\n\n");
            if combined.chars().count() > 3000 {
                format!("{}{}\n<i>...and more</i>", summary_header, lines.iter().take(15).cloned().collect::<Vec<_>>().join("\n\n"))
            } else {
                format!("{}{}", summary_header, combined)
            }
        };

        let keyboard = json!({
            "inline_keyboard": [
                [
                    { "text": "📊 Today", "callback_data": "adm:peers:today" },
                    { "text": "📅 Monthly", "callback_data": "adm:peers:monthly" },
                    { "text": "📈 All Time", "callback_data": "adm:peers:alltime" }
                ],
                [
                    { "text": "« Back to Admin", "callback_data": "adm:menu" }
                ]
            ]
        });

        let _ = self.send_message(chat_id, &body, Some(keyboard)).await;
    }

    async fn send_admin_outbox(&self, chat_id: i64, tg_user_id: i64, page: i64, lang: &str) {
        if !self.is_admin(tg_user_id, None) {
            let _ = self.send_message(chat_id, &t("admin_unauthorized", lang), None).await;
            return;
        }

        let page_size = 5;
        let offset = page * page_size;
        let pool = self.pool.clone();

        let (total, items) = match tokio::task::spawn_blocking(move || {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(_) => return (0, Vec::new()),
            };
            let tot: i64 = conn.query_row("SELECT COUNT(*) FROM telegram_broadcasts", [], |r| r.get(0)).unwrap_or(0);
            let b_items: Vec<String> = match conn.prepare(
                "SELECT id, body, recipient_mode, status, total_count, sent_count, failed_count, created_at
                 FROM telegram_broadcasts ORDER BY id DESC LIMIT ?1 OFFSET ?2"
            ) {
                Ok(mut stmt) => {
                    stmt.query_map(params![page_size, offset], |row| {
                        let id: i64 = row.get(0)?;
                        let body: String = row.get(1)?;
                        let mode: String = row.get(2)?;
                        let status: String = row.get(3)?;
                        let total_c: i64 = row.get(4)?;
                        let sent_c: i64 = row.get(5)?;
                        let failed_c: i64 = row.get(6)?;
                        let created_at: String = row.get(7)?;
                        let preview: String = body.chars().take(40).collect();
                        Ok(format!(
                            "📢 <b>Broadcast #{}</b> ({})\nStatus: <b>{}</b> (sent: {}, failed: {}, total: {})\nCreated: {}\nPreview: <i>{}</i>",
                            id, escape_html(&mode), escape_html(&status), sent_c, failed_c, total_c, escape_html(&created_at), escape_html(&preview)
                        ))
                    }).unwrap().flatten().collect()
                }
                Err(_) => Vec::new(),
            };
            (tot, b_items)
        }).await {
            Ok(res) => res,
            Err(_) => (0, Vec::new()),
        };

        let total_pages = (total + page_size - 1) / page_size;
        let text = if items.is_empty() {
            "📢 <b>Outbox Broadcasts</b>\nNo broadcasts created yet.".to_string()
        } else {
            format!("📢 <b>Outbox Broadcasts</b> (Page {} of {})\n\n{}", page + 1, std::cmp::max(1, total_pages), items.join("\n\n"))
        };

        let mut nav_row = Vec::new();
        if page > 0 {
            nav_row.push(json!({ "text": "⬅️ Prev", "callback_data": format!("adm:outbox:{}", page - 1) }));
        }
        if page + 1 < total_pages {
            nav_row.push(json!({ "text": "Next ➡️", "callback_data": format!("adm:outbox:{}", page + 1) }));
        }

        let mut buttons = Vec::new();
        if !nav_row.is_empty() {
            buttons.push(nav_row);
        }
        buttons.push(vec![json!({ "text": "« Back to Admin", "callback_data": "adm:menu" })]);

        let keyboard = json!({ "inline_keyboard": buttons });
        let _ = self.send_message(chat_id, &text, Some(keyboard)).await;
    }

    fn get_user_peers_sync(conn: &rusqlite::Connection, tg_user_id: i64) -> Vec<Peer> {
        let mut stmt = match conn.prepare(
            "SELECT p.id, p.router_id, p.interface, p.ros_id, p.name, p.public_key, p.allowed_address,
                    p.comment, p.disabled, p.selected, p.router_sync_status
             FROM peers p
             JOIN telegram_peer_bindings b ON p.id = b.peer_id
             JOIN telegram_users u ON b.telegram_user_id = u.id
             WHERE u.telegram_user_id = ?1 AND b.visible = 1",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = stmt.query_map(params![tg_user_id], |row| {
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
            for item in r.flatten() {
                list.push(item);
            }
        }
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_pool;

    #[test]
    fn test_in_flight_acquire_and_debouncing() {
        let temp_dir = std::env::temp_dir().join(format!("wgmik_tg_test_{}", rand::random::<u64>()));
        let db_path = temp_dir.join("test.db");
        let db_url = format!("sqlite:///{}", db_path.display());
        let pool = create_pool(&db_url);
        let bot = TelegramBot::new("test_token".to_string(), pool, "test_secret".to_string());

        let chat_id_1: i64 = 1001;
        let chat_id_2: i64 = 1002;

        // First request from chat_id_1: should acquire successfully
        let guard_1 = match bot.try_acquire_in_flight(chat_id_1) {
            InFlightStatus::Acquired(g) => g,
            InFlightStatus::AlreadyRunning { .. } => panic!("Expected first acquire to succeed"),
        };

        // Second immediate request from chat_id_1 while guard_1 is alive: should return AlreadyRunning { should_notify: true }
        match bot.try_acquire_in_flight(chat_id_1) {
            InFlightStatus::AlreadyRunning { should_notify } => {
                assert!(should_notify, "Expected should_notify=true on first duplicate request");
            }
            InFlightStatus::Acquired(_) => panic!("Expected second acquire to be rejected"),
        }

        // Third immediate request from chat_id_1 within 1.5s: should return AlreadyRunning { should_notify: false } (debounced)
        match bot.try_acquire_in_flight(chat_id_1) {
            InFlightStatus::AlreadyRunning { should_notify } => {
                assert!(!should_notify, "Expected should_notify=false on rapid subsequent duplicate request to prevent spam");
            }
            InFlightStatus::Acquired(_) => panic!("Expected third acquire to be rejected"),
        }

        // Request from chat_id_2: should succeed independently
        let guard_2 = match bot.try_acquire_in_flight(chat_id_2) {
            InFlightStatus::Acquired(g) => g,
            InFlightStatus::AlreadyRunning { .. } => panic!("Expected chat_id_2 to acquire successfully"),
        };

        // Drop guard_1: chat_id_1 should now be free to acquire again
        drop(guard_1);

        let guard_1_new = match bot.try_acquire_in_flight(chat_id_1) {
            InFlightStatus::Acquired(g) => g,
            InFlightStatus::AlreadyRunning { .. } => panic!("Expected acquire to succeed after guard drop"),
        };

        drop(guard_1_new);
        drop(guard_2);
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_heavy_concurrency_limiter() {
        let temp_dir = std::env::temp_dir().join(format!("wgmik_tg_test_heavy_{}", rand::random::<u64>()));
        let db_path = temp_dir.join("test.db");
        let db_url = format!("sqlite:///{}", db_path.display());
        let pool = create_pool(&db_url);
        let bot = TelegramBot::new("test_token".to_string(), pool, "test_secret".to_string());

        // Capacity is 6
        let p1 = bot.heavy_ops_semaphore.clone().try_acquire_owned();
        let p2 = bot.heavy_ops_semaphore.clone().try_acquire_owned();
        let p3 = bot.heavy_ops_semaphore.clone().try_acquire_owned();
        let p4 = bot.heavy_ops_semaphore.clone().try_acquire_owned();
        let p5 = bot.heavy_ops_semaphore.clone().try_acquire_owned();
        let p6 = bot.heavy_ops_semaphore.clone().try_acquire_owned();

        assert!(p1.is_ok());
        assert!(p2.is_ok());
        assert!(p3.is_ok());
        assert!(p4.is_ok());
        assert!(p5.is_ok());
        assert!(p6.is_ok());

        // 7th attempt must fail immediately because all 6 slots are occupied
        let p7 = bot.heavy_ops_semaphore.clone().try_acquire_owned();
        assert!(p7.is_err(), "Expected 7th permit attempt to fail when all threads/slots are busy");

        // Release one permit
        drop(p1);

        // Now an attempt should succeed
        let p8 = bot.heavy_ops_semaphore.clone().try_acquire_owned();
        assert!(p8.is_ok(), "Expected permit acquisition to succeed after slot release");

        drop(p2);
        drop(p3);
        drop(p4);
        drop(p5);
        drop(p6);
        drop(p8);
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[tokio::test]
    async fn test_user_cache_and_admin_check() {
        let temp_dir = std::env::temp_dir().join(format!("wgmik_tg_test_cache_{}", rand::random::<u64>()));
        let db_path = temp_dir.join("test.db");
        let db_url = format!("sqlite:///{}", db_path.display());
        let pool = create_pool(&db_url);
        let bot = TelegramBot::new("test_token".to_string(), pool.clone(), "test_secret".to_string());

        // Initial user sync
        let (lang, blocked) = bot.get_or_sync_user(12345, "testuser", "Test", "User").await;
        assert_eq!(lang, "en");
        assert!(!blocked);

        // Language update
        bot.set_user_language(12345, "fa").await;
        let (lang2, _) = bot.get_or_sync_user(12345, "testuser", "Test", "User").await;
        assert_eq!(lang2, "fa");

        // Admin check with configured admin
        {
            let conn = pool.get().unwrap();
            conn.execute("INSERT INTO settings_kv (key, value) VALUES ('tg_admin_chat_id', '99999')", []).unwrap();
        }

        assert!(bot.is_admin(99999, None));
        assert!(!bot.is_admin(12345, None));

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[tokio::test]
    async fn test_async_svg_render() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect width="100" height="100" fill="#2563eb"/></svg>"##.to_string();
        let png_res = render_svg_to_png_async(svg, 1.0).await;
        assert!(png_res.is_ok(), "Expected async render to succeed: {:?}", png_res.err());
        let png = png_res.unwrap();
        assert!(!png.is_empty());
        assert_eq!(&png[0..4], &[0x89, b'P', b'N', b'G']);
    }
}
