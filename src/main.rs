mod config;
mod db;
mod models;
mod routes;
mod services;

use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::services::ServeDir;
use tracing_subscriber;

use crate::config::Config;
use crate::db::Database;
use crate::models::job::JobType;
use crate::services::bitails::{BitailsClient, Utxo};
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
        payment_watcher(watcher_state).await;
    });

    // Build router with increased body limit for large files (50MB)
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
                .route("/api/flac/cover", post(routes::flac::get_cover_image))
        // Wallet API endpoints
        .route("/api/wallet/generate", post(routes::wallet::generate_wallet))
        .route("/api/wallet/import", post(routes::wallet::import_wif))
        .route("/api/wallet/balance", post(routes::wallet::get_balance))
        .route("/api/wallet/send", post(routes::wallet::send_bsv))
                // Admin panel
                .route("/admin", get(routes::admin::admin_page))
                .route("/api/admin/verify", post(routes::admin::verify_admin_key))
                .route("/api/admin/config", post(routes::admin::get_admin_config))
                .route("/api/admin/config/update", post(routes::admin::update_admin_config))
                .route("/api/admin/wallet/balance", post(routes::admin::get_admin_wallet_balance))
                .route("/api/admin/check-pay", post(routes::admin::check_admin_pay))
        // Static files and downloads
        .nest_service("/static", ServeDir::new("static"))
        .nest_service("/downloads", ServeDir::new("./data/downloads"))
        // Set body limit to 50MB for large file uploads
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024))
        .with_state(state);

    // Start server
    let addr = format!("{}:{}", config.host, config.port);
    tracing::info!("Starting server on {}", addr);
    
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

/// Background payment watcher
async fn payment_watcher(state: Arc<RwLock<AppState>>) {
    use crate::models::job::{JobStatus, JobType};
    use tokio::time::{sleep, Duration};

    loop {
        // Get pending payment jobs
        let pending_jobs = {
            let state = state.read().await;
            state.db.get_pending_payment_jobs().unwrap_or_default()
        };

        for job in pending_jobs {
            let state_clone = state.clone();
            let job_id = job.id.clone();
            let address = job.payment_address.clone().unwrap_or_default();
            let job_type = job.job_type.clone();
            let network = job.network.clone().unwrap_or_else(|| "mainnet".to_string());
            
            tokio::spawn(async move {
                // Check for payment based on network
                let has_payment = if network == "testnet" {
                    // Use WhatsOnChain API for testnet
                    check_testnet_payment(&address).await
                } else {
                    // Use Bitails API for mainnet
                    let state = state_clone.read().await;
                    match state.bitails.get_address_unspent(&address).await {
                        Ok(utxos) => !utxos.is_empty(),
                        Err(_) => false,
                    }
                };

                if has_payment {
                    // Payment received! Update job status to processing
                    let state = state_clone.read().await;
                    let _ = state.db.update_job_status(&job_id, JobStatus::Processing, "Payment received, processing...");
                    drop(state);

                    // Process the job
                    process_job(state_clone, job_id, job_type, address, network).await;
                }
            });
        }

        sleep(Duration::from_secs(3)).await;
    }
}

/// Check for payment on testnet using WhatsOnChain API
async fn check_testnet_payment(address: &str) -> bool {
    let client = reqwest::Client::new();
    let url = format!("https://api.whatsonchain.com/v1/bsv/test/address/{}/unspent", address);
    
    match client.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                match response.json::<Vec<serde_json::Value>>().await {
                    Ok(utxos) => !utxos.is_empty(),
                    Err(_) => false,
                }
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

/// Get testnet UTXOs for upload using WhatsOnChain API
async fn get_testnet_utxos_for_upload(address: &str) -> Result<Vec<crate::services::bitails::Utxo>, String> {
    let client = reqwest::Client::new();
    let url = format!("https://api.whatsonchain.com/v1/bsv/test/address/{}/unspent", address);
    
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    
    if !response.status().is_success() {
        return Err(format!("API error: {}", response.status()));
    }
    
    let json: Vec<serde_json::Value> = response
        .json()
        .await
        .map_err(|e| format!("Parse error: {}", e))?;
    
    let utxos: Vec<crate::services::bitails::Utxo> = json
        .iter()
        .filter_map(|v| {
            let txid = v.get("tx_hash")?.as_str()?.to_string();
            let vout = v.get("tx_pos")?.as_u64()? as u32;
            let satoshis = v.get("value")?.as_i64()?;
            Some(crate::services::bitails::Utxo { 
                txid, 
                vout, 
                satoshis,
                script_pubkey: String::new(),
                blockheight: None,
                confirmations: None,
            })
        })
        .collect();
    
    Ok(utxos)
}

/// Broadcast transaction to testnet using WhatsOnChain API
async fn broadcast_testnet_tx(raw_tx: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let url = "https://api.whatsonchain.com/v1/bsv/test/tx/raw";
    
    let response = client
        .post(url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "txhex": raw_tx }))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    
    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("Broadcast failed: {}", error_text));
    }
    
    let txid = response
        .text()
        .await
        .map_err(|e| format!("Parse error: {}", e))?;
    
    // Remove quotes, whitespace, and newlines
    Ok(txid.trim().trim_matches('"').trim().to_string())
}

