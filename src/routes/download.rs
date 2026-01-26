use axum::{
    extract::State,
    response::{Html, Json},
    Form,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::models::Job;
use crate::AppState;

pub async fn download_page() -> Html<String> {
    Html(include_str!("../../templates/download.html").to_string())
}

#[derive(Deserialize)]
pub struct StartDownloadInput {
    pub txid: String,
}

#[derive(Serialize)]
pub struct StartDownloadResponse {
    pub success: bool,
    pub job_id: Option<String>,
    pub redirect_url: Option<String>,
    pub error: Option<String>,
}

pub async fn start_download(
    State(state): State<Arc<RwLock<AppState>>>,
    Form(input): Form<StartDownloadInput>,
) -> Json<StartDownloadResponse> {
    let txid = input.txid.trim().to_string();

    // Validate TXID format (64 hex characters)
    if txid.len() != 64 || !txid.chars().all(|c| c.is_ascii_hexdigit()) {
        return Json(StartDownloadResponse {
            success: false,
            job_id: None,
            redirect_url: None,
            error: Some("Invalid TXID format. Must be 64 hex characters.".to_string()),
        });
    }

    // Create job
    let job_id = Uuid::new_v4().to_string().replace("-", "");
    let job = Job::new_download(job_id.clone(), txid.clone());

    // Save job to database
    {
        let state_guard = state.read().await;
        if let Err(e) = state_guard.db.insert_job(&job) {
            return Json(StartDownloadResponse {
                success: false,
                job_id: None,
                redirect_url: None,
                error: Some(format!("Failed to create job: {}", e)),
            });
        }
    }

    // Start download process in background
    let state_clone = state.clone();
    let job_id_clone = job_id.clone();
    tokio::spawn(async move {
        crate::process_download(state_clone, job_id_clone, Some(txid)).await;
    });

    Json(StartDownloadResponse {
        success: true,
        job_id: Some(job_id.clone()),
        redirect_url: Some(format!("/status/{}", job_id)),
        error: None,
    })
}
