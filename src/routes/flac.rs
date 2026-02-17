use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::models::{Job, JobStatus, JobType};
use crate::services::bsv::BsvService;
use crate::AppState;

/// FLAC upload page
pub async fn flac_upload_page() -> Html<String> {
    let html = include_str!("../../templates/flac_upload.html");
    Html(html.to_string())
}

/// FLAC player page (download + playback)
pub async fn flac_player_page() -> Html<String> {
    let html = include_str!("../../templates/flac_player.html");
    Html(html.to_string())
}

/// FLAC status page
pub async fn flac_status_page(Path(job_id): Path<String>) -> Html<String> {
    let html = include_str!("../../templates/flac_status.html");
    let html = html.replace("{{JOB_ID}}", &job_id);
    Html(html)
}

#[derive(Serialize)]
pub struct FlacUploadResponse {
    pub success: bool,
    pub job_id: Option<String>,
    pub payment_address: Option<String>,
    pub required_satoshis: Option<i64>,
    pub error: Option<String>,
}

/// Prepare FLAC upload - creates job and returns payment address
pub async fn prepare_flac_upload(
    State(state): State<Arc<RwLock<AppState>>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut filename: Option<String> = None;
    let mut file_data: Option<Vec<u8>> = None;
    let mut track_title: Option<String> = None;
    let mut cover_data: Option<Vec<u8>> = None;
    let mut cover_filename: Option<String> = None;
    let mut lyrics: Option<String> = None;
    let mut network: String = "mainnet".to_string();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "file" => {
                filename = field.file_name().map(|s| s.to_string());
                if let Ok(data) = field.bytes().await {
                    file_data = Some(data.to_vec());
                }
            }
            "title" => {
                if let Ok(data) = field.text().await {
                    if !data.trim().is_empty() {
                        track_title = Some(data.trim().to_string());
                    }
                }
            }
            "cover" => {
                cover_filename = field.file_name().map(|s| s.to_string());
                if let Ok(data) = field.bytes().await {
                    if !data.is_empty() {
                        cover_data = Some(data.to_vec());
                    }
                }
            }
            "lyrics" => {
                if let Ok(data) = field.text().await {
                    if !data.trim().is_empty() {
                        lyrics = Some(data.trim().to_string());
                    }
                }
            }
            "network" => {
                if let Ok(data) = field.text().await {
                    let net = data.trim().to_lowercase();
                    if net == "testnet" {
                        network = "testnet".to_string();
                    }
                }
            }
            _ => {}
        }
    }

    let file_data = match file_data {
        Some(data) => data,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(FlacUploadResponse {
                    success: false,
                    job_id: None,
                    payment_address: None,
                    required_satoshis: None,
                    error: Some("No file provided".to_string()),
                }),
            );
        }
    };

    let filename = filename.unwrap_or_else(|| "audio.flac".to_string());

    // Validate audio file (FLAC, WAV, or MP3)
    let lower_filename = filename.to_lowercase();
    if !lower_filename.ends_with(".flac") && !lower_filename.ends_with(".wav") && !lower_filename.ends_with(".mp3") {
        return (
            StatusCode::BAD_REQUEST,
            Json(FlacUploadResponse {
                success: false,
                job_id: None,
                payment_address: None,
                required_satoshis: None,
                error: Some("Only FLAC, WAV, and MP3 files are supported".to_string()),
            }),
        );
    }

    // Generate payment keypair based on selected network
    let (wif, address) = BsvService::generate_keypair(&network);

    // Calculate required satoshis
    // For large files, we need to account for UTXO splitting and multiple chunk transactions
    let max_chunk_size = 1024 * 1024; // 1MB chunks
    let file_size = file_data.len();
    
    let required_satoshis = {
        let state = state.read().await;
        if file_size > max_chunk_size {
            // Multi-chunk upload: use calculate_multi_chunk_cost
            let (total, _, _) = state.bsv.calculate_multi_chunk_cost(file_size, max_chunk_size);
            // Add 20% buffer for safety
            (total as f64 * 1.2).ceil() as i64
        } else {
            // Single transaction upload
            state.bsv.calculate_upload_cost(file_size)
        }
    };

    // Create job
    let job_id = uuid::Uuid::new_v4().to_string().replace("-", "");
    let now = chrono::Utc::now();

    // Store cover data temporarily for later upload (will be uploaded when payment is confirmed)
    // For now, we store cover_data in a separate field or handle it during processing
    let _ = cover_data; // Will be used in future for BSV upload
    let _ = cover_filename; // Will be used in future for BSV upload

    let job = Job {
        id: job_id.clone(),
        job_type: JobType::FlacUpload,
        status: JobStatus::PendingPayment,
        filename: Some(filename),
        file_size: Some(file_data.len() as i64),
        file_data: Some(file_data),
        payment_address: Some(address.clone()),
        payment_wif: Some(wif),
        required_satoshis: Some(required_satoshis),
        manifest_txid: None,
        download_link: None,
        progress: 0.0,
        message: "Waiting for payment...".to_string(),
        created_at: now,
        updated_at: now,
        track_title,
        cover_txid: None, // Will be set after cover image is uploaded to BSV
        lyrics,
        network: Some(network),
    };

    {
        let state = state.read().await;
        if let Err(e) = state.db.insert_job(&job) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(FlacUploadResponse {
                    success: false,
                    job_id: None,
                    payment_address: None,
                    required_satoshis: None,
                    error: Some(format!("Failed to create job: {}", e)),
                }),
            );
        }
    }

    (
        StatusCode::OK,
        Json(FlacUploadResponse {
            success: true,
            job_id: Some(job_id),
            payment_address: Some(address),
            required_satoshis: Some(required_satoshis),
            error: None,
        }),
    )
}

