use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Result};
use std::path::Path;
use std::sync::Mutex;

use crate::models::{Job, JobStatus, JobSummary, JobType};

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn new(path: &str) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let conn = Connection::open(path)?;

        // Create tables
        conn.execute(
            "CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY,
                job_type TEXT NOT NULL,
                status TEXT NOT NULL,
                filename TEXT,
                file_size INTEGER,
                file_data BLOB,
                payment_address TEXT,
                payment_wif TEXT,
                required_satoshis INTEGER,
                manifest_txid TEXT,
                download_link TEXT,
                message TEXT NOT NULL,
                progress REAL NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            [],
        )?;

        Ok(Database {
            conn: Mutex::new(conn),
        })
    }

    pub fn insert_job(&self, job: &Job) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO jobs (
                id, job_type, status, filename, file_size, file_data,
                payment_address, payment_wif, required_satoshis,
                manifest_txid, download_link, message, progress,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                job.id,
                job.job_type.as_str(),
                job.status.as_str(),
                job.filename,
                job.file_size,
                job.file_data,
                job.payment_address,
                job.payment_wif,
                job.required_satoshis,
                job.manifest_txid,
                job.download_link,
                job.message,
                job.progress,
                job.created_at.to_rfc3339(),
                job.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn get_job(&self, id: &str) -> Result<Option<Job>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, job_type, status, filename, file_size, file_data,
                    payment_address, payment_wif, required_satoshis,
                    manifest_txid, download_link, message, progress,
                    created_at, updated_at
             FROM jobs WHERE id = ?1",
        )?;

        let mut rows = stmt.query(params![id])?;

        if let Some(row) = rows.next()? {
            Ok(Some(self.row_to_job(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn get_processing_jobs(&self) -> Result<Vec<Job>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, job_type, status, filename, file_size, file_data,
                    payment_address, payment_wif, required_satoshis,
                    manifest_txid, download_link, message, progress,
                    created_at, updated_at
             FROM jobs WHERE status = 'processing'",
        )?;

        let mut jobs = Vec::new();
        let mut rows = stmt.query([])?;

        while let Some(row) = rows.next()? {
            jobs.push(self.row_to_job(row)?);
        }

        Ok(jobs)
    }

    pub fn get_pending_payment_jobs(&self) -> Result<Vec<Job>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, job_type, status, filename, file_size, file_data,
                    payment_address, payment_wif, required_satoshis,
                    manifest_txid, download_link, message, progress,
                    created_at, updated_at
             FROM jobs WHERE status = 'pending_payment'",
        )?;

        let mut jobs = Vec::new();
        let mut rows = stmt.query([])?;

        while let Some(row) = rows.next()? {
            jobs.push(self.row_to_job(row)?);
        }

        Ok(jobs)
    }

    pub fn get_all_jobs(&self) -> Result<Vec<JobSummary>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, job_type, status, filename, file_size,
                    manifest_txid, message, created_at
             FROM jobs ORDER BY created_at DESC LIMIT 100",
        )?;

        let mut jobs = Vec::new();
        let mut rows = stmt.query([])?;

        while let Some(row) = rows.next()? {
            let created_at_str: String = row.get(7)?;
            jobs.push(JobSummary {
                id: row.get(0)?,
                job_type: JobType::from_str(&row.get::<_, String>(1)?).unwrap_or(JobType::Upload),
                status: JobStatus::from_str(&row.get::<_, String>(2)?).unwrap_or(JobStatus::Error),
                filename: row.get(3)?,
                file_size: row.get(4)?,
                manifest_txid: row.get(5)?,
                message: row.get(6)?,
                created_at: DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            });
        }

        Ok(jobs)
    }

    pub fn update_job_status_only(&self, id: &str, status: JobStatus) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE jobs SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status.as_str(), Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn update_job_status(&self, id: &str, status: JobStatus, message: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE jobs SET status = ?1, message = ?2, updated_at = ?3 WHERE id = ?4",
            params![status.as_str(), message, Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn update_job_progress(&self, id: &str, progress: f64, message: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE jobs SET progress = ?1, message = ?2, updated_at = ?3 WHERE id = ?4",
            params![progress, message, Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn update_job_complete(
        &self,
        id: &str,
        manifest_txid: &str,
        download_link: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'complete', manifest_txid = ?1, download_link = ?2,
             message = 'Complete', progress = 100.0, updated_at = ?3 WHERE id = ?4",
            params![manifest_txid, download_link, Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn update_job_error(&self, id: &str, message: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'error', message = ?1, updated_at = ?2 WHERE id = ?3",
            params![message, Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    fn row_to_job(&self, row: &rusqlite::Row) -> Result<Job> {
        let created_at_str: String = row.get(13)?;
        let updated_at_str: String = row.get(14)?;

        Ok(Job {
            id: row.get(0)?,
            job_type: JobType::from_str(&row.get::<_, String>(1)?).unwrap_or(JobType::Upload),
            status: JobStatus::from_str(&row.get::<_, String>(2)?).unwrap_or(JobStatus::Error),
            filename: row.get(3)?,
            file_size: row.get(4)?,
            file_data: row.get(5)?,
            payment_address: row.get(6)?,
            payment_wif: row.get(7)?,
            required_satoshis: row.get(8)?,
            manifest_txid: row.get(9)?,
            download_link: row.get(10)?,
            message: row.get(11)?,
            progress: row.get(12)?,
            created_at: DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }
}
