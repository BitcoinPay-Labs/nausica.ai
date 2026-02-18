use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
};
use base64::Engine;
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
    pub admin_pay: bool,
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
    let mut artist_name: Option<String> = None;
    let mut cover_data: Option<Vec<u8>> = None;
    let mut cover_filename: Option<String> = None;
    let mut lyrics: Option<String> = None;
    let mut network: String = "mainnet".to_string();
    let mut admin_pay_requested: bool = false;

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
            "artist" => {
                if let Ok(data) = field.text().await {
                    if !data.trim().is_empty() {
                        artist_name = Some(data.trim().to_string());
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
            "admin_pay" => {
                if let Ok(data) = field.text().await {
                    admin_pay_requested = data.trim().to_lowercase() == "true";
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
                                admin_pay: false,
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
                        admin_pay: false,
                        error: Some("Only FLAC, WAV, and MP3 files are supported".to_string()),
                    }),
                );
    }

    // Check if admin pay is enabled and get admin WIF
    let admin_wif = if admin_pay_requested {
        let state_read = state.read().await;
        crate::routes::admin::get_admin_wif_for_network(&state_read.db, &network)
    } else {
        None
    };
    let use_admin_pay = admin_wif.is_some();

    // Generate payment keypair based on selected network (or use admin wallet)
    let (wif, address) = if let Some(ref admin_wif_value) = admin_wif {
        let addr = BsvService::wif_to_address(admin_wif_value, &network)
            .unwrap_or_else(|_| "invalid".to_string());
        (admin_wif_value.clone(), addr)
    } else {
        BsvService::generate_keypair(&network)
    };

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

    // If admin pay is enabled, start processing immediately
    let (initial_status, initial_message) = if use_admin_pay {
        (JobStatus::Processing, "Admin pay enabled, starting upload...".to_string())
    } else {
        (JobStatus::PendingPayment, "Waiting for payment...".to_string())
    };

    let job = Job {
        id: job_id.clone(),
        job_type: JobType::FlacUpload,
        status: initial_status,
        filename: Some(filename),
        file_size: Some(file_data.len() as i64),
        file_data: Some(file_data),
        payment_address: Some(address.clone()),
        payment_wif: Some(wif),
        required_satoshis: Some(required_satoshis),
        manifest_txid: None,
        download_link: None,
        progress: 0.0,
        message: initial_message,
        created_at: now,
        updated_at: now,
        track_title,
        artist_name,
        cover_txid: None, // Will be set after cover image is uploaded to BSV
        cover_data,
        lyrics,
        network: Some(network.clone()),
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
                    admin_pay: false,
                    error: Some(format!("Failed to create job: {}", e)),
                }),
            );
        }
    }

        // If admin pay is enabled, start processing immediately
        if use_admin_pay {
            let state_clone = state.clone();
            let job_id_clone = job_id.clone();
            let address_clone = address.clone();
            let network_clone = network.clone();
            tokio::spawn(async move {
                crate::process_job(
                    state_clone, 
                    job_id_clone, 
                    crate::models::job::JobType::FlacUpload,
                    address_clone,
                    network_clone
                ).await;
            });
        }

    (
        StatusCode::OK,
        Json(FlacUploadResponse {
            success: true,
            job_id: Some(job_id),
            payment_address: if use_admin_pay { None } else { Some(address) },
            required_satoshis: if use_admin_pay { None } else { Some(required_satoshis) },
            admin_pay: use_admin_pay,
            error: None,
        }),
    )
}