/// Process a job based on its type
async fn process_job(state: Arc<RwLock<AppState>>, job_id: String, job_type: JobType, address: String, network: String) {
    use crate::models::job::JobStatus;
    use tokio::time::{sleep, Duration};

    // Get job details
    let job = {
        let state = state.read().await;
        state.db.get_job(&job_id).ok().flatten()
    };

    let job = match job {
        Some(j) => j,
        None => return,
    };

    match job_type {
        JobType::Upload => {
            process_upload(
                state,
                job_id,
                job.payment_wif.unwrap_or_default(),
                address,
                job.file_data,
                job.filename,
                network,
            ).await;
        }
        JobType::FlacUpload => {
            process_flac_upload(
                state,
                job_id,
                job.payment_wif.unwrap_or_default(),
                address,
                job.file_data,
                job.filename,
                network,
                job.track_title,
                job.artist_name,
                job.lyrics,
                job.cover_data,
            ).await;
        }
        JobType::Download => {
            process_download(state, job_id, job.manifest_txid).await;
        }
        JobType::FlacDownload => {
            let network = job.network.unwrap_or_else(|| "mainnet".to_string());
            process_flac_download(state, job_id, job.manifest_txid, network).await;
        }
    }
}

/// Process regular upload
async fn process_upload(
    state: Arc<RwLock<AppState>>,
    job_id: String,
    wif: String,
    address: String,
    file_data: Option<Vec<u8>>,
    filename: Option<String>,
    _network: String,
) {
    use crate::models::job::JobStatus;
    use crate::services::bsv::BsvService;

    let file_data = match file_data {
        Some(data) => data,
        None => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, "No file data found");
            return;
        }
    };

    let filename = filename.unwrap_or_else(|| "file.bin".to_string());

    // Update progress
    {
        let state = state.read().await;
        let _ = state.db.update_job_progress(&job_id, 10.0, "Fetching UTXOs...");
    }

    // Get UTXOs
    let utxos = {
        let state = state.read().await;
        state.bitails.get_address_unspent(&address).await
    };

    let utxos = match utxos {
        Ok(u) => u,
        Err(e) => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, &format!("Failed to get UTXOs: {}", e));
            return;
        }
    };

    if utxos.is_empty() {
        let state = state.read().await;
        let _ = state.db.update_job_error(&job_id, "No UTXOs found");
        return;
    }

    // Update progress
    {
        let state = state.read().await;
        let _ = state.db.update_job_progress(&job_id, 30.0, "Creating transaction...");
    }

    // Calculate total input
    let total_input: i64 = utxos.iter().map(|u| u.satoshis).sum();

    // Get scriptPubKey for the address
    let script_pubkey = match BsvService::create_p2pkh_script(&address) {
        Ok(s) => s,
        Err(e) => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, &format!("Failed to create script: {}", e));
            return;
        }
    };

    // Prepare UTXOs for transaction
    let utxo_inputs: Vec<(String, u32, i64, Vec<u8>)> = utxos
        .iter()
        .map(|u| (u.txid.clone(), u.vout, u.satoshis, script_pubkey.clone()))
        .collect();

    // Create OP_RETURN script with file data
    let protocol = b"upfile";
    let mime = b"application/octet-stream";
    let op_return_script = BsvService::create_op_return_script(&[protocol, mime, filename.as_bytes(), &file_data]);

    // Calculate fee
    let tx_size = 150 + op_return_script.len();
    let fee = {
        let state = state.read().await;
        (tx_size as f64 * state.bsv.fee_rate).ceil() as i64
    };

    // Outputs: OP_RETURN (0 satoshis)
    let outputs: Vec<(Vec<u8>, i64)> = vec![(op_return_script, 0)];

    // Check if we have enough for fee
    if total_input < fee {
        let state = state.read().await;
        let _ = state.db.update_job_error(
            &job_id,
            &format!("Insufficient funds: {} < {}", total_input, fee),
        );
        return;
    }

    // Create transaction
    let raw_tx = {
        let state = state.read().await;
        state.bsv.create_transaction(&wif, &utxo_inputs, &outputs)
    };

    let raw_tx = match raw_tx {
        Ok(tx) => tx,
        Err(e) => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, &format!("Failed to create tx: {}", e));
            return;
        }
    };

    // Update progress
    {
        let state = state.read().await;
        let _ = state.db.update_job_progress(&job_id, 60.0, "Broadcasting transaction...");
    }

    // Broadcast transaction
    let broadcast_result = {
        let state = state.read().await;
        state.bitails.broadcast_transaction(&raw_tx).await
    };

    match broadcast_result {
        Ok(txid) => {
            let state = state.read().await;
            let _ = state.db.update_job_complete(&job_id, &txid, None);
            tracing::info!("Upload complete for job {}: txid={}", job_id, txid);
        }
        Err(e) => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, &format!("Broadcast failed: {}", e));
        }
    }
}

