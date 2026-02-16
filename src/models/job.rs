use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JobType {
    Upload,
    Download,
    FlacUpload,
    FlacDownload,
}

impl JobType {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobType::Upload => "upload",
            JobType::Download => "download",
            JobType::FlacUpload => "flac_upload",
            JobType::FlacDownload => "flac_download",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "upload" => Some(JobType::Upload),
            "download" => Some(JobType::Download),
            "flac_upload" => Some(JobType::FlacUpload),
            "flac_download" => Some(JobType::FlacDownload),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    PendingPayment,
    Processing,
    Complete,
    Error,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::PendingPayment => "pending_payment",
            JobStatus::Processing => "processing",
            JobStatus::Complete => "complete",
            JobStatus::Error => "error",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending_payment" => Some(JobStatus::PendingPayment),
            "processing" => Some(JobStatus::Processing),
            "complete" => Some(JobStatus::Complete),
            "error" => Some(JobStatus::Error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub job_type: JobType,
    pub status: JobStatus,
    pub filename: Option<String>,
    pub file_size: Option<i64>,
    pub file_data: Option<Vec<u8>>,
    pub payment_address: Option<String>,
    pub payment_wif: Option<String>,
    pub required_satoshis: Option<i64>,
    pub manifest_txid: Option<String>,
    pub download_link: Option<String>,
    pub message: String,
    pub progress: f64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // Track metadata
    pub track_title: Option<String>,
    pub cover_txid: Option<String>,
    pub lyrics: Option<String>,
}

impl Job {
    pub fn new_upload(
        id: String,
        filename: String,
        file_size: i64,
        file_data: Vec<u8>,
        payment_address: String,
        payment_wif: String,
        required_satoshis: i64,
    ) -> Self {
        let now = Utc::now();
        Job {
            id,
            job_type: JobType::Upload,
            status: JobStatus::PendingPayment,
            filename: Some(filename),
            file_size: Some(file_size),
            file_data: Some(file_data),
            payment_address: Some(payment_address),
            payment_wif: Some(payment_wif),
            required_satoshis: Some(required_satoshis),
            manifest_txid: None,
            download_link: None,
            message: "Waiting for payment...".to_string(),
            progress: 0.0,
            created_at: now,
            updated_at: now,
            track_title: None,
            cover_txid: None,
            lyrics: None,
        }
    }

    pub fn new_flac_upload(
        id: String,
        filename: String,
        file_size: i64,
        file_data: Vec<u8>,
        payment_address: String,
        payment_wif: String,
        required_satoshis: i64,
    ) -> Self {
        let now = Utc::now();
        Job {
            id,
            job_type: JobType::FlacUpload,
            status: JobStatus::PendingPayment,
            filename: Some(filename),
            file_size: Some(file_size),
            file_data: Some(file_data),
            payment_address: Some(payment_address),
            payment_wif: Some(payment_wif),
            required_satoshis: Some(required_satoshis),
            manifest_txid: None,
            download_link: None,
            message: "Waiting for payment...".to_string(),
            progress: 0.0,
            created_at: now,
            updated_at: now,
            track_title: None,
            cover_txid: None,
            lyrics: None,
        }
    }

    pub fn new_download(id: String, txid: String) -> Self {
        let now = Utc::now();
        Job {
            id,
            job_type: JobType::Download,
            status: JobStatus::Processing,
            filename: None,
            file_size: None,
            file_data: None,
            payment_address: None,
            payment_wif: None,
            required_satoshis: None,
            manifest_txid: Some(txid),
            download_link: None,
            message: "Fetching data from blockchain...".to_string(),
            progress: 0.0,
            created_at: now,
            updated_at: now,
            track_title: None,
            cover_txid: None,
            lyrics: None,
        }
    }

    pub fn new_flac_download(id: String, txid: String) -> Self {
        let now = Utc::now();
        Job {
            id,
            job_type: JobType::FlacDownload,
            status: JobStatus::Processing,
            filename: None,
            file_size: None,
            file_data: None,
            payment_address: None,
            payment_wif: None,
            required_satoshis: None,
            manifest_txid: Some(txid),
            download_link: None,
            message: "Fetching FLAC data from blockchain...".to_string(),
            progress: 0.0,
            created_at: now,
            updated_at: now,
            track_title: None,
            cover_txid: None,
            lyrics: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JobSummary {
    pub id: String,
    pub job_type: JobType,
    pub status: JobStatus,
    pub filename: Option<String>,
    pub file_size: Option<i64>,
    pub manifest_txid: Option<String>,
    pub message: String,
    pub created_at: DateTime<Utc>,
}

impl From<Job> for JobSummary {
    fn from(job: Job) -> Self {
        JobSummary {
            id: job.id,
            job_type: job.job_type,
            status: job.status,
            filename: job.filename,
            file_size: job.file_size,
            manifest_txid: job.manifest_txid,
            message: job.message,
            created_at: job.created_at,
        }
    }
}
