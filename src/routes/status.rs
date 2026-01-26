use axum::{
    extract::{Path, State},
    response::{Html, Json},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::Luma;
use qrcode::QrCode;
use serde::Serialize;
use std::io::Cursor;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::models::JobStatus;
use crate::AppState;

pub async fn status_page() -> Html<String> {
    Html(include_str!("../../templates/status.html").to_string())
}

#[derive(Serialize)]
pub struct StatusUpdateResponse {
    pub success: bool,
    pub job_id: String,
    pub job_type: String,
    pub status: String,
    pub filename: Option<String>,
    pub file_size: Option<i64>,
    pub payment_address: Option<String>,
    pub required_satoshis: Option<i64>,
    pub required_bsv: Option<String>,
    pub qr_code: Option<String>,
    pub manifest_txid: Option<String>,
    pub download_link: Option<String>,
    pub message: String,
    pub progress: f64,
    pub error: Option<String>,
}

pub async fn status_update(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(job_id): Path<String>,
) -> Json<StatusUpdateResponse> {
    let state = state.read().await;

    let job = match state.db.get_job(&job_id) {
        Ok(Some(j)) => j,
        Ok(None) => {
            return Json(StatusUpdateResponse {
                success: false,
                job_id: job_id.clone(),
                job_type: "unknown".to_string(),
                status: "error".to_string(),
                filename: None,
                file_size: None,
                payment_address: None,
                required_satoshis: None,
                required_bsv: None,
                qr_code: None,
                manifest_txid: None,
                download_link: None,
                message: "Job not found".to_string(),
                progress: 0.0,
                error: Some("Job not found".to_string()),
            });
        }
        Err(e) => {
            return Json(StatusUpdateResponse {
                success: false,
                job_id: job_id.clone(),
                job_type: "unknown".to_string(),
                status: "error".to_string(),
                filename: None,
                file_size: None,
                payment_address: None,
                required_satoshis: None,
                required_bsv: None,
                qr_code: None,
                manifest_txid: None,
                download_link: None,
                message: format!("Database error: {}", e),
                progress: 0.0,
                error: Some(format!("Database error: {}", e)),
            });
        }
    };

    // Generate QR code if pending payment
    let qr_code = if job.status == JobStatus::PendingPayment {
        if let (Some(address), Some(sats)) = (&job.payment_address, job.required_satoshis) {
            generate_qr_code(address, sats as u64).ok()
        } else {
            None
        }
    } else {
        None
    };

    let required_bsv = job.required_satoshis.map(|s| format!("{:.8}", s as f64 / 100_000_000.0));

    Json(StatusUpdateResponse {
        success: true,
        job_id: job.id,
        job_type: job.job_type.as_str().to_string(),
        status: job.status.as_str().to_string(),
        filename: job.filename,
        file_size: job.file_size,
        payment_address: job.payment_address,
        required_satoshis: job.required_satoshis,
        required_bsv,
        qr_code,
        manifest_txid: job.manifest_txid,
        download_link: job.download_link,
        message: job.message,
        progress: job.progress,
        error: None,
    })
}

fn generate_qr_code(address: &str, amount_satoshis: u64) -> Result<String, String> {
    let amount_bsv = amount_satoshis as f64 / 100_000_000.0;
    let uri = format!("bitcoin:{}?sv&amount={:.8}", address, amount_bsv);

    let code = QrCode::new(uri.as_bytes()).map_err(|e| format!("QR error: {}", e))?;

    let image = code.render::<Luma<u8>>().min_dimensions(200, 200).build();

    let mut buffer = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);
    image
        .write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| format!("Image error: {}", e))?;

    Ok(format!("data:image/png;base64,{}", STANDARD.encode(&buffer)))
}