/// Process FLAC upload with multi-transaction chunking
async fn process_flac_upload(
    state: Arc<RwLock<AppState>>,
    job_id: String,
    wif: String,
    address: String,
    file_data: Option<Vec<u8>>,
    filename: Option<String>,
    network: String,
    track_title: Option<String>,
    artist_name: Option<String>,
    lyrics: Option<String>,
    cover_data: Option<Vec<u8>>,
) {
    use crate::models::job::JobStatus;
    use crate::services::bsv::BsvService;
    use crate::services::bitails::Utxo;
    use tokio::time::{sleep, Duration};

    let file_data = match file_data {
        Some(data) => data,
        None => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, "No file data found");
            return;
        }
    };

    let filename = filename.unwrap_or_else(|| "audio.flac".to_string());
    let file_size = file_data.len();

    // Maximum chunk size per transaction (1MB chunks)
    let max_tx_data_size = 1024 * 1024; // 1MB chunks

    // Check if we need multi-transaction approach
    let needs_chunking = file_size > max_tx_data_size;

    // Update progress
    {
        let state = state.read().await;
        let _ = state.db.update_job_progress(&job_id, 5.0, "Fetching UTXOs...");
    }

    // Get UTXOs based on network
    let mut utxos: Vec<Utxo> = if network == "testnet" {
        match get_testnet_utxos_for_upload(&address).await {
            Ok(u) => u,
            Err(e) => {
                let state = state.read().await;
                let _ = state.db.update_job_error(&job_id, &format!("Failed to get UTXOs: {}", e));
                return;
            }
        }
    } else {
        let result = {
            let state = state.read().await;
            state.bitails.get_address_unspent(&address).await
        };
        match result {
            Ok(u) => u,
            Err(e) => {
                let state = state.read().await;
                let _ = state.db.update_job_error(&job_id, &format!("Failed to get UTXOs: {}", e));
                return;
            }
        }
    };

    if utxos.is_empty() {
        let state = state.read().await;
        let _ = state.db.update_job_error(&job_id, "No UTXOs found");
        return;
    }

    // Get scriptPubKey for the address
    let script_pubkey = match BsvService::create_p2pkh_script(&address) {
        Ok(s) => s,
        Err(e) => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, &format!("Failed to create script: {}", e));
            return;
        }
    };

    // Upload cover image to BSV if present
    let cover_txid: Option<String> = if let Some(ref cover_bytes) = cover_data {
        {
            let state = state.read().await;
            let _ = state.db.update_job_progress(&job_id, 3.0, "Uploading cover image...");
        }
        
        // Create cover image transaction
        let cover_script = BsvService::create_cover_image_script(cover_bytes);
        
        // Use first UTXO for cover image
        if utxos.is_empty() {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, "No UTXOs for cover image");
            return;
        }
        
        let cover_utxo = utxos.remove(0);
        let cover_utxo_input = vec![(
            cover_utxo.txid.clone(),
            cover_utxo.vout,
            cover_utxo.satoshis,
            script_pubkey.clone(),
        )];
        
        // Calculate change
        let cover_tx_size = 150 + cover_script.len();
        let cover_fee = {
            let state = state.read().await;
            (cover_tx_size as f64 * state.bsv.fee_rate).ceil() as i64
        };
        
        let change_amount = cover_utxo.satoshis - cover_fee - 1;
        let mut outputs: Vec<(Vec<u8>, i64)> = vec![(cover_script, 1)];
        if change_amount > 546 {
            outputs.push((script_pubkey.clone(), change_amount));
        }
        
        let cover_raw_tx = {
            let state = state.read().await;
            state.bsv.create_transaction(&wif, &cover_utxo_input, &outputs)
        };
        
        let cover_raw_tx = match cover_raw_tx {
            Ok(tx) => tx,
            Err(e) => {
                tracing::warn!("Failed to create cover tx: {}", e);
                String::new()
            }
        };
        
        if cover_raw_tx.is_empty() {
            None
        } else {
            // Broadcast cover image transaction
            let cover_broadcast_result = if network == "testnet" {
                broadcast_testnet_tx(&cover_raw_tx).await
            } else {
                let state = state.read().await;
                state.bitails.broadcast_transaction(&cover_raw_tx).await
            };
            
            match cover_broadcast_result {
                Ok(txid) => {
                    tracing::info!("Cover image uploaded: {}", txid);
                    // Add change output as new UTXO if we created one
                    if change_amount > 546 {
                        utxos.insert(0, Utxo {
                            txid: txid.clone(),
                            vout: 1,
                            satoshis: change_amount,
                            script_pubkey: String::new(),
                            blockheight: Some(0),
                            confirmations: Some(0),
                        });
                    }
                    // Wait for propagation
                    sleep(Duration::from_millis(1000)).await;
                    Some(txid)
                }
                Err(e) => {
                    tracing::warn!("Failed to broadcast cover image: {}", e);
                    None
                }
            }
        }
    } else {
        None
    };

    if needs_chunking {
        // Multi-transaction chunking approach with UTXO pre-splitting
        let total_input: i64 = utxos.iter().map(|u| u.satoshis).sum();
        
        // Split file into chunks
        let mut chunks: Vec<Vec<u8>> = Vec::new();
        let mut offset = 0;
        while offset < file_size {
            let end = std::cmp::min(offset + max_tx_data_size, file_size);
            chunks.push(file_data[offset..end].to_vec());
            offset = end;
        }

        let total_chunks = chunks.len();
        let num_outputs = total_chunks + 1; // +1 for manifest
        
        tracing::info!("Splitting {} bytes into {} chunks for job {}", file_size, total_chunks, job_id);

        // Calculate satoshis needed per output
        let satoshis_per_output = {
            let state = state.read().await;
            state.bsv.calculate_chunk_output_satoshis(max_tx_data_size)
        };
        
        tracing::info!("Satoshis per output: {}, total outputs: {}", satoshis_per_output, num_outputs);

        // Update progress
        {
            let state = state.read().await;
            let _ = state.db.update_job_progress(
                &job_id,
                5.0,
                &format!("Preparing UTXO split for {} chunks...", total_chunks),
            );
        }

        // Step 1: Create and broadcast UTXO split transaction
        let first_utxo = &utxos[0];
        let split_tx = {
            let state = state.read().await;
            state.bsv.create_split_transaction(
                &wif,
                &first_utxo.txid,
                first_utxo.vout,
                total_input,
                &script_pubkey,
                num_outputs,
                satoshis_per_output,
            )
        };

        let split_tx = match split_tx {
            Ok(tx) => tx,
            Err(e) => {
                let state = state.read().await;
                let _ = state.db.update_job_error(&job_id, &format!("Failed to create split tx: {}", e));
                return;
            }
        };

        {
            let state = state.read().await;
            let _ = state.db.update_job_progress(&job_id, 8.0, "Broadcasting UTXO split transaction...");
        }

        let split_txid = if network == "testnet" {
            broadcast_testnet_tx(&split_tx).await
        } else {
            let state = state.read().await;
            state.bitails.broadcast_transaction(&split_tx).await
        };

        let split_txid = match split_txid {
            Ok(txid) => {
                tracing::info!("UTXO split transaction broadcast: {}", txid);
                txid
            }
            Err(e) => {
                let state = state.read().await;
                let _ = state.db.update_job_error(&job_id, &format!("Failed to broadcast split tx: {}", e));
                return;
            }
        };

        // Small delay to let the split tx propagate
        sleep(Duration::from_millis(1000)).await;

        // Now we have num_outputs UTXOs from the split transaction
        // Each output is at vout 0, 1, 2, ... (num_outputs - 1)
        // We'll use outputs 0 to (total_chunks - 1) for chunks
        // And output total_chunks for the manifest

        {
            let state = state.read().await;
            let _ = state.db.update_job_progress(
                &job_id,
                10.0,
                &format!("Uploading {} chunks...", total_chunks),
            );
        }

        // Broadcast each chunk using its dedicated UTXO
        let mut chunk_txids: Vec<String> = Vec::new();
        
        for (i, chunk) in chunks.iter().enumerate() {
            let progress = 10.0 + (70.0 * (i as f64 / total_chunks as f64));
            
            {
                let state = state.read().await;
                let _ = state.db.update_job_progress(
                    &job_id,
                    progress,
                    &format!("Uploading chunk {}/{}...", i + 1, total_chunks),
                );
            }

            // Create chunk script
            let chunk_script = BsvService::create_flac_chunk_script(i as u32, total_chunks as u32, chunk);

            // Calculate fee for this chunk
            let tx_size = 200 + chunk_script.len();
            let fee = {
                let state = state.read().await;
                (tx_size as f64 * state.bsv.fee_rate).ceil() as i64
            };

            // Use the dedicated UTXO for this chunk (from split transaction)
            let chunk_utxo_input = vec![(
                split_txid.clone(),
                i as u32,  // vout is the chunk index
                satoshis_per_output,
                script_pubkey.clone(),
            )];

            // Output: chunk data only (use all remaining satoshis as implicit fee)
            let outputs: Vec<(Vec<u8>, i64)> = vec![(chunk_script, 1)];

            // Create transaction
            let raw_tx = {
                let state = state.read().await;
                state.bsv.create_transaction(&wif, &chunk_utxo_input, &outputs)
            };

            let raw_tx = match raw_tx {
                Ok(tx) => tx,
                Err(e) => {
                    let state = state.read().await;
                    let _ = state.db.update_job_error(&job_id, &format!("Failed to create chunk {} tx: {}", i + 1, e));
                    return;
                }
            };

            // Broadcast with retry logic
            let mut broadcast_success = false;
            let mut last_error = String::new();
            
            for retry in 0..5 {
                if retry > 0 {
                    // Exponential backoff: 1s, 2s, 4s, 8s
                    let delay = Duration::from_secs(1 << retry);
                    tracing::warn!("Retrying chunk {} broadcast after {:?} (attempt {})", i + 1, delay, retry + 1);
                    sleep(delay).await;
                }
                
                let broadcast_result = if network == "testnet" {
                    broadcast_testnet_tx(&raw_tx).await
                } else {
                    let state = state.read().await;
                    state.bitails.broadcast_transaction(&raw_tx).await
                };

                match broadcast_result {
                    Ok(txid) => {
                        tracing::info!("Chunk {}/{} broadcast: {}", i + 1, total_chunks, txid);
                        chunk_txids.push(txid);
                        broadcast_success = true;
                        break;
                    }
                    Err(e) => {
                        last_error = e;
                        tracing::warn!("Chunk {} broadcast failed: {}", i + 1, last_error);
                    }
                }
            }
            
            if !broadcast_success {
                let state = state.read().await;
                let _ = state.db.update_job_error(&job_id, &format!("Failed to broadcast chunk {} after 5 retries: {}", i + 1, last_error));
                return;
            }
            
            // Small delay between broadcasts
            sleep(Duration::from_millis(500)).await;
        }

        // Now create manifest transaction using the last split UTXO
        {
            let state = state.read().await;
            let _ = state.db.update_job_progress(&job_id, 85.0, "Creating manifest...");
        }

        // Create manifest script with title, artist, lyrics, and cover
        let manifest_script = BsvService::create_flac_manifest_script(
            &filename,
            file_size,
            &chunk_txids,
            track_title.as_deref(),
            artist_name.as_deref(),
            lyrics.as_deref(),
            cover_txid.as_deref(),
        );

        // Use the last split UTXO for manifest (vout = total_chunks)
        let manifest_utxo_input = vec![(
            split_txid.clone(),
            total_chunks as u32,  // Last output from split tx
            satoshis_per_output,
            script_pubkey.clone(),
        )];

        let outputs: Vec<(Vec<u8>, i64)> = vec![(manifest_script, 1)];

        let raw_tx = {
            let state = state.read().await;
            state.bsv.create_transaction(&wif, &manifest_utxo_input, &outputs)
        };

        let raw_tx = match raw_tx {
            Ok(tx) => tx,
            Err(e) => {
                let state = state.read().await;
                let _ = state.db.update_job_error(&job_id, &format!("Failed to create manifest tx: {}", e));
                return;
            }
        };

        // Broadcast manifest
        {
            let state = state.read().await;
            let _ = state.db.update_job_progress(&job_id, 95.0, "Broadcasting manifest...");
        }

        let broadcast_result = if network == "testnet" {
            broadcast_testnet_tx(&raw_tx).await
        } else {
            let state = state.read().await;
            state.bitails.broadcast_transaction(&raw_tx).await
        };

        match broadcast_result {
            Ok(manifest_txid) => {
                let state = state.read().await;
                let _ = state.db.update_job_complete(&job_id, &manifest_txid, None);
                tracing::info!(
                    "FLAC upload complete for job {}: manifest_txid={}, {} chunks",
                    job_id,
                    manifest_txid,
                    total_chunks
                );
            }
            Err(e) => {
                let state = state.read().await;
                let _ = state.db.update_job_error(&job_id, &format!("Failed to broadcast manifest: {}", e));
            }
        }
    } else {
        // Single transaction approach (for small files)
        {
            let state = state.read().await;
            let _ = state.db.update_job_progress(&job_id, 30.0, "Creating FLAC transaction...");
        }

        let total_input: i64 = utxos.iter().map(|u| u.satoshis).sum();

        let utxo_inputs: Vec<(String, u32, i64, Vec<u8>)> = utxos
            .iter()
            .map(|u| (u.txid.clone(), u.vout, u.satoshis, script_pubkey.clone()))
            .collect();

        // Create OP_FALSE OP_IF script for FLAC storage
        let protocol = b"flacstore";
        let mime_type = b"audio/flac";
        
        let metadata = serde_json::json!({
            "filename": filename,
            "size": file_data.len(),
            "version": "1.0",
            "chunked": false
        }).to_string();

        let max_chunk_size = 100 * 1024; // 100KB
        let data_chunks = BsvService::split_into_chunks(&file_data, max_chunk_size);

        let flac_script = BsvService::create_flac_store_script(
            protocol,
            mime_type,
            metadata.as_bytes(),
            &data_chunks,
        );

        let tx_size = 150 + flac_script.len();
        let fee = {
            let state = state.read().await;
            (tx_size as f64 * state.bsv.fee_rate).ceil() as i64
        };

        let outputs: Vec<(Vec<u8>, i64)> = vec![(flac_script, 1)];

        if total_input < fee {
            let state = state.read().await;
            let _ = state.db.update_job_error(
                &job_id,
                &format!("Insufficient funds: {} < {}", total_input, fee),
            );
            return;
        }

        let raw_tx = {
            let state = state.read().await;
            state.bsv.create_transaction(&wif, &utxo_inputs, &outputs)
        };

        let raw_tx = match raw_tx {
            Ok(tx) => tx,
            Err(e) => {
                let state = state.read().await;
                let _ = state.db.update_job_error(&job_id, &format!("Failed to create tx: {}", e));
                return;
            }
        };

        {
            let state = state.read().await;
            let _ = state.db.update_job_progress(&job_id, 60.0, "Broadcasting FLAC transaction...");
        }

        let broadcast_result = if network == "testnet" {
            broadcast_testnet_tx(&raw_tx).await
        } else {
            let state = state.read().await;
            state.bitails.broadcast_transaction(&raw_tx).await
        };

        match broadcast_result {
            Ok(txid) => {
                let state = state.read().await;
                let _ = state.db.update_job_complete(&job_id, &txid, None);
                tracing::info!("FLAC upload complete for job {}: txid={}", job_id, txid);
            }
            Err(e) => {
                let state = state.read().await;
                let _ = state.db.update_job_error(&job_id, &format!("Broadcast failed: {}", e));
            }
        }
    }
}

