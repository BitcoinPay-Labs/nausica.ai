use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::sleep;

use crate::models::JobStatus;
use crate::services::bsv::BsvService;
use crate::AppState;

pub async fn payment_watcher(state: Arc<RwLock<AppState>>) {
    loop {
        sleep(Duration::from_secs(5)).await;

        let pending_jobs = {
            let state = state.read().await;
            match state.db.get_pending_payment_jobs() {
                Ok(jobs) => jobs,
                Err(e) => {
                    tracing::error!("Failed to get pending jobs: {}", e);
                    continue;
                }
            }
        };

        for job in pending_jobs {
            if let (Some(address), Some(required_sats), Some(wif)) = (
                &job.payment_address,
                job.required_satoshis,
                &job.payment_wif,
            ) {
                let balance = {
                    let state = state.read().await;
                    state.bitails.get_address_balance(address).await
                };

                match balance {
                    Ok(bal) => {
                        // Check if we received enough (confirmed or unconfirmed)
                        if bal.summary >= required_sats {
                            tracing::info!(
                                "Payment detected for job {}: {} satoshis",
                                job.id,
                                bal.summary
                            );

                            // Update status to processing
                            {
                                let state = state.read().await;
                                let _ = state.db.update_job_status(
                                    &job.id,
                                    JobStatus::Processing,
                                    "Payment received, uploading to blockchain...",
                                );
                            }

                            // Start upload process
                            let state_clone = state.clone();
                            let job_id = job.id.clone();
                            let wif_clone = wif.clone();
                            let address_clone = address.clone();
                            let file_data = job.file_data.clone();
                            let filename = job.filename.clone();

                            tokio::spawn(async move {
                                process_upload(
                                    state_clone,
                                    job_id,
                                    wif_clone,
                                    address_clone,
                                    file_data,
                                    filename,
                                )
                                .await;
                            });
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to check balance for {}: {}", address, e);
                    }
                }
            }
        }
    }
}

async fn process_upload(
    state: Arc<RwLock<AppState>>,
    job_id: String,
    wif: String,
    address: String,
    file_data: Option<Vec<u8>>,
    filename: Option<String>,
) {
    let file_data = match file_data {
        Some(data) => data,
        None => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, "No file data found");
            return;
        }
    };

    let filename = filename.unwrap_or_else(|| "unknown".to_string());

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

    // Create OP_RETURN data
    let protocol_tag = b"upfile";
    let filename_bytes = filename.as_bytes();
    let mime_type = mime_guess::from_path(&filename)
        .first_or_octet_stream()
        .to_string();
    let mime_bytes = mime_type.as_bytes();

    let op_return_script = BsvService::create_op_return_script(&[
        protocol_tag,
        filename_bytes,
        mime_bytes,
        &file_data,
    ]);

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
        Ok(response) => {
            let state = state.read().await;
            let _ = state.db.update_job_complete(&job_id, &response.txid, None);
            tracing::info!("Upload complete for job {}: txid={}", job_id, response.txid);
        }
        Err(e) => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, &format!("Broadcast failed: {}", e));
        }
    }
}

pub async fn process_download(
    state: Arc<RwLock<AppState>>,
    job_id: String,
    txid: String,
) {
    // Update progress
    {
        let state = state.read().await;
        let _ = state.db.update_job_progress(&job_id, 10.0, "Fetching transaction...");
    }

    // Get transaction info
    let tx_info = {
        let state = state.read().await;
        state.bitails.get_transaction(&txid).await
    };

    let tx_info = match tx_info {
        Ok(info) => info,
        Err(e) => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, &format!("Failed to get tx: {}", e));
            return;
        }
    };

    // Update progress
    {
        let state = state.read().await;
        let _ = state.db.update_job_progress(&job_id, 30.0, "Downloading data...");
    }

    // Download the raw transaction
    let tx_raw = {
        let state = state.read().await;
        state.bitails.download_tx_raw(&txid).await
    };

    let tx_raw = match tx_raw {
        Ok(data) => data,
        Err(e) => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, &format!("Failed to download: {}", e));
            return;
        }
    };

    // Extract OP_RETURN data from raw transaction
    let output_data = match extract_op_return_from_tx(&tx_raw) {
        Ok(data) => data,
        Err(e) => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, &format!("Failed to extract data: {}", e));
            return;
        }
    };

    // Update progress
    {
        let state = state.read().await;
        let _ = state.db.update_job_progress(&job_id, 60.0, "Parsing data...");
    }

    // Parse the OP_RETURN script
    let parsed = parse_op_return_script(&output_data);

    match parsed {
        Ok((filename, _mime_type, file_data)) => {
            // Save file to downloads directory
            let download_path = format!("./data/downloads/{}", job_id);
            std::fs::create_dir_all("./data/downloads").ok();

            let file_path = format!("{}/{}", download_path, filename);
            std::fs::create_dir_all(&download_path).ok();

            if let Err(e) = std::fs::write(&file_path, &file_data) {
                let state = state.read().await;
                let _ = state.db.update_job_error(&job_id, &format!("Failed to save file: {}", e));
                return;
            }

            let download_link = format!("/downloads/{}/{}", job_id, filename);

            let state = state.read().await;
            let _ = state.db.update_job_complete(&job_id, &txid, Some(&download_link));
            tracing::info!("Download complete for job {}", job_id);
        }
        Err(e) => {
            let state = state.read().await;
            let _ = state.db.update_job_error(&job_id, &format!("Failed to parse data: {}", e));
        }
    }
}

