mod config;
mod db;
mod models;
mod routes;
mod services;

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::services::ServeDir;
use tracing_subscriber;

use crate::config::Config;
use crate::db::Database;
use crate::services::bitails::BitailsClient;
use crate::services::bsv::BsvService;

pub struct AppState {
    pub db: Database,
    pub config: Config,
    pub bitails: BitailsClient,
    pub bsv: BsvService,
}

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Load configuration
    dotenvy::dotenv().ok();
    let config = Config::from_env();

    // Initialize database
    let db = Database::new(&config.database_path).expect("Failed to initialize database");

    // Initialize Bitails client
    let bitails = BitailsClient::new(
        config.bitails_api_url.clone(),
        config.bitails_api_key.clone(),
    );

    // Initialize BSV service
    let bsv = BsvService::new(config.bsv_private_key.clone(), config.bsv_fee_rate);

    // Create shared state
    let state = Arc::new(RwLock::new(AppState {
        db,
        config: config.clone(),
        bitails,
        bsv,
    }));

    // Spawn background payment watcher
    let watcher_state = state.clone();
    tokio::spawn(async move {
        services::job::payment_watcher(watcher_state).await;
    });

    // Build router
    let app = Router::new()
        // Pages
        .route("/", get(routes::dashboard::dashboard_page))
        .route("/upload", get(routes::upload::upload_page))
        .route("/download", get(routes::download::download_page))
        .route("/status/:job_id", get(routes::status::status_page))
        // FLAC pages
        .route("/flac", get(routes::flac::flac_upload_page))
        .route("/flac/upload", get(routes::flac::flac_upload_page))
        .route("/flac/player", get(routes::flac::flac_player_page))
        .route("/flac/status/:job_id", get(routes::flac::flac_status_page))
        // API endpoints
        .route("/prepare_upload", post(routes::upload::prepare_upload))
        .route("/start_download", post(routes::download::start_download))
        .route("/status_update/:job_id", get(routes::status::status_update))
        .route("/api/jobs", get(routes::dashboard::get_jobs))
        // FLAC API endpoints
        .route("/api/flac/upload", post(routes::flac::prepare_flac_upload))
        .route("/api/flac/download", post(routes::flac::start_flac_download))
        .route("/api/flac/status/:job_id", get(routes::flac::get_flac_status))
        // Static files and downloads
        .nest_service("/static", ServeDir::new("static"))
        .nest_service("/downloads", ServeDir::new("./data/downloads"))
        .with_state(state);

    // Start server
    let addr = format!("{}:{}", config.host, config.port);
    tracing::info!("Starting server on {}", addr);
    
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