/// Process download
async fn process_download(state: Arc<RwLock<AppState>>, job_id: String, txid: Option<String>) {
    let txid = match txid {
        Some(t) => t,
        None => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, "No TXID provided");
            return;
        }
    };

    {
        let state = state.read().await;
        let _ = state.db.update_job_progress(&job_id, 10.0, "Fetching transaction...");
    }

    let tx_data = {
        let state = state.read().await;
        state.bitails.download_tx_raw(&txid).await
    };

    let tx_data = match tx_data {
        Ok(data) => data,
        Err(e) => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, &format!("Failed to fetch tx: {}", e));
            return;
        }
    };

    {
        let state = state.read().await;
        let _ = state.db.update_job_progress(&job_id, 50.0, "Extracting data...");
    }

    let (file_data, filename) = match extract_op_return_from_tx(&tx_data) {
        Some(data) => data,
        None => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, "No OP_RETURN data found in transaction");
            return;
        }
    };

    let downloads_dir = std::path::Path::new("./data/downloads");
    std::fs::create_dir_all(downloads_dir).ok();

    let file_path = downloads_dir.join(&filename);
    if let Err(e) = std::fs::write(&file_path, &file_data) {
        let state = state.read().await;
        let _ = state.db.update_job_error(&job_id, &format!("Failed to save file: {}", e));
        return;
    }

    {
        let state = state.read().await;
        let _ = state.db.update_job_complete(
            &job_id,
            &txid,
            Some(&file_path.to_string_lossy()),
        );
    }

    tracing::info!("Download complete for job {}: {}", job_id, filename);
}

