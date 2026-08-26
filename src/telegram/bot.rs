use super::fair_usage_card::generate_fair_usage_card_svg;
use super::i18n::t;
use super::svg_render::{fmt_bytes, render_svg_to_png};
use super::usage_chart::generate_usage_chart_svg;
use crate::accounting::deltas::counter_day_key;
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
use tracing::{info, warn};

#[derive(Clone)]
pub struct TelegramBot {
    client: Client,
    token: String,
    pool: DbPool,
    secret_key: String,
    running: Arc<AtomicBool>,
}

impl TelegramBot {
    pub fn new(token: String, pool: DbPool, secret_key: String) -> Self {
        Self {
            client: Client::builder().timeout(Duration::from_secs(35)).build().unwrap(),
            token,
            pool,
            secret_key,
            running: Arc::new(AtomicBool::new(false)),
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
        if let Some(rm) = reply_markup {
            body["reply_markup"] = rm;
        }

        let resp = self.client.post(&self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        resp.json::<Value>().await.map_err(|e| e.to_string())
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

        if let Some(rm) = reply_markup {
            form = form.text("reply_markup", rm.to_string());
        }

        let resp = self.client.post(&self.api_url("sendPhoto"))
            .multipart(form)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        resp.json::<Value>().await.map_err(|e| e.to_string())
    }

    pub async fn answer_callback_query(&self, callback_query_id: &str, text: Option<&str>) -> Result<(), String> {
        let mut body = json!({ "callback_query_id": callback_query_id });
        if let Some(t) = text {
            body["text"] = json!(t);
        }
        let _ = self.client.post(&self.api_url("answerCallbackQuery")).json(&body).send().await;
        Ok(())
    }

    pub async fn start_polling(self: Arc<Self>) {
        self.running.store(true, Ordering::SeqCst);
        info!("Telegram bot polling started");

        let mut offset: i64 = 0;
        while self.running.load(Ordering::SeqCst) {
            let url = format!("{}?offset={}&timeout=20", self.api_url("getUpdates"), offset);
            match self.client.get(&url).send().await {
                Ok(resp) => {
                    if let Ok(data) = resp.json::<Value>().await {
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
                    }
                }
                Err(e) => {
                    warn!("Telegram polling error: {}", e);
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

        // Ensure user in DB
        let lang = self.ensure_telegram_user(tg_user_id, username, first_name, last_name);

        if text.starts_with("/start") {
            let parts: Vec<&str> = text.split_whitespace().collect();
            if parts.len() > 1 {
                // Token signup deep-link
                let token = parts[1];
                self.handle_signup_token(chat_id, tg_user_id, token, &lang).await;
            } else {
                self.send_home_menu(chat_id, &lang).await;
            }
        } else if text == "/home" {
            self.send_home_menu(chat_id, &lang).await;
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
        } else if text == "/admin" {
            self.send_admin_menu(chat_id, tg_user_id, &lang).await;
        } else {
            self.send_home_menu(chat_id, &lang).await;
        }
    }

    async fn handle_callback_query(&self, cb: &Value) {
        let id = cb.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let data = cb.get("data").and_then(|v| v.as_str()).unwrap_or("");
        let message = cb.get("message");
        let chat_id = message.and_then(|m| m.get("chat")).and_then(|c| c.get("id")).and_then(|v| v.as_i64()).unwrap_or(0);
        let from = cb.get("from");
        let tg_user_id = from.and_then(|f| f.get("id")).and_then(|v| v.as_i64()).unwrap_or(chat_id);

        let lang = self.get_user_language(tg_user_id);
        let _ = self.answer_callback_query(id, None).await;

        if data == "menu:home" {
            self.send_home_menu(chat_id, &lang).await;
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
        } else if data == "lang:en" {
            self.set_user_language(tg_user_id, "en");
            let _ = self.send_message(chat_id, &t("lang_changed", "en"), None).await;
            self.send_home_menu(chat_id, "en").await;
        } else if data == "lang:fa" {
            self.set_user_language(tg_user_id, "fa");
            let _ = self.send_message(chat_id, &t("lang_changed", "fa"), None).await;
            self.send_home_menu(chat_id, "fa").await;
        }
    }

    fn ensure_telegram_user(&self, tg_user_id: i64, username: &str, first_name: &str, last_name: &str) -> String {
        let conn = self.pool.get().unwrap();
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
        let conn = self.pool.get().unwrap();
        conn.query_row(
            "SELECT language FROM telegram_users WHERE telegram_user_id = ?1",
            params![tg_user_id],
            |row| row.get::<_, String>(0),
        ).unwrap_or_else(|_| "en".to_string())
    }

    fn set_user_language(&self, tg_user_id: i64, lang: &str) {
        let conn = self.pool.get().unwrap();
        let _ = conn.execute(
            "UPDATE telegram_users SET language = ?1 WHERE telegram_user_id = ?2",
            params![lang, tg_user_id],
        );
    }

    async fn send_home_menu(&self, chat_id: i64, lang: &str) {
        let text = t("welcome", lang);
        let keyboard = json!({
            "inline_keyboard": [
                [
                    { "text": format!("📊 {}", t("btn_today", lang)), "callback_data": "menu:today" },
                    { "text": format!("📅 {}", t("btn_monthly", lang)), "callback_data": "menu:monthly" }
                ],
                [
                    { "text": format!("📈 {}", t("btn_alltime", lang)), "callback_data": "menu:alltime" },
                    { "text": format!("⚖️ {}", t("btn_fair_usage", lang)), "callback_data": "menu:fair" }
                ],
                [
                    { "text": format!("⚙️ {}", t("btn_settings", lang)), "callback_data": "menu:settings" }
                ]
            ]
        });
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
        let conn = self.pool.get().unwrap();
        let token_row = conn.query_row(
            "SELECT id, peer_ids, used_at, single_use FROM telegram_signup_tokens WHERE token = ?1",
            params![token],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, bool>(3)?)),
        );

        match token_row {
            Ok((token_id, peer_ids_json, used_at, single_use)) => {
                if used_at.is_some() && single_use {
                    let _ = self.send_message(chat_id, &t("token_used", lang), None).await;
                    return;
                }

                // Get telegram_users.id
                let db_user_id = conn.query_row(
                    "SELECT id FROM telegram_users WHERE telegram_user_id = ?1",
                    params![tg_user_id],
                    |row| row.get::<_, i64>(0),
                ).unwrap_or(0);

                let peer_ids: Vec<i64> = serde_json::from_str(&peer_ids_json).unwrap_or_default();
                for pid in &peer_ids {
                    let _ = conn.execute(
                        "INSERT INTO telegram_peer_bindings (telegram_user_id, peer_id, visible) VALUES (?1, ?2, 1)
                         ON CONFLICT(telegram_user_id, peer_id) DO NOTHING",
                        params![db_user_id, pid],
                    );
                }

                let now_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
                let _ = conn.execute(
                    "UPDATE telegram_signup_tokens SET used_by = ?1, used_at = ?2 WHERE id = ?3",
                    params![db_user_id, now_str, token_id],
                );

                let welcome_msg = t("welcome_signup", lang).replace("{count}", &peer_ids.len().to_string());
                let _ = self.send_message(chat_id, &welcome_msg, None).await;
                self.send_home_menu(chat_id, lang).await;
            }
            Err(_) => {
                let _ = self.send_message(chat_id, &t("token_invalid", lang), None).await;
            }
        }
    }

    async fn send_today_usage(&self, chat_id: i64, tg_user_id: i64, lang: &str) {
        let conn = self.pool.get().unwrap();
        let peers = self.get_user_peers(&conn, tg_user_id);
        if peers.is_empty() {
            let _ = self.send_message(chat_id, &t("no_peers", lang), None).await;
            return;
        }

        let now_utc = Utc::now();
        let tz = parse_timezone("UTC");
        let day_key = counter_day_key(now_utc, tz);

        for peer in peers {
            let daily = conn.query_row(
                "SELECT rx, tx FROM usage_daily WHERE peer_id = ?1 AND day = ?2",
                params![peer.id, day_key],
                |row| Ok((row.get::<_, i64>(0).unwrap_or(0), row.get::<_, i64>(1).unwrap_or(0))),
            ).unwrap_or((0, 0));

            let title = t("today_title", lang);
            let peer_name = if !peer.name.is_empty() { &peer.name } else { &peer.interface };
            let svg = generate_usage_chart_svg(&title, peer_name, daily.0, daily.1, &[(day_key.clone(), daily.0, daily.1)]);

            if let Ok(png) = render_svg_to_png(&svg, 2.0) {
                let caption = format!("📊 <b>{}</b> - {}\n⬇️ {}\n⬆️ {}\n📈 Total: {}",
                    title, peer_name, fmt_bytes(daily.0), fmt_bytes(daily.1), fmt_bytes(daily.0 + daily.1));
                let _ = self.send_photo(chat_id, png, &caption, None).await;
            }
        }
    }

    async fn send_monthly_usage(&self, chat_id: i64, tg_user_id: i64, lang: &str) {
        let conn = self.pool.get().unwrap();
        let peers = self.get_user_peers(&conn, tg_user_id);
        if peers.is_empty() {
            let _ = self.send_message(chat_id, &t("no_peers", lang), None).await;
            return;
        }

        let now_utc = Utc::now();
        let month_key = now_utc.format("%Y-%m").to_string();

        for peer in peers {
            let monthly = conn.query_row(
                "SELECT rx, tx FROM usage_monthly WHERE peer_id = ?1 AND month_key = ?2",
                params![peer.id, month_key],
                |row| Ok((row.get::<_, i64>(0).unwrap_or(0), row.get::<_, i64>(1).unwrap_or(0))),
            ).unwrap_or((0, 0));

            let title = t("monthly_title", lang);
            let peer_name = if !peer.name.is_empty() { &peer.name } else { &peer.interface };
            let svg = generate_usage_chart_svg(&title, peer_name, monthly.0, monthly.1, &[(month_key.clone(), monthly.0, monthly.1)]);

            if let Ok(png) = render_svg_to_png(&svg, 2.0) {
                let caption = format!("📅 <b>{}</b> - {}\n⬇️ {}\n⬆️ {}\n📈 Total: {}",
                    title, peer_name, fmt_bytes(monthly.0), fmt_bytes(monthly.1), fmt_bytes(monthly.0 + monthly.1));
                let _ = self.send_photo(chat_id, png, &caption, None).await;
            }
        }
    }

    async fn send_alltime_usage(&self, chat_id: i64, tg_user_id: i64, lang: &str) {
        let conn = self.pool.get().unwrap();
        let peers = self.get_user_peers(&conn, tg_user_id);
        if peers.is_empty() {
            let _ = self.send_message(chat_id, &t("no_peers", lang), None).await;
            return;
        }

        for peer in peers {
            let totals = conn.query_row(
                "SELECT COALESCE(SUM(rx), 0), COALESCE(SUM(tx), 0) FROM usage_monthly WHERE peer_id = ?1",
                params![peer.id],
                |row| Ok((row.get::<_, i64>(0).unwrap_or(0), row.get::<_, i64>(1).unwrap_or(0))),
            ).unwrap_or((0, 0));

            let title = t("alltime_title", lang);
            let peer_name = if !peer.name.is_empty() { &peer.name } else { &peer.interface };
            let svg = generate_usage_chart_svg(&title, peer_name, totals.0, totals.1, &[("All Time".to_string(), totals.0, totals.1)]);

            if let Ok(png) = render_svg_to_png(&svg, 2.0) {
                let caption = format!("📈 <b>{}</b> - {}\n⬇️ {}\n⬆️ {}\n📈 Total: {}",
                    title, peer_name, fmt_bytes(totals.0), fmt_bytes(totals.1), fmt_bytes(totals.0 + totals.1));
                let _ = self.send_photo(chat_id, png, &caption, None).await;
            }
        }
    }

    async fn send_fair_usage(&self, chat_id: i64, tg_user_id: i64, lang: &str) {
        let conn = self.pool.get().unwrap();
        let peers = self.get_user_peers(&conn, tg_user_id);
        if peers.is_empty() {
            let _ = self.send_message(chat_id, &t("no_peers", lang), None).await;
            return;
        }

        let now_utc = Utc::now();
        let tz = parse_timezone("UTC");
        let calendar = "gregorian";

        for peer in peers {
            let dto = build_fair_usage_peer_status_dto(&conn, &peer, now_utc, tz, calendar, 1);
            let peer_name = if !peer.name.is_empty() { &peer.name } else { &peer.interface };
            let svg = generate_fair_usage_card_svg(&dto, peer_name);

            if let Ok(png) = render_svg_to_png(&svg, 2.0) {
                let status_icon = if dto.throttled { "🔴" } else { "🟢" };
                let caption = format!("{} <b>Fair Usage Policy</b> - {}\nState: {}\nDown/Up: {} / {} Kbps",
                    status_icon, peer_name, if dto.throttled { "Throttled" } else { "Normal" },
                    dto.throttle_download_kbps, dto.throttle_upload_kbps);
                let _ = self.send_photo(chat_id, png, &caption, None).await;
            }
        }
    }

    async fn send_admin_menu(&self, chat_id: i64, tg_user_id: i64, lang: &str) {
        let conn = self.pool.get().unwrap();
        let admin_chat_id = conn.query_row(
            "SELECT value FROM settings_kv WHERE key = 'tg_admin_chat_id'",
            [],
            |row| row.get::<_, String>(0),
        ).unwrap_or_default();

        if admin_chat_id.trim() != tg_user_id.to_string() && admin_chat_id.trim() != chat_id.to_string() {
            let _ = self.send_message(chat_id, &t("admin_unauthorized", lang), None).await;
            return;
        }

        let routers_count: i64 = conn.query_row("SELECT COUNT(*) FROM routers", [], |r| r.get(0)).unwrap_or(0);
        let peers_count: i64 = conn.query_row("SELECT COUNT(*) FROM peers", [], |r| r.get(0)).unwrap_or(0);
        let users_count: i64 = conn.query_row("SELECT COUNT(*) FROM telegram_users", [], |r| r.get(0)).unwrap_or(0);

        let msg = format!(
            "🛡️ <b>Admin Dashboard</b>\n\nRouters: {}\nTotal Peers: {}\nTelegram Users: {}",
            routers_count, peers_count, users_count
        );
        let _ = self.send_message(chat_id, &msg, None).await;
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
