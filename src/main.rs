pub mod accounting;
pub mod api;
pub mod backup;
pub mod calendar;
pub mod config;
pub mod crypto;
pub mod db;
pub mod fair_usage;
pub mod ops;
pub mod routeros;
pub mod scheduler;
pub mod telegram;
pub mod web;

use api::auth::AppState;
use api::build_api_router;
use axum::routing::get;
use axum::{Json, Router};
use config::AppSettings;
use db::create_pool;
use ops::ExclusiveOperationGate;
use scheduler::Scheduler;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting wgmik-server (Rust)...");

    let settings = AppSettings::from_env();
    info!("Using database: {}", settings.database_url);

    let pool = create_pool(&settings.database_url);
    let gate = ExclusiveOperationGate::new();
    let maintenance = accounting::new_maintenance_manager();
    let backup = backup::new_backup_manager();
    let tls_setup = routeros::tls_setup::new_tls_setup_manager();
    let bot = Arc::new(tokio::sync::Mutex::new(None));

    // Start background scheduler
    let scheduler = Arc::new(Scheduler::new(pool.clone(), settings.clone(), gate.clone()));
    scheduler.start();

    // Start Telegram bot if enabled
    {
        let conn = pool.get().unwrap();
        let token_enc = conn.query_row("SELECT value FROM settings_kv WHERE key = 'tg_bot_token'", [], |r| r.get::<_, String>(0)).unwrap_or_default();
        let sbox = crypto::SecretBox::new(&settings.secret_key);
        let token = sbox.decrypt(&token_enc).unwrap_or(token_enc);
        let enabled = conn.query_row("SELECT value FROM settings_kv WHERE key = 'tg_bot_enabled'", [], |r| r.get::<_, String>(0))
            .map(|v| v == "true" || v == "1").unwrap_or(false);

        if enabled && !token.trim().is_empty() {
            let tg_bot = Arc::new(telegram::TelegramBot::new(token, pool.clone(), settings.secret_key.clone()));
            let b = tg_bot.clone();
            tokio::spawn(async move {
                b.start_polling().await;
            });
            let mut lock = bot.lock().await;
            *lock = Some(tg_bot);
        }
    }

    let app_state = AppState {
        pool,
        settings: settings.clone(),
        gate,
        maintenance,
        backup,
        tls_setup,
        bot,
    };

    let api_router = build_api_router(app_state);

    let app = Router::new()
        .route("/health", get(|| async { Json(serde_json::json!({"status": "ok"})) }))
        .merge(api_router)
        .fallback(web::static_handler)
        .layer(CorsLayer::permissive());

    let addr: SocketAddr = format!("{}:{}", settings.host, settings.port)
        .parse()
        .expect("Invalid bind address");

    info!("wgmik-server listening on http://{}", addr);
    let listener = TcpListener::bind(&addr).await.expect("Failed to bind port");
    axum::serve(listener, app).await.expect("Server error");
}