/// Fetch transaction data from appropriate API based on network
async fn fetch_tx_raw(state: &Arc<RwLock<AppState>>, txid: &str, network: &str) -> Result<String, String> {
    if network == "testnet" {
        // Use WhatsOnChain Testnet API
        let url = format!("https://api.whatsonchain.com/v1/bsv/test/tx/{}/hex", txid);
        let client = reqwest::Client::new();
        let response = client.get(&url).send().await.map_err(|e| format!("Request failed: {}", e))?;
        if !response.status().is_success() {
            return Err(format!("API error: {}", response.status()));
        }
        response.text().await.map_err(|e| format!("Parse error: {}", e))
    } else {
        // Use Bitails Mainnet API
        let state = state.read().await;
        state.bitails.download_tx_raw(txid).await
    }
}

/// Process FLAC download
async fn process_flac_download(state: Arc<RwLock<AppState>>, job_id: String, txid: Option<String>, network: String) {
    use tokio::time::{sleep, Duration};

    let txid = match txid {
        Some(t) => t,
        None => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, "No TXID provided");
            return;
        }
    };

    {
        let state = state.read().await;
        let _ = state.db.update_job_progress(&job_id, 5.0, "Fetching manifest transaction...");
    }

    let tx_data = fetch_tx_raw(&state, &txid, &network).await;

    let tx_data = match tx_data {
        Ok(data) => data,
        Err(e) => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, &format!("Failed to fetch tx: {}", e));
            return;
        }
    };

    {
        let state = state.read().await;
        let _ = state.db.update_job_progress(&job_id, 10.0, "Parsing transaction...");
    }

    // Try to extract as manifest first
    if let Some(manifest) = extract_flac_manifest_from_tx(&tx_data) {
        // Multi-chunk download
        let filename = manifest.filename;
        let chunk_txids = manifest.chunk_txids;
        let track_title = manifest.title;
        let artist_name = manifest.artist;
        let lyrics = manifest.lyrics;
        let cover_txid = manifest.cover_txid;
        let total_chunks = chunk_txids.len();
        let mut all_data: Vec<u8> = Vec::new();

        for (i, chunk_txid) in chunk_txids.iter().enumerate() {
            let progress = 15.0 + (75.0 * (i as f64 / total_chunks as f64));
            
            {
                let state = state.read().await;
                let _ = state.db.update_job_progress(
                    &job_id,
                    progress,
                    &format!("Downloading chunk {}/{}...", i + 1, total_chunks),
                );
            }

            let chunk_tx_data = fetch_tx_raw(&state, chunk_txid, &network).await;

            let chunk_tx_data = match chunk_tx_data {
                Ok(data) => data,
                Err(e) => {
                    let state = state.read().await;
                    let _ = state.db.update_job_error(
                        &job_id,
                        &format!("Failed to fetch chunk {}: {}", i + 1, e),
                    );
                    return;
                }
            };

            if let Some(chunk_data) = extract_flac_chunk_from_tx(&chunk_tx_data) {
                all_data.extend(chunk_data);
            } else {
                let state = state.read().await;
                let _ = state.db.update_job_error(
                    &job_id,
                    &format!("Failed to extract data from chunk {}", i + 1),
                );
                return;
            }

            sleep(Duration::from_millis(100)).await;
        }

        {
            let state = state.read().await;
            let _ = state.db.update_job_progress(&job_id, 95.0, "Saving file...");
        }

        let downloads_dir = std::path::Path::new("./data/downloads");
        std::fs::create_dir_all(downloads_dir).ok();

        let file_path = downloads_dir.join(&filename);
        if let Err(e) = std::fs::write(&file_path, &all_data) {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, &format!("Failed to save file: {}", e));
            return;
        }

        // Create web-accessible download link
        let download_link = format!("/downloads/{}", filename);
        
        {
            let state = state.read().await;
            let _ = state.db.update_job_complete_with_filename(
                &job_id,
                &txid,
                Some(&download_link),
                &filename,
            );
            // Update metadata (title, artist, lyrics, cover_txid) from manifest
            let _ = state.db.update_job_metadata(
                &job_id,
                track_title.as_deref(),
                artist_name.as_deref(),
                lyrics.as_deref(),
            );
            // Update cover_txid if available
            if let Some(ref cover) = cover_txid {
                let _ = state.db.update_job_cover_txid(&job_id, cover);
            }
        }
        tracing::info!(
            "FLAC chunked download complete for job {}: {} ({} chunks, {} bytes, title: {:?})",
            job_id,
            filename,
            total_chunks,
            all_data.len(),
            track_title
        );
    } else if let Some((file_data, filename)) = extract_flac_from_tx(&tx_data) {
        // Single transaction download
        let downloads_dir = std::path::Path::new("./data/downloads");
        std::fs::create_dir_all(downloads_dir).ok();

        let file_path = downloads_dir.join(&filename);
        if let Err(e) = std::fs::write(&file_path, &file_data) {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, &format!("Failed to save file: {}", e));
            return;
        }

        // Create web-accessible download link
        let download_link = format!("/downloads/{}", filename);
        
        let state = state.read().await;
        let _ = state.db.update_job_complete_with_filename(
            &job_id,
            &txid,
            Some(&download_link),
            &filename,
        );
        tracing::info!("FLAC download complete for job {}: {}", job_id, filename);
    } else {
        let state = state.read().await;
        let _ = state.db.update_job_error(&job_id, "No FLAC data found in transaction");
    }
}