fn parse_op_return_script(script: &[u8]) -> Result<(String, String, Vec<u8>), String> {
    let mut pos = 0;

    // Skip OP_FALSE OP_RETURN if present
    if script.len() > 2 && script[0] == 0x00 && script[1] == 0x6a {
        pos = 2;
    } else if script.len() > 1 && script[0] == 0x6a {
        pos = 1;
    }

    let mut parts: Vec<Vec<u8>> = Vec::new();

    while pos < script.len() {
        let (data, new_pos) = read_push_data(script, pos)?;
        parts.push(data);
        pos = new_pos;
    }

    if parts.len() < 4 {
        return Err(format!("Expected 4 parts, got {}", parts.len()));
    }

    // parts[0] = protocol tag ("upfile")
    // parts[1] = filename
    // parts[2] = mime type
    // parts[3] = file data

    let filename = String::from_utf8(parts[1].clone())
        .unwrap_or_else(|_| "unknown".to_string());
    let mime_type = String::from_utf8(parts[2].clone())
        .unwrap_or_else(|_| "application/octet-stream".to_string());
    let file_data = parts[3].clone();

    Ok((filename, mime_type, file_data))
}

fn read_push_data(script: &[u8], pos: usize) -> Result<(Vec<u8>, usize), String> {
    if pos >= script.len() {
        return Err("Unexpected end of script".to_string());
    }

    let opcode = script[pos];

    if opcode <= 75 {
        // Direct push
        let len = opcode as usize;
        let end = pos + 1 + len;
        if end > script.len() {
            return Err("Data extends beyond script".to_string());
        }
        Ok((script[pos + 1..end].to_vec(), end))
    } else if opcode == 0x4c {
        // OP_PUSHDATA1
        if pos + 1 >= script.len() {
            return Err("Missing length byte".to_string());
        }
        let len = script[pos + 1] as usize;
        let end = pos + 2 + len;
        if end > script.len() {
            return Err("Data extends beyond script".to_string());
        }
        Ok((script[pos + 2..end].to_vec(), end))
    } else if opcode == 0x4d {
        // OP_PUSHDATA2
        if pos + 2 >= script.len() {
            return Err("Missing length bytes".to_string());
        }
        let len = u16::from_le_bytes([script[pos + 1], script[pos + 2]]) as usize;
        let end = pos + 3 + len;
        if end > script.len() {
            return Err("Data extends beyond script".to_string());
        }
        Ok((script[pos + 3..end].to_vec(), end))
    } else if opcode == 0x4e {
        // OP_PUSHDATA4
        if pos + 4 >= script.len() {
            return Err("Missing length bytes".to_string());
        }
        let len = u32::from_le_bytes([
            script[pos + 1],
            script[pos + 2],
            script[pos + 3],
            script[pos + 4],
        ]) as usize;
        let end = pos + 5 + len;
        if end > script.len() {
            return Err("Data extends beyond script".to_string());
        }
        Ok((script[pos + 5..end].to_vec(), end))
    } else {
        Err(format!("Unknown opcode: 0x{:02x}", opcode))
    }
}

fn extract_op_return_from_tx(tx_raw: &[u8]) -> Result<Vec<u8>, String> {
    // Parse raw transaction to find OP_RETURN output
    // Transaction format:
    // - 4 bytes: version
    // - varint: input count
    // - inputs
    // - varint: output count
    // - outputs (each: 8 bytes value + varint script length + script)
    
    let mut pos = 4; // Skip version
    
    // Read input count
    let (input_count, new_pos) = read_varint(tx_raw, pos)?;
    pos = new_pos;
    
    // Skip inputs
    for _ in 0..input_count {
        pos += 32; // txid
        pos += 4;  // vout
        let (script_len, new_pos) = read_varint(tx_raw, pos)?;
        pos = new_pos + script_len as usize;
        pos += 4; // sequence
    }
    
    // Read output count
    let (output_count, new_pos) = read_varint(tx_raw, pos)?;
    pos = new_pos;
    
    // Find OP_RETURN output
    for _ in 0..output_count {
        pos += 8; // value (8 bytes)
        let (script_len, new_pos) = read_varint(tx_raw, pos)?;
        pos = new_pos;
        
        let script_end = pos + script_len as usize;
        if script_end > tx_raw.len() {
            return Err("Script extends beyond transaction".to_string());
        }
        
        let script = &tx_raw[pos..script_end];
        
        // Check if this is an OP_RETURN output
        if script.len() > 1 && (script[0] == 0x6a || (script.len() > 2 && script[0] == 0x00 && script[1] == 0x6a)) {
            return Ok(script.to_vec());
        }
        
        pos = script_end;
    }
    
    Err("No OP_RETURN output found".to_string())
}

fn read_varint(data: &[u8], pos: usize) -> Result<(u64, usize), String> {
    if pos >= data.len() {
        return Err("Unexpected end of data".to_string());
    }
    
    let first = data[pos];
    
    if first < 0xfd {
        Ok((first as u64, pos + 1))
    } else if first == 0xfd {
        if pos + 2 >= data.len() {
            return Err("Unexpected end of data".to_string());
        }
        let val = u16::from_le_bytes([data[pos + 1], data[pos + 2]]) as u64;
        Ok((val, pos + 3))
    } else if first == 0xfe {
        if pos + 4 >= data.len() {
            return Err("Unexpected end of data".to_string());
        }
        let val = u32::from_le_bytes([data[pos + 1], data[pos + 2], data[pos + 3], data[pos + 4]]) as u64;
        Ok((val, pos + 5))
    } else {
        if pos + 8 >= data.len() {
            return Err("Unexpected end of data".to_string());
        }
        let val = u64::from_le_bytes([
            data[pos + 1], data[pos + 2], data[pos + 3], data[pos + 4],
            data[pos + 5], data[pos + 6], data[pos + 7], data[pos + 8],
        ]);
        Ok((val, pos + 9))
    }
}