#[derive(Deserialize)]
pub struct FlacDownloadRequest {
    pub txid: String,
}

#[derive(Serialize)]
pub struct FlacDownloadResponse {
    pub success: bool,
    pub job_id: Option<String>,
    pub error: Option<String>,
}

/// Start FLAC download
pub async fn start_flac_download(
    State(state): State<Arc<RwLock<AppState>>>,
    Json(req): Json<FlacDownloadRequest>,
) -> impl IntoResponse {
    let txid = req.txid.trim().to_string();

    if txid.len() != 64 {
        return (
            StatusCode::BAD_REQUEST,
            Json(FlacDownloadResponse {
                success: false,
                job_id: None,
                error: Some("Invalid TXID format (must be 64 characters)".to_string()),
            }),
        );
    }

    // Create download job
    let job_id = uuid::Uuid::new_v4().to_string().replace("-", "");
    let now = chrono::Utc::now();

    let job = Job {
        id: job_id.clone(),
        job_type: JobType::FlacDownload,
        status: JobStatus::Processing,
        filename: None,
        file_size: None,
        file_data: None,
        payment_address: None,
        payment_wif: None,
        required_satoshis: None,
        manifest_txid: Some(txid.clone()),
        download_link: None,
        progress: 0.0,
        message: "Starting FLAC download...".to_string(),
        created_at: now,
        updated_at: now,
        track_title: None,
        cover_txid: None,
        lyrics: None,
        network: None,
    };

    {
        let state_read = state.read().await;
        if let Err(e) = state_read.db.insert_job(&job) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(FlacDownloadResponse {
                    success: false,
                    job_id: None,
                    error: Some(format!("Failed to create job: {}", e)),
                }),
            );
        }
    }

    // Start download process
    let state_clone = state.clone();
    let job_id_clone = job_id.clone();
    tokio::spawn(async move {
        crate::process_flac_download(state_clone, job_id_clone, Some(txid)).await;
    });

    (
        StatusCode::OK,
        Json(FlacDownloadResponse {
            success: true,
            job_id: Some(job_id),
            error: None,
        }),
    )
}

#[derive(Serialize)]
pub struct FlacStatusResponse {
    pub status: String,
    pub progress: f64,
    pub message: String,
    pub txid: Option<String>,
    pub download_link: Option<String>,
    pub filename: Option<String>,
    pub track_title: Option<String>,
    pub cover_txid: Option<String>,
    pub lyrics: Option<String>,
}

/// Get FLAC job status
pub async fn get_flac_status(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(job_id): Path<String>,
) -> impl IntoResponse {
    let state = state.read().await;

    match state.db.get_job(&job_id) {
        Ok(Some(job)) => {
            let status = match job.status {
                JobStatus::PendingPayment => "pending_payment",
                JobStatus::Processing => "processing",
                JobStatus::Complete => "complete",
                JobStatus::Error => "error",
            };

            Json(FlacStatusResponse {
                status: status.to_string(),
                progress: job.progress,
                message: job.message,
                txid: job.manifest_txid,
                download_link: job.download_link,
                filename: job.filename,
                track_title: job.track_title,
                cover_txid: job.cover_txid,
                lyrics: job.lyrics,
            })
        }
        Ok(None) => Json(FlacStatusResponse {
            status: "not_found".to_string(),
            progress: 0.0,
            message: "Job not found".to_string(),
            txid: None,
            download_link: None,
            filename: None,
            track_title: None,
            cover_txid: None,
            lyrics: None,
        }),
        Err(e) => Json(FlacStatusResponse {
            status: "error".to_string(),
            progress: 0.0,
            message: format!("Database error: {}", e),
            txid: None,
            download_link: None,
            filename: None,
            track_title: None,
            cover_txid: None,
            lyrics: None,
        }),
    }
}