// Helper functions for transaction parsing

fn extract_op_return_from_tx(tx_hex: &str) -> Option<(Vec<u8>, String)> {
    let tx_bytes = hex::decode(tx_hex).ok()?;
    
    let mut i = 0;
    i += 4; // version
    
    let (input_count, varint_size) = read_varint(&tx_bytes[i..])?;
    i += varint_size;
    
    for _ in 0..input_count {
        i += 32;
        i += 4;
        let (script_len, vs) = read_varint(&tx_bytes[i..])?;
        i += vs;
        i += script_len as usize;
        i += 4;
    }
    
    let (output_count, varint_size) = read_varint(&tx_bytes[i..])?;
    i += varint_size;
    
    for _ in 0..output_count {
        i += 8;
        let (script_len, vs) = read_varint(&tx_bytes[i..])?;
        i += vs;
        
        let script = &tx_bytes[i..i + script_len as usize];
        i += script_len as usize;
        
        if script.len() > 2 && ((script[0] == 0x00 && script[1] == 0x6a) || script[0] == 0x6a) {
            let start = if script[0] == 0x00 { 2 } else { 1 };
            return parse_op_return_script(&script[start..]);
        }
    }
    
    None
}

fn extract_flac_manifest_from_tx(tx_hex: &str) -> Option<ManifestMetadata> {
    let tx_bytes = hex::decode(tx_hex).ok()?;
    
    let mut i = 0;
    i += 4;
    
    let (input_count, varint_size) = read_varint(&tx_bytes[i..])?;
    i += varint_size;
    
    for _ in 0..input_count {
        i += 32;
        i += 4;
        let (script_len, vs) = read_varint(&tx_bytes[i..])?;
        i += vs;
        i += script_len as usize;
        i += 4;
    }
    
    let (output_count, varint_size) = read_varint(&tx_bytes[i..])?;
    i += varint_size;
    
    for _ in 0..output_count {
        i += 8;
        let (script_len, vs) = read_varint(&tx_bytes[i..])?;
        i += vs;
        
        let script = &tx_bytes[i..i + script_len as usize];
        i += script_len as usize;
        
        if script.len() > 2 && script[0] == 0x00 && script[1] == 0x63 {
            if let Some(manifest) = parse_flac_manifest_script(&script[2..]) {
                return Some(manifest);
            }
        }
    }
    
    None
}

