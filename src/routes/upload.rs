use axum::{
    extract::{Multipart, State},
    response::{Html, Json},
};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::models::Job;
use crate::services::bsv::BsvService;
use crate::AppState;

pub async fn upload_page() -> Html<String> {
    Html(include_str!("../../templates/upload.html").to_string())
}

#[derive(Serialize)]
pub struct PrepareUploadResponse {
    pub success: bool,
    pub job_id: Option<String>,
    pub redirect_url: Option<String>,
    pub error: Option<String>,
}

pub async fn prepare_upload(
    State(state): State<Arc<RwLock<AppState>>>,
    mut multipart: Multipart,
) -> Json<PrepareUploadResponse> {
    let mut filename: Option<String> = None;
    let mut file_data: Option<Vec<u8>> = None;

    // Parse multipart form
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        if name == "file" {
            filename = field.file_name().map(|s: &str| s.to_string());
            match field.bytes().await {
                Ok(bytes) => file_data = Some(bytes.to_vec()),
                Err(e) => {
                    return Json(PrepareUploadResponse {
                        success: false,
                        job_id: None,
                        redirect_url: None,
                        error: Some(format!("Failed to read file: {}", e)),
                    });
                }
            }
        }
    }

    let filename = match filename {
        Some(f) => f,
        None => {
            return Json(PrepareUploadResponse {
                success: false,
                job_id: None,
                redirect_url: None,
                error: Some("No file provided".to_string()),
            });
        }
    };

    let file_data = match file_data {
        Some(d) => d,
        None => {
            return Json(PrepareUploadResponse {
                success: false,
                job_id: None,
                redirect_url: None,
                error: Some("No file data".to_string()),
            });
        }
    };

    let file_size = file_data.len() as i64;

    // Generate new keypair for payment (mainnet for production)
    let (wif, address) = BsvService::generate_keypair("mainnet");

    // Calculate required payment
    let required_satoshis = {
        let state = state.read().await;
        state.bsv.calculate_upload_cost(file_data.len())
    };

    // Create job
    let job_id = Uuid::new_v4().to_string().replace("-", "");
    let job = Job::new_upload(
        job_id.clone(),
        filename,
        file_size,
        file_data,
        address,
        wif,
        required_satoshis,
    );

    // Save job to database
    {
        let state = state.read().await;
        if let Err(e) = state.db.insert_job(&job) {
            return Json(PrepareUploadResponse {
                success: false,
                job_id: None,
                redirect_url: None,
                error: Some(format!("Failed to create job: {}", e)),
            });
        }
    }

    Json(PrepareUploadResponse {
        success: true,
        job_id: Some(job_id.clone()),
        redirect_url: Some(format!("/status/{}", job_id)),
        error: None,
    })
}
