pub mod admin;
pub mod auth;
pub mod dashboard;
pub mod fair_usage;
pub mod peers;
pub mod routers;
pub mod settings;
pub mod summary;
pub mod telegram;
pub mod users;

pub use auth::AppState;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;

pub fn build_api_router(state: AppState) -> Router {
    Router::new()
        // Auth
        .route("/api/auth/login", post(auth::auth_login))
        .route("/api/auth/logout", post(auth::auth_logout))
        .route("/api/auth/setup-state", get(auth::auth_setup_state))
        .route("/api/auth/setup", post(auth::auth_setup))
        .route("/api/auth/me", get(auth::auth_me))
        .route("/api/auth/bootstrap", get(auth::auth_bootstrap))
        .route("/api/auth/change-password", post(auth::auth_change_password))
        // Users
        .route("/api/users", get(users::list_users).post(users::create_user))
        .route("/api/users/:id", patch(users::update_user).delete(users::delete_user))
        .route("/api/users/:id/reset-password", post(users::reset_user_password))
        // Settings & Metrics
        .route("/api/settings", get(settings::get_settings).put(settings::update_settings))
        .route("/api/metrics", get(settings::get_metrics))
        // Routers
        .route("/api/routers", get(routers::list_routers).post(routers::create_router))
        .route("/api/routers/:id", get(routers::get_router).put(routers::update_router).delete(routers::delete_router))
        .route("/api/routers/:id/delete-impact", get(routers::delete_router_impact))
        .route("/api/routers/:id/test", post(routers::test_router))
        .route("/api/routers/:id/interfaces", get(routers::list_router_interfaces))
        .route("/api/routers/:id/interfaces/:iface", get(routers::get_router_interface))
        .route("/api/routers/:id/peers", get(routers::list_router_peers))
        .route("/api/routers/:id/peers/import", post(routers::import_router_peers))
        .route("/api/routers/:id/peers/add", post(routers::add_router_peer))
        // Peers
        .route("/api/peers", get(peers::list_peers))
        .route("/api/peers/:id", get(peers::get_peer).patch(peers::patch_peer).delete(peers::delete_peer))
        .route("/api/peers/:id/usage", get(peers::get_peer_usage))
        .route("/api/peers/:id/reset_metrics", post(peers::reset_peer_metrics))
        .route("/api/peers/:id/reconcile", post(peers::reconcile_peer))
        .route("/api/peers/:id/client_private_key", get(peers::get_peer_client_private_key))
        .route("/api/peers/:id/client_export_prefs", get(peers::get_peer_client_export_prefs))
        .route("/api/peers/:id/renew_keys", post(peers::renew_peer_keys))
        .route("/api/peers/:id/router-sync/resolve", post(peers::resolve_peer_sync))
        .route("/api/peers/:id/actions", get(peers::get_peer_actions))
        .route("/api/peers/:id/quota", get(peers::get_peer_quota).patch(peers::patch_peer_quota))
        // Dashboard
        .route("/api/dashboard/live_status", get(dashboard::get_dashboard_live_status))
        .route("/api/dashboard/metrics", get(dashboard::get_dashboard_metrics))
        .route("/api/actions/last", get(dashboard::get_last_actions))
        // Summary
        .route("/api/summary/month", get(summary::get_summary_month))
        .route("/api/summary/month/by_router", get(summary::get_summary_month_by_router))
        .route("/api/summary/peers", get(summary::get_summary_peers))
        .route("/api/summary/raw", get(summary::get_summary_raw))
        .route("/api/summary/raw/by_router", get(summary::get_summary_raw_by_router))
        // Fair Usage
        .route("/api/fair-usage/rules", get(fair_usage::list_fair_usage_rules).post(fair_usage::create_fair_usage_rule))
        .route("/api/fair-usage/rules/:id", get(fair_usage::get_fair_usage_rule).put(fair_usage::update_fair_usage_rule).delete(fair_usage::delete_fair_usage_rule))
        .route("/api/fair-usage/rules/:id/assign", post(fair_usage::assign_fair_usage_rule))
        .route("/api/fair-usage/rules/:id/assign/:peer_id", delete(fair_usage::unassign_fair_usage_rule))
        .route("/api/fair-usage/peers/:id/status", get(fair_usage::get_peer_fair_usage_status))
        .route("/api/fair-usage/peers/:id/reset", post(fair_usage::reset_peer_fair_usage))
        // Telegram
        .route("/api/telegram/config", get(telegram::get_telegram_config).put(telegram::update_telegram_config))
        .route("/api/telegram/status", get(telegram::get_telegram_status))
        .route("/api/telegram/restart", post(telegram::restart_telegram_bot))
        .route("/api/telegram/tokens", get(telegram::list_telegram_tokens).post(telegram::create_telegram_token))
        .route("/api/telegram/tokens/:id", delete(telegram::delete_telegram_token))
        .route("/api/telegram/users", get(telegram::list_telegram_users))
        .route("/api/telegram/users/:id", delete(telegram::delete_telegram_user).patch(telegram::patch_telegram_user))
        .route("/api/telegram/users/:id/peers", put(telegram::set_telegram_user_peers))
        .route("/api/telegram/notifications", get(telegram::get_telegram_notifications).put(telegram::update_telegram_notifications))
        .route("/api/telegram/broadcasts", get(telegram::list_telegram_broadcasts).post(telegram::create_telegram_broadcast))
        .route("/api/telegram/broadcasts/:id", get(telegram::get_telegram_broadcast))
        .route("/api/telegram/broadcasts/:id/retry-failed", post(telegram::retry_failed_telegram_broadcast))
        .route("/api/telegram/test-notify", post(telegram::test_telegram_notify))
        .route("/api/telegram/test-notify/:event", post(telegram::test_telegram_notify_event))
        // Admin
        .route("/api/admin/usage_maintenance", get(admin::get_usage_maintenance))
        .route("/api/admin/usage_maintenance/run", post(admin::run_usage_maintenance))
        .route("/api/admin/usage_maintenance/cancel", post(admin::cancel_usage_maintenance))
        .route("/api/admin/backup", get(admin::get_backup_status))
        .route("/api/admin/backup/run", post(admin::run_backup))
        .route("/api/admin/purge_usage", post(admin::purge_usage))
        .route("/api/admin/purge_peers", post(admin::purge_peers))
        .with_state(state)
}