/// Manifest metadata structure
#[derive(Debug, Clone)]
struct ManifestMetadata {
    filename: String,
    chunk_txids: Vec<String>,
    title: Option<String>,
    artist: Option<String>,
    lyrics: Option<String>,
    cover_txid: Option<String>,
}

fn parse_flac_manifest_script(script: &[u8]) -> Option<ManifestMetadata> {
    let mut i = 0;
    let mut push_data_items: Vec<Vec<u8>> = Vec::new();
    
    while i < script.len() {
        if script[i] == 0x68 {
            break;
        }
        
        let (data, consumed) = read_push_data(&script[i..])?;
        push_data_items.push(data);
        i += consumed;
    }
    
    if push_data_items.len() < 3 {
        return None;
    }
    
    let protocol = String::from_utf8_lossy(&push_data_items[0]);
    if protocol != "flacstore-manifest" {
        return None;
    }
    
    let filename = String::from_utf8_lossy(&push_data_items[1]).to_string();
    
    // Parse metadata JSON to extract title, artist, lyrics, and cover_txid
    let metadata_str = String::from_utf8_lossy(&push_data_items[2]);
    let (title, artist, lyrics, cover_txid) = if let Ok(metadata) = serde_json::from_str::<serde_json::Value>(&metadata_str) {
        let title = metadata["title"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string());
        let artist = metadata["artist"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string());
        let lyrics = metadata["lyrics"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string());
        let cover_txid = metadata["cover_txid"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string());
        (title, artist, lyrics, cover_txid)
    } else {
        (None, None, None, None)
    };
    
    let chunk_txids: Vec<String> = push_data_items[3..]
        .iter()
        .map(|data| String::from_utf8_lossy(data).to_string())
        .collect();
    
    if chunk_txids.is_empty() {
        return None;
    }
    
    Some(ManifestMetadata {
        filename,
        chunk_txids,
        title,
        artist,
        lyrics,
        cover_txid,
    })
}

fn extract_flac_chunk_from_tx(tx_hex: &str) -> Option<Vec<u8>> {
    let tx_bytes = hex::decode(tx_hex).ok()?;
    
    let mut i = 0;
    i += 4;
    
    let (input_count, varint_size) = read_varint(&tx_bytes[i..])?;
    i += varint_size;
    
    for _ in 0..input_count {
        i += 32;
        i += 4;
        let (script_len, vs) = read_varint(&tx_bytes[i..])?;
        i += vs;
        i += script_len as usize;
        i += 4;
    }
    
    let (output_count, varint_size) = read_varint(&tx_bytes[i..])?;
    i += varint_size;
    
    for _ in 0..output_count {
        i += 8;
        let (script_len, vs) = read_varint(&tx_bytes[i..])?;
        i += vs;
        
        let script = &tx_bytes[i..i + script_len as usize];
        i += script_len as usize;
        
        if script.len() > 2 && script[0] == 0x00 && script[1] == 0x63 {
            if let Some(data) = parse_flac_chunk_script(&script[2..]) {
                return Some(data);
            }
        }
    }
    
    None
}