#[derive(Deserialize)]
pub struct FlacDownloadRequest {
    pub txid: String,
    pub network: Option<String>,
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
    let network = req.network.unwrap_or_else(|| "mainnet".to_string());

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
        artist_name: None,
        cover_txid: None,
        cover_data: None,
        lyrics: None,
        network: Some(network.clone()),
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
    let network_clone = network.clone();
    tokio::spawn(async move {
        crate::process_flac_download(state_clone, job_id_clone, Some(txid), network_clone).await;
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
    pub artist_name: Option<String>,
    pub cover_txid: Option<String>,
    pub lyrics: Option<String>,
}

/// Get cover image from BSV transaction
#[derive(Deserialize)]
pub struct CoverRequest {
    pub txid: String,
    pub network: Option<String>,
}

#[derive(Serialize)]
pub struct CoverResponse {
    pub success: bool,
    pub data: Option<String>,  // Base64 encoded image data
    pub content_type: Option<String>,
    pub error: Option<String>,
}

pub async fn get_cover_image(
    State(state): State<Arc<RwLock<AppState>>>,
    Json(req): Json<CoverRequest>,
) -> impl IntoResponse {
    let txid = req.txid.trim().to_string();
    let network = req.network.unwrap_or_else(|| "mainnet".to_string());

    if txid.len() != 64 {
        return Json(CoverResponse {
            success: false,
            data: None,
            content_type: None,
            error: Some("Invalid TXID format".to_string()),
        });
    }

    // Fetch transaction from blockchain
    let tx_data = crate::fetch_tx_raw(&state, &txid, &network).await;
    
    match tx_data {
        Ok(tx_hex) => {
            // Extract image data from transaction
            if let Some(image_data) = extract_image_from_tx(&tx_hex) {
                let base64_data = base64::engine::general_purpose::STANDARD.encode(&image_data);
                
                // Detect content type from magic bytes
                let content_type = detect_image_type(&image_data);
                
                Json(CoverResponse {
                    success: true,
                    data: Some(base64_data),
                    content_type: Some(content_type),
                    error: None,
                })
            } else {
                Json(CoverResponse {
                    success: false,
                    data: None,
                    content_type: None,
                    error: Some("No image data found in transaction".to_string()),
                })
            }
        }
        Err(e) => {
            Json(CoverResponse {
                success: false,
                data: None,
                content_type: None,
                error: Some(format!("Failed to fetch transaction: {}", e)),
            })
        }
    }
}

fn extract_image_from_tx(tx_hex: &str) -> Option<Vec<u8>> {
    let tx_bytes = hex::decode(tx_hex).ok()?;
    
    let mut i = 0;
    i += 4; // version
    
    let (input_count, varint_size) = crate::read_varint(&tx_bytes[i..])?;
    i += varint_size;
    
    for _ in 0..input_count {
        i += 32;
        i += 4;
        let (script_len, vs) = crate::read_varint(&tx_bytes[i..])?;
        i += vs;
        i += script_len as usize;
        i += 4;
    }
    
    let (output_count, varint_size) = crate::read_varint(&tx_bytes[i..])?;
    i += varint_size;
    
    for _ in 0..output_count {
        i += 8; // value
        let (script_len, vs) = crate::read_varint(&tx_bytes[i..])?;
        i += vs;
        
        if i + script_len as usize > tx_bytes.len() {
            break;
        }
        
        let script = &tx_bytes[i..i + script_len as usize];
        i += script_len as usize;
        
        // Check for OP_FALSE OP_IF (0x00 0x63) - our cover image format
        if script.len() > 4 && script[0] == 0x00 && script[1] == 0x63 {
            if let Some(data) = parse_coverart_script(&script[2..]) {
                return Some(data);
            }
        }
        
        // Also check for OP_RETURN (0x6a) or OP_FALSE OP_RETURN (0x00 0x6a)
        if script.len() > 2 && (script[0] == 0x6a || (script[0] == 0x00 && script[1] == 0x6a)) {
            let start = if script[0] == 0x6a { 1 } else { 2 };
            if let Some(data) = parse_image_script(&script[start..]) {
                return Some(data);
            }
        }
    }
    
    None
}

/// Parse cover art script in OP_FALSE OP_IF "coverart" <data chunks> OP_ENDIF format
fn parse_coverart_script(script: &[u8]) -> Option<Vec<u8>> {
    let mut i = 0;
    
    // First push should be "coverart" protocol identifier
    if let Some((data, size)) = crate::read_push_data(&script[i..]) {
        let data_str = String::from_utf8_lossy(&data);
        if data_str == "coverart" {
            i += size;
        } else {
            return None; // Not a coverart script
        }
    } else {
        return None;
    }
    
    // Read all image data chunks until OP_ENDIF (0x68)
    let mut image_data = Vec::new();
    while i < script.len() {
        // Check for OP_ENDIF
        if script[i] == 0x68 {
            break;
        }
        
        if let Some((chunk, size)) = crate::read_push_data(&script[i..]) {
            image_data.extend_from_slice(&chunk);
            i += size;
        } else {
            break;
        }
    }
    
    if !image_data.is_empty() {
        Some(image_data)
    } else {
        None
    }
}

fn parse_image_script(script: &[u8]) -> Option<Vec<u8>> {
    let mut i = 0;
    
    // Skip protocol identifier if present
    if let Some((data, size)) = crate::read_push_data(&script[i..]) {
        let data_str = String::from_utf8_lossy(&data);
        if data_str == "NAUSICA_COVER" || data_str == "19HxigV4QyBv3tHpQVcUEQyq1pzZVdoAut" {
            i += size;
        }
    }
    
    // Read image data
    if i < script.len() {
        if let Some((data, _)) = crate::read_push_data(&script[i..]) {
            // Check if it looks like image data (PNG, JPEG, etc.)
            if data.len() > 4 {
                return Some(data);
            }
        }
    }
    
    None
}

fn detect_image_type(data: &[u8]) -> String {
    if data.len() >= 8 {
        // PNG: 89 50 4E 47 0D 0A 1A 0A
        if data[0..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
            return "image/png".to_string();
        }
        // JPEG: FF D8 FF
        if data[0..3] == [0xFF, 0xD8, 0xFF] {
            return "image/jpeg".to_string();
        }
        // GIF: 47 49 46 38
        if data[0..4] == [0x47, 0x49, 0x46, 0x38] {
            return "image/gif".to_string();
        }
        // WebP: 52 49 46 46 ... 57 45 42 50
        if data[0..4] == [0x52, 0x49, 0x46, 0x46] && data.len() >= 12 && data[8..12] == [0x57, 0x45, 0x42, 0x50] {
            return "image/webp".to_string();
        }
    }
    "image/png".to_string() // Default
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
                artist_name: job.artist_name,
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
            artist_name: None,
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
            artist_name: None,
            cover_txid: None,
            lyrics: None,
        }),
    }
}
