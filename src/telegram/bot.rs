use super::fair_usage_card::generate_fair_usage_card_svg;
use super::i18n::t;
use super::svg_render::{fmt_bytes, render_svg_to_png};
use super::usage_chart::generate_usage_chart_svg;
use crate::calendar::parse_timezone;
use crate::db::models::Peer;
use crate::db::DbPool;
use crate::fair_usage::build_fair_usage_peer_status_dto;
use chrono::Utc;
use reqwest::multipart::{Form, Part};
use reqwest::Client;
use rusqlite::params;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}

#[derive(Clone)]
pub struct TelegramBot {
    client: Client,
    pub token: String,
    pool: DbPool,
    running: Arc<AtomicBool>,
    started_at: chrono::DateTime<Utc>,
}

impl TelegramBot {
    pub fn new(token: String, pool: DbPool, _secret_key: String) -> Self {
        Self {
            client: Client::builder().timeout(Duration::from_secs(35)).build().unwrap(),
            token,
            pool,
            running: Arc::new(AtomicBool::new(false)),
            started_at: Utc::now(),
        }
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

        let conn = match self.pool.get() {
            Ok(c) => c,
            Err(_) => return,
        };
        let raw: String = conn.query_row(
            "SELECT value FROM settings_kv WHERE key = 'tg_admin_chat_id'",
            [],
            |r| r.get(0),
        ).unwrap_or_default();
        let clean = raw.trim().trim_start_matches('@');
        if let Ok(admin_id) = clean.parse::<i64>() {
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
        let conn = match self.pool.get() {
            Ok(c) => c,
            Err(_) => return false,
        };
        let raw: String = conn.query_row(
            "SELECT value FROM settings_kv WHERE key = 'tg_admin_chat_id'",
            [],
            |r| r.get(0),
        ).unwrap_or_default();

        let clean = raw.trim().trim_start_matches('@').trim_matches('"').trim_matches('\'');
        if clean.is_empty() {
            return false;
        }

        if let Ok(admin_id) = clean.parse::<i64>() {
            if admin_id == tg_user_id {
                return true;
            }
        }

        if let Some(uname) = username {
            if !uname.is_empty() && uname.trim_start_matches('@').eq_ignore_ascii_case(clean) {
                return true;
            }
        }

        let db_uname: Option<String> = conn.query_row(
            "SELECT telegram_username FROM telegram_users WHERE telegram_user_id = ?1",
            params![tg_user_id],
            |r| r.get(0),
        ).ok();

        if let Some(u) = db_uname {
            if !u.is_empty() && u.trim_start_matches('@').eq_ignore_ascii_case(clean) {
                return true;
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

        let lang = self.ensure_telegram_user(tg_user_id, username, first_name, last_name);

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

        let lang = self.ensure_telegram_user(tg_user_id, username, first_name, last_name);
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
            self.set_user_language(tg_user_id, "en");
            let _ = self.send_message(chat_id, &t("lang_changed", "en"), None).await;
            self.send_home_menu(chat_id, tg_user_id, "en").await;
        } else if data == "lang:fa" {
            self.set_user_language(tg_user_id, "fa");
            let _ = self.send_message(chat_id, &t("lang_changed", "fa"), None).await;
            self.send_home_menu(chat_id, tg_user_id, "fa").await;
        }
    }

    fn ensure_telegram_user(&self, tg_user_id: i64, username: &str, first_name: &str, last_name: &str) -> String {
        let conn = match self.pool.get() {
            Ok(c) => c,
            Err(_) => return "en".to_string(),
        };
        let existing = conn.query_row(
            "SELECT language FROM telegram_users WHERE telegram_user_id = ?1",
            params![tg_user_id],
            |row| row.get::<_, String>(0),
        );

        if let Ok(lang) = existing {
            let _ = conn.execute(
                "UPDATE telegram_users SET telegram_username = ?1, first_name = ?2, last_name = ?3 WHERE telegram_user_id = ?4",
                params![username, first_name, last_name, tg_user_id],
            );
            lang
        } else {
            let _ = conn.execute(
                "INSERT INTO telegram_users (telegram_user_id, telegram_username, first_name, last_name, language)
                 VALUES (?1, ?2, ?3, ?4, 'en')",
                params![tg_user_id, username, first_name, last_name],
            );
            "en".to_string()
        }
    }

    fn get_user_language(&self, tg_user_id: i64) -> String {
        let conn = match self.pool.get() {
            Ok(c) => c,
            Err(_) => return "en".to_string(),
        };
        conn.query_row(
            "SELECT language FROM telegram_users WHERE telegram_user_id = ?1",
            params![tg_user_id],
            |row| row.get::<_, String>(0),
        ).unwrap_or_else(|_| "en".to_string())
    }

    fn set_user_language(&self, tg_user_id: i64, lang: &str) {
        let conn = match self.pool.get() {
            Ok(c) => c,
            Err(_) => return,
        };
        let _ = conn.execute(
            "UPDATE telegram_users SET language = ?1 WHERE telegram_user_id = ?2",
            params![lang, tg_user_id],
        );
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

        let result = {
            let conn = match self.pool.get() {
                Ok(c) => c,
                Err(_) => return,
            };
            let token_row = conn.query_row(
                "SELECT id, peer_ids, used_at, single_use FROM telegram_signup_tokens WHERE token = ?1",
                params![token],
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
        let items: Vec<(String, i64, i64)> = {
            let conn = match self.pool.get() {
                Ok(c) => c,
                Err(_) => return,
            };
            let peers = self.get_user_peers(&conn, tg_user_id);
            if peers.is_empty() {
                Vec::new()
            } else {
                let today_utc = Utc::now().date_naive().format("%Y-%m-%d").to_string();
                peers.into_iter().map(|peer| {
                    let (rx, tx) = conn.query_row(
                        "SELECT rx, tx FROM usage_daily WHERE peer_id = ?1 AND day = ?2",
                        params![peer.id, today_utc],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                    ).unwrap_or((0, 0));
                    let peer_name = if !peer.name.is_empty() { peer.name } else { peer.interface };
                    (peer_name, rx, tx)
                }).collect()
            }
        };

        if items.is_empty() {
            if self.is_admin(tg_user_id, None) {
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

        let title = t("today_title", lang);
        for (peer_name, rx, tx) in items {
            let escaped_name = escape_html(&peer_name);
            let caption = format!("📊 <b>{}</b> - {}\n⬇️ {}\n⬆️ {}\n📈 <b>Total:</b> {}",
                title, escaped_name, fmt_bytes(rx), fmt_bytes(tx), fmt_bytes(rx + tx));

            let svg = generate_usage_chart_svg(&title, &peer_name, rx, tx, &[("Today".to_string(), rx, tx)]);
            if let Ok(png) = render_svg_to_png(&svg, 2.0) {
                let _ = self.send_photo(chat_id, png, &caption, None).await;
            } else {
                let _ = self.send_message(chat_id, &caption, None).await;
            }
        }
    }

    async fn send_monthly_usage(&self, chat_id: i64, tg_user_id: i64, lang: &str) {
        let items: Vec<(String, (i64, i64), Vec<(String, i64, i64)>)> = {
            let conn = match self.pool.get() {
                Ok(c) => c,
                Err(_) => return,
            };
            let peers = self.get_user_peers(&conn, tg_user_id);
            if peers.is_empty() {
                Vec::new()
            } else {
                let month_prefix = Utc::now().date_naive().format("%Y-%m").to_string();
                peers.into_iter().map(|peer| {
                    let points: Vec<(String, i64, i64)> = match conn.prepare(
                        "SELECT day, rx, tx FROM usage_daily
                         WHERE peer_id = ?1 AND day LIKE ?2 ORDER BY day ASC",
                    ) {
                        Ok(mut stmt) => {
                            stmt.query_map(params![peer.id, format!("{}%", month_prefix)], |row| {
                                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
                            }).unwrap().flatten().collect()
                        }
                        Err(_) => Vec::new(),
                    };
                    let totals = points.iter().fold((0i64, 0i64), |acc, p| (acc.0 + p.1, acc.1 + p.2));
                    let peer_name = if !peer.name.is_empty() { peer.name } else { peer.interface };
                    (peer_name, totals, points)
                }).collect()
            }
        };

        if items.is_empty() {
            if self.is_admin(tg_user_id, None) {
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

        let title = t("monthly_title", lang);
        for (peer_name, totals, points) in items {
            let escaped_name = escape_html(&peer_name);
            let caption = format!("📅 <b>{}</b> - {}\n⬇️ {}\n⬆️ {}\n📈 <b>Total:</b> {}",
                title, escaped_name, fmt_bytes(totals.0), fmt_bytes(totals.1), fmt_bytes(totals.0 + totals.1));

            let svg = generate_usage_chart_svg(&title, &peer_name, totals.0, totals.1, &points);
            if let Ok(png) = render_svg_to_png(&svg, 2.0) {
                let _ = self.send_photo(chat_id, png, &caption, None).await;
            } else {
                let _ = self.send_message(chat_id, &caption, None).await;
            }
        }
    }

    async fn send_alltime_usage(&self, chat_id: i64, tg_user_id: i64, lang: &str) {
        let items: Vec<(String, (i64, i64))> = {
            let conn = match self.pool.get() {
                Ok(c) => c,
                Err(_) => return,
            };
            let peers = self.get_user_peers(&conn, tg_user_id);
            if peers.is_empty() {
                Vec::new()
            } else {
                peers.into_iter().map(|peer| {
                    let totals: (i64, i64) = conn.query_row(
                        "SELECT COALESCE(SUM(rx), 0), COALESCE(SUM(tx), 0) FROM usage_daily WHERE peer_id = ?1",
                        params![peer.id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    ).unwrap_or((0, 0));
                    let peer_name = if !peer.name.is_empty() { peer.name } else { peer.interface };
                    (peer_name, totals)
                }).collect()
            }
        };

        if items.is_empty() {
            if self.is_admin(tg_user_id, None) {
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

        let title = t("alltime_title", lang);
        for (peer_name, totals) in items {
            let escaped_name = escape_html(&peer_name);
            let caption = format!("📈 <b>{}</b> - {}\n⬇️ {}\n⬆️ {}\n📈 <b>Total:</b> {}",
                title, escaped_name, fmt_bytes(totals.0), fmt_bytes(totals.1), fmt_bytes(totals.0 + totals.1));

            let svg = generate_usage_chart_svg(&title, &peer_name, totals.0, totals.1, &[("All Time".to_string(), totals.0, totals.1)]);
            if let Ok(png) = render_svg_to_png(&svg, 2.0) {
                let _ = self.send_photo(chat_id, png, &caption, None).await;
            } else {
                let _ = self.send_message(chat_id, &caption, None).await;
            }
        }
    }

    async fn send_fair_usage(&self, chat_id: i64, tg_user_id: i64, lang: &str) {
        let items = {
            let conn = match self.pool.get() {
                Ok(c) => c,
                Err(_) => return,
            };
            let peers = self.get_user_peers(&conn, tg_user_id);
            if peers.is_empty() {
                Vec::new()
            } else {
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
            }
        };

        if items.is_empty() {
            if self.is_admin(tg_user_id, None) {
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
            if let Ok(png) = render_svg_to_png(&svg, 2.0) {
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

        let (routers_count, peers_count, users_count, broadcasts_count) = {
            let conn = match self.pool.get() {
                Ok(c) => c,
                Err(_) => return,
            };
            let rc: i64 = conn.query_row("SELECT COUNT(*) FROM routers", [], |r| r.get(0)).unwrap_or(0);
            let pc: i64 = conn.query_row("SELECT COUNT(*) FROM peers", [], |r| r.get(0)).unwrap_or(0);
            let uc: i64 = conn.query_row("SELECT COUNT(*) FROM telegram_users", [], |r| r.get(0)).unwrap_or(0);
            let bc: i64 = conn.query_row("SELECT COUNT(*) FROM telegram_broadcasts", [], |r| r.get(0)).unwrap_or(0);
            (rc, pc, uc, bc)
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

        let (total_users, user_items) = {
            let conn = match self.pool.get() {
                Ok(c) => c,
                Err(_) => return,
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

        let user_data = {
            let conn = match self.pool.get() {
                Ok(c) => c,
                Err(_) => return,
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

        {
            let conn = match self.pool.get() {
                Ok(c) => c,
                Err(_) => return,
            };
            let _ = conn.execute("UPDATE telegram_users SET is_blocked = NOT is_blocked WHERE id = ?1", params![user_db_id]);
        }

        self.send_admin_user_detail(chat_id, tg_user_id, user_db_id, lang).await;
    }

    async fn send_admin_peers_usage(&self, chat_id: i64, tg_user_id: i64, scope: &str, lang: &str) {
        if !self.is_admin(tg_user_id, None) {
            let _ = self.send_message(chat_id, &t("admin_unauthorized", lang), None).await;
            return;
        }

        let (total_rx, total_tx, lines) = {
            let conn = match self.pool.get() {
                Ok(c) => c,
                Err(_) => return,
            };

            let peer_rows: Vec<(i64, String, String, String)> = match conn.prepare(
                "SELECT p.id, p.name, p.public_key, p.interface FROM peers p ORDER BY p.name ASC"
            ) {
                Ok(mut stmt) => {
                    stmt.query_map([], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                    }).unwrap().flatten().collect()
                }
                Err(_) => Vec::new(),
            };

            let mut trx: i64 = 0;
            let mut ttx: i64 = 0;
            let mut lns = Vec::new();

            let today_utc = Utc::now().date_naive().format("%Y-%m-%d").to_string();
            let month_prefix = Utc::now().date_naive().format("%Y-%m").to_string();

            for (pid, name, pubkey, iface) in peer_rows {
                let display = if !name.is_empty() { name } else { pubkey.chars().take(8).collect::<String>() };
                let (rx, tx) = match scope {
                    "today" => {
                        conn.query_row(
                            "SELECT rx, tx FROM usage_daily WHERE peer_id = ?1 AND day = ?2",
                            params![pid, today_utc],
                            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                        ).unwrap_or((0, 0))
                    },
                    "monthly" => {
                        conn.query_row(
                            "SELECT COALESCE(SUM(rx), 0), COALESCE(SUM(tx), 0) FROM usage_daily WHERE peer_id = ?1 AND day LIKE ?2",
                            params![pid, format!("{}%", month_prefix)],
                            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                        ).unwrap_or((0, 0))
                    },
                    _ => {
                        conn.query_row(
                            "SELECT COALESCE(SUM(rx), 0), COALESCE(SUM(tx), 0) FROM usage_daily WHERE peer_id = ?1",
                            params![pid],
                            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                        ).unwrap_or((0, 0))
                    }
                };

                trx += rx;
                ttx += tx;
                if rx > 0 || tx > 0 {
                    lns.push(format!("• <b>{}</b> ({})\n  ⬇️ {} | ⬆️ {} | Total: {}",
                        escape_html(&display), escape_html(&iface), fmt_bytes(rx), fmt_bytes(tx), fmt_bytes(rx + tx)));
                }
            }

            (trx, ttx, lns)
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

        let (total, items) = {
            let conn = match self.pool.get() {
                Ok(c) => c,
                Err(_) => return,
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

    fn get_user_peers(&self, conn: &rusqlite::Connection, tg_user_id: i64) -> Vec<Peer> {
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
            for item in r {
                if let Ok(p) = item {
                    list.push(p);
                }
            }
        }
        list
    }
}