fn parse_flac_chunk_script(script: &[u8]) -> Option<Vec<u8>> {
    let mut i = 0;
    let mut push_data_items: Vec<Vec<u8>> = Vec::new();
    
    while i < script.len() {
        if script[i] == 0x68 {
            break;
        }
        
        let (data, consumed) = read_push_data(&script[i..])?;
        push_data_items.push(data);
        i += consumed;
    }
    
    if push_data_items.len() < 3 {
        return None;
    }
    
    let protocol = String::from_utf8_lossy(&push_data_items[0]);
    if protocol != "flacstore-chunk" {
        return None;
    }
    
    if push_data_items.len() >= 3 {
        return Some(push_data_items[2].clone());
    }
    
    None
}

fn extract_flac_from_tx(tx_hex: &str) -> Option<(Vec<u8>, String)> {
    let tx_bytes = hex::decode(tx_hex).ok()?;
    
    let mut i = 0;
    i += 4;
    
    let (input_count, varint_size) = read_varint(&tx_bytes[i..])?;
    i += varint_size;
    
    for _ in 0..input_count {
        i += 32;
        i += 4;
        let (script_len, vs) = read_varint(&tx_bytes[i..])?;
        i += vs;
        i += script_len as usize;
        i += 4;
    }
    
    let (output_count, varint_size) = read_varint(&tx_bytes[i..])?;
    i += varint_size;
    
    for _ in 0..output_count {
        i += 8;
        let (script_len, vs) = read_varint(&tx_bytes[i..])?;
        i += vs;
        
        let script = &tx_bytes[i..i + script_len as usize];
        i += script_len as usize;
        
        if script.len() > 2 && script[0] == 0x00 && script[1] == 0x63 {
            if let Some((data, filename)) = parse_flac_store_script(&script[2..]) {
                return Some((data, filename));
            }
        }
    }
    
    None
}

fn parse_flac_store_script(script: &[u8]) -> Option<(Vec<u8>, String)> {
    let mut i = 0;
    let mut push_data_items: Vec<Vec<u8>> = Vec::new();
    
    while i < script.len() {
        if script[i] == 0x68 {
            break;
        }
        
        let (data, consumed) = read_push_data(&script[i..])?;
        push_data_items.push(data);
        i += consumed;
    }
    
    if push_data_items.len() < 4 {
        return None;
    }
    
    let protocol = String::from_utf8_lossy(&push_data_items[0]);
    if protocol != "flacstore" {
        return None;
    }
    
    let metadata_str = String::from_utf8_lossy(&push_data_items[2]);
    let filename = if let Ok(metadata) = serde_json::from_str::<serde_json::Value>(&metadata_str) {
        metadata["filename"].as_str().unwrap_or("audio.flac").to_string()
    } else {
        "audio.flac".to_string()
    };
    
    let mut file_data = Vec::new();
    for chunk in &push_data_items[3..] {
        file_data.extend(chunk);
    }
    
    Some((file_data, filename))
}

fn parse_op_return_script(script: &[u8]) -> Option<(Vec<u8>, String)> {
    let mut i = 0;
    let mut push_data_items: Vec<Vec<u8>> = Vec::new();
    
    while i < script.len() {
        let (data, consumed) = read_push_data(&script[i..])?;
        push_data_items.push(data);
        i += consumed;
    }
    
    if push_data_items.len() < 4 {
        return None;
    }
    
    let filename = String::from_utf8_lossy(&push_data_items[2]).to_string();
    
    let mut file_data = Vec::new();
    for chunk in &push_data_items[3..] {
        file_data.extend(chunk);
    }
    
    Some((file_data, filename))
}

fn read_push_data(script: &[u8]) -> Option<(Vec<u8>, usize)> {
    if script.is_empty() {
        return None;
    }
    
    let opcode = script[0];
    
    if opcode <= 0x4b {
        let len = opcode as usize;
        if script.len() < 1 + len {
            return None;
        }
        Some((script[1..1 + len].to_vec(), 1 + len))
    } else if opcode == 0x4c {
        if script.len() < 2 {
            return None;
        }
        let len = script[1] as usize;
        if script.len() < 2 + len {
            return None;
        }
        Some((script[2..2 + len].to_vec(), 2 + len))
    } else if opcode == 0x4d {
        if script.len() < 3 {
            return None;
        }
        let len = u16::from_le_bytes([script[1], script[2]]) as usize;
        if script.len() < 3 + len {
            return None;
        }
        Some((script[3..3 + len].to_vec(), 3 + len))
    } else if opcode == 0x4e {
        if script.len() < 5 {
            return None;
        }
        let len = u32::from_le_bytes([script[1], script[2], script[3], script[4]]) as usize;
        if script.len() < 5 + len {
            return None;
        }
        Some((script[5..5 + len].to_vec(), 5 + len))
    } else {
        None
    }
}

fn read_varint(data: &[u8]) -> Option<(u64, usize)> {
    if data.is_empty() {
        return None;
    }
    
    let first = data[0];
    if first < 0xfd {
        Some((first as u64, 1))
    } else if first == 0xfd {
        if data.len() < 3 {
            return None;
        }
        let value = u16::from_le_bytes([data[1], data[2]]) as u64;
        Some((value, 3))
    } else if first == 0xfe {
        if data.len() < 5 {
            return None;
        }
        let value = u32::from_le_bytes([data[1], data[2], data[3], data[4]]) as u64;
        Some((value, 5))
    } else {
        if data.len() < 9 {
            return None;
        }
        let value = u64::from_le_bytes([
            data[1], data[2], data[3], data[4], data[5], data[6], data[7], data[8],
        ]);
        Some((value, 9))
    }
}
