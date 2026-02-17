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
                updated_at TEXT NOT NULL,
                track_title TEXT,
                cover_txid TEXT,
                lyrics TEXT,
                network TEXT
            )",
            [],
        )?;

        // Add new columns if they don't exist (for migration)
        let _ = conn.execute("ALTER TABLE jobs ADD COLUMN track_title TEXT", []);
        let _ = conn.execute("ALTER TABLE jobs ADD COLUMN artist_name TEXT", []);
        let _ = conn.execute("ALTER TABLE jobs ADD COLUMN cover_txid TEXT", []);
        let _ = conn.execute("ALTER TABLE jobs ADD COLUMN cover_data BLOB", []);
        let _ = conn.execute("ALTER TABLE jobs ADD COLUMN lyrics TEXT", []);
        let _ = conn.execute("ALTER TABLE jobs ADD COLUMN network TEXT", []);

        // Create admin_config table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS admin_config (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                admin_pay_mainnet INTEGER NOT NULL DEFAULT 0,
                admin_pay_testnet INTEGER NOT NULL DEFAULT 0,
                mainnet_wif TEXT,
                testnet_wif TEXT,
                updated_at TEXT NOT NULL
            )",
            [],
        )?;

        // Insert default config if not exists
        let _ = conn.execute(
            "INSERT OR IGNORE INTO admin_config (id, admin_pay_mainnet, admin_pay_testnet, updated_at) 
             VALUES (1, 0, 0, ?1)",
            params![Utc::now().to_rfc3339()],
        );

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
                created_at, updated_at, track_title, artist_name, cover_txid, cover_data, lyrics, network
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
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
                job.track_title,
                job.artist_name,
                job.cover_txid,
                job.cover_data,
                job.lyrics,
                job.network,
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
                    created_at, updated_at, track_title, artist_name, cover_txid, cover_data, lyrics, network
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
                    created_at, updated_at, track_title, artist_name, cover_txid, cover_data, lyrics, network
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
                    created_at, updated_at, track_title, artist_name, cover_txid, cover_data, lyrics, network
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

    pub fn update_job_complete_with_filename(
        &self,
        id: &str,
        manifest_txid: &str,
        download_link: Option<&str>,
        filename: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'complete', manifest_txid = ?1, download_link = ?2,
             filename = ?3, message = 'Complete', progress = 100.0, updated_at = ?4 WHERE id = ?5",
            params![manifest_txid, download_link, filename, Utc::now().to_rfc3339(), id],
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
            track_title: row.get(15).ok(),
            artist_name: row.get(16).ok(),
            cover_txid: row.get(17).ok(),
            cover_data: row.get(18).ok(),
            lyrics: row.get(19).ok(),
            network: row.get(20).ok(),
        })
    }

    pub fn update_job_metadata(
        &self,
        id: &str,
        track_title: Option<&str>,
        artist_name: Option<&str>,
        lyrics: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE jobs SET track_title = ?1, artist_name = ?2, lyrics = ?3, updated_at = ?4 WHERE id = ?5",
            params![track_title, artist_name, lyrics, Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn update_job_cover_txid(&self, id: &str, cover_txid: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE jobs SET cover_txid = ?1, updated_at = ?2 WHERE id = ?3",
            params![cover_txid, Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    // Admin config methods
    pub fn get_admin_config(&self) -> Result<AdminConfig> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT admin_pay_mainnet, admin_pay_testnet, mainnet_wif, testnet_wif, updated_at
             FROM admin_config WHERE id = 1",
        )?;

        let mut rows = stmt.query([])?;

        if let Some(row) = rows.next()? {
            Ok(AdminConfig {
                admin_pay_mainnet: row.get::<_, i32>(0)? != 0,
                admin_pay_testnet: row.get::<_, i32>(1)? != 0,
                mainnet_wif: row.get(2).ok(),
                testnet_wif: row.get(3).ok(),
            })
        } else {
            Ok(AdminConfig::default())
        }
    }

    pub fn update_admin_config(&self, config: &AdminConfig) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE admin_config SET admin_pay_mainnet = ?1, admin_pay_testnet = ?2, 
             mainnet_wif = ?3, testnet_wif = ?4, updated_at = ?5 WHERE id = 1",
            params![
                config.admin_pay_mainnet as i32,
                config.admin_pay_testnet as i32,
                config.mainnet_wif,
                config.testnet_wif,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct AdminConfig {
    pub admin_pay_mainnet: bool,
    pub admin_pay_testnet: bool,
    pub mainnet_wif: Option<String>,
    pub testnet_wif: Option<String>,
}
