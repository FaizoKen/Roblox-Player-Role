#![recursion_limit = "512"]

use std::sync::Arc;

use axum::routing::{delete, get, post};
use axum::Router;
use sqlx::PgPool;
use tokio::sync::mpsc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

mod config;
mod db;
mod error;
mod models;
mod routes;
mod schema;
mod services;
mod tasks;

use routes::ingest::IngestLimiterTable;
use services::roblox_api::RobloxApiClient;
use services::roblox_oauth::RobloxOAuthClient;
use services::rolelogic::RoleLogicClient;
use services::sync::{ConfigSyncEvent, PlayerSyncEvent};

pub struct AppState {
    pub pool: PgPool,
    pub config: config::AppConfig,
    pub player_sync_tx: mpsc::Sender<PlayerSyncEvent>,
    pub config_sync_tx: mpsc::Sender<ConfigSyncEvent>,
    pub roblox_client: RobloxApiClient,
    pub roblox_oauth: RobloxOAuthClient,
    pub rl_client: RoleLogicClient,
    pub http: reqwest::Client,
    pub encryption_key: [u8; 32],
    pub ingest_limiters: IngestLimiterTable,
    pub verify_html: bytes::Bytes,
    pub players_html: bytes::Bytes,
    pub games_html: bytes::Bytes,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "roblox_player_role=info,tower_http=info".into()),
        )
        .init();

    let app_config = config::AppConfig::from_env();
    let listen_addr = app_config.listen_addr.clone();

    let pool = db::create_pool(&app_config.database_url).await;
    db::run_migrations(&pool).await;
    tracing::info!("Database connected and migrations applied");

    // 4096 / 256 channel sizes per BLUEPRINT scaling guidance
    let (player_sync_tx, player_sync_rx) = mpsc::channel::<PlayerSyncEvent>(4096);
    let (config_sync_tx, config_sync_rx) = mpsc::channel::<ConfigSyncEvent>(256);

    let roblox_client = RobloxApiClient::new(app_config.roblox_api_rate_limit);
    let redirect_uri = format!("{}/verify/callback", app_config.base_url);
    let roblox_oauth = RobloxOAuthClient::new(
        app_config.roblox_client_id.clone(),
        app_config.roblox_client_secret.clone(),
        redirect_uri,
    );
    let rl_client = RoleLogicClient::new();
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("Failed to build HTTP client");
    let encryption_key = services::crypto::parse_key(&app_config.token_encryption_key_hex)
        .expect("TOKEN_ENCRYPTION_KEY invalid");

    let verify_html = bytes::Bytes::from(routes::verification::render_verify_page(&app_config.base_url));
    let players_html = bytes::Bytes::from(routes::players::render_players_page(&app_config.base_url));
    let games_html = bytes::Bytes::from(routes::games::render_games_page(&app_config.base_url));

    let state = Arc::new(AppState {
        pool,
        config: app_config,
        player_sync_tx,
        config_sync_tx,
        roblox_client,
        roblox_oauth,
        rl_client,
        http,
        encryption_key,
        ingest_limiters: IngestLimiterTable::new(),
        verify_html,
        players_html,
        games_html,
    });

    tokio::spawn(tasks::refresh_worker::run(Arc::clone(&state)));
    tokio::spawn(tasks::player_sync_worker::run(player_sync_rx, Arc::clone(&state)));
    tokio::spawn(tasks::config_sync_worker::run(config_sync_rx, Arc::clone(&state)));
    tokio::spawn(tasks::opencloud_poll_worker::run(Arc::clone(&state)));
    tokio::spawn(tasks::cleanup_expired(Arc::clone(&state)));

    let app = Router::new()
        .nest(
            "/roblox-player-role",
            Router::new()
                // RoleLogic plugin contract
                .route("/register", post(routes::plugin::register))
                .route("/config", get(routes::plugin::get_config))
                .route("/config", post(routes::plugin::post_config))
                .route("/config", delete(routes::plugin::delete_config))
                // Verification (user-facing)
                .route("/verify", get(routes::verification::verify_page))
                .route("/verify/login", get(routes::verification::login))
                .route("/verify/status", get(routes::verification::status))
                .route("/verify/roblox", get(routes::verification::roblox_login))
                .route("/verify/callback", get(routes::verification::callback))
                .route("/verify/unlink", post(routes::verification::unlink))
                .route("/verify/logout", post(routes::verification::logout))
                // Player list
                .route("/players/{guild_id}", get(routes::players::players_page))
                .route("/players/{guild_id}/data", get(routes::players::players_data))
                // Game-creator admin UI
                .route("/games", get(routes::games::games_page))
                .route("/games/data", get(routes::games::games_data))
                .route("/games", post(routes::games::create_universe))
                .route(
                    "/games/{universe_id}/regenerate-secret",
                    post(routes::games::regenerate_secret),
                )
                .route(
                    "/games/{universe_id}/open-cloud",
                    post(routes::games::save_open_cloud),
                )
                .route("/games/{universe_id}/delete", post(routes::games::delete_universe))
                // Game ingest webhook
                .route("/ingest/{universe_id}/stats", post(routes::ingest::ingest_stats))
                // Studio plugin download
                .route(
                    "/studio-plugin/Roblox-Player-Role.rbxm",
                    get(routes::health::studio_plugin_rbxm),
                )
                // Health & favicon
                .route("/favicon.ico", get(routes::health::favicon))
                .route("/health", get(routes::health::health)),
        )
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state);

    tracing::info!("Server starting on {listen_addr}");

    let listener = tokio::net::TcpListener::bind(&listen_addr)
        .await
        .expect("Failed to bind listener");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("Shutdown signal received, draining connections...");
        })
        .await
        .expect("Server error");

    tracing::info!("Server stopped");
}
