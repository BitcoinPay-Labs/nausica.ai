use axum::{
    extract::State,
    response::Json,
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::AppState;
use crate::services::bsv::BsvService;

#[derive(Deserialize)]
pub struct GenerateWalletRequest {
    pub network: Option<String>, // "mainnet" or "testnet"
}

#[derive(Serialize)]
pub struct WalletResponse {
    pub success: bool,
    pub wif: Option<String>,
    pub address: Option<String>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct ImportWifRequest {
    pub wif: String,
    pub network: Option<String>,
}

#[derive(Deserialize)]
pub struct ImportMnemonicRequest {
    pub mnemonic: String,
    pub network: Option<String>,
}

#[derive(Deserialize)]
pub struct BalanceRequest {
    pub address: String,
    pub network: Option<String>,
}

#[derive(Serialize)]
pub struct BalanceResponse {
    pub success: bool,
    pub balance: Option<i64>,
    pub balance_bsv: Option<String>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct SendRequest {
    pub wif: String,
    pub to_address: String,
    pub amount_satoshis: i64,
    pub network: Option<String>,
}

#[derive(Serialize)]
pub struct SendResponse {
    pub success: bool,
    pub txid: Option<String>,
    pub error: Option<String>,
}

/// Generate a new wallet
pub async fn generate_wallet(
    State(_state): State<Arc<RwLock<AppState>>>,
    Json(req): Json<GenerateWalletRequest>,
) -> Json<WalletResponse> {
    let network = req.network.unwrap_or_else(|| "mainnet".to_string());
    
    // Generate keypair with correct network format
    // Mainnet: address starts with "1", WIF starts with "5", "K", or "L"
    // Testnet: address starts with "m" or "n", WIF starts with "c"
    let (wif, address) = BsvService::generate_keypair(&network);
    
    Json(WalletResponse {
        success: true,
        wif: Some(wif),
        address: Some(address),
        error: None,
    })
}

/// Import wallet from WIF
pub async fn import_wif(
    State(_state): State<Arc<RwLock<AppState>>>,
    Json(req): Json<ImportWifRequest>,
) -> Json<WalletResponse> {
    let network = req.network.unwrap_or_else(|| "mainnet".to_string());
    
    match BsvService::wif_to_address(&req.wif, &network) {
        Ok(address) => Json(WalletResponse {
            success: true,
            wif: Some(req.wif),
            address: Some(address),
            error: None,
        }),
        Err(e) => Json(WalletResponse {
            success: false,
            wif: None,
            address: None,
            error: Some(format!("Invalid WIF: {}", e)),
        }),
    }
}

/// Get balance for an address
pub async fn get_balance(
    State(state): State<Arc<RwLock<AppState>>>,
    Json(req): Json<BalanceRequest>,
) -> Json<BalanceResponse> {
    let network = req.network.unwrap_or_else(|| "mainnet".to_string());
    
    // Use WhatsOnChain API for testnet, Bitails for mainnet
    if network == "testnet" {
        match get_testnet_balance(&req.address).await {
            Ok((balance, balance_bsv)) => {
                Json(BalanceResponse {
                    success: true,
                    balance: Some(balance),
                    balance_bsv: Some(balance_bsv),
                    error: None,
                })
            }
            Err(e) => Json(BalanceResponse {
                success: false,
                balance: None,
                balance_bsv: None,
                error: Some(format!("Failed to get balance: {}", e)),
            }),
        }
    } else {
        let state = state.read().await;
        
        // Get UTXOs for the address
        match state.bitails.get_address_unspent(&req.address).await {
            Ok(utxos) => {
                let balance: i64 = utxos.iter().map(|u| u.satoshis).sum();
                let balance_bsv = format!("{:.8}", balance as f64 / 100_000_000.0);
                
                Json(BalanceResponse {
                    success: true,
                    balance: Some(balance),
                    balance_bsv: Some(balance_bsv),
                    error: None,
                })
            }
            Err(e) => Json(BalanceResponse {
                success: false,
                balance: None,
                balance_bsv: None,
                error: Some(format!("Failed to get balance: {}", e)),
            }),
        }
    }
}

/// Get testnet balance using WhatsOnChain API
async fn get_testnet_balance(address: &str) -> Result<(i64, String), String> {
    let client = reqwest::Client::new();
    let url = format!("https://api.whatsonchain.com/v1/bsv/test/address/{}/balance", address);
    
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    
    if !response.status().is_success() {
        return Err(format!("API error: {}", response.status()));
    }
    
    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Parse error: {}", e))?;
    
    let confirmed = json.get("confirmed").and_then(|v| v.as_i64()).unwrap_or(0);
    let unconfirmed = json.get("unconfirmed").and_then(|v| v.as_i64()).unwrap_or(0);
    let balance = confirmed + unconfirmed;
    let balance_bsv = format!("{:.8}", balance as f64 / 100_000_000.0);
    
    Ok((balance, balance_bsv))
}

/// Send BSV to an address
pub async fn send_bsv(
    State(state): State<Arc<RwLock<AppState>>>,
    Json(req): Json<SendRequest>,
) -> Json<SendResponse> {
    let network = req.network.unwrap_or_else(|| "mainnet".to_string());
    
    // Validate WIF and get sender address
    let sender_address = match BsvService::wif_to_address(&req.wif, &network) {
        Ok(addr) => addr,
        Err(e) => {
            return Json(SendResponse {
                success: false,
                txid: None,
                error: Some(format!("Invalid WIF: {}", e)),
            });
        }
    };
    
    let state_guard = state.read().await;
    
    // Get UTXOs based on network
    let utxos = if network == "testnet" {
        match get_testnet_utxos(&sender_address).await {
            Ok(u) => u,
            Err(e) => {
                return Json(SendResponse {
                    success: false,
                    txid: None,
                    error: Some(format!("Failed to get UTXOs: {}", e)),
                });
            }
        }
    } else {
        match state_guard.bitails.get_address_unspent(&sender_address).await {
            Ok(u) => u.iter().map(|utxo| TestnetUtxo {
                txid: utxo.txid.clone(),
                vout: utxo.vout,
                satoshis: utxo.satoshis,
            }).collect(),
            Err(e) => {
                return Json(SendResponse {
                    success: false,
                    txid: None,
                    error: Some(format!("Failed to get UTXOs: {}", e)),
                });
            }
        }
    };
    
    if utxos.is_empty() {
        return Json(SendResponse {
            success: false,
            txid: None,
            error: Some("No UTXOs available".to_string()),
        });
    }
    
    // Calculate total input
    let total_input: i64 = utxos.iter().map(|u| u.satoshis).sum();
    
    // Get scriptPubKey for sender address
    let sender_script = match BsvService::create_p2pkh_script(&sender_address) {
        Ok(s) => s,
        Err(e) => {
            return Json(SendResponse {
                success: false,
                txid: None,
                error: Some(format!("Failed to create sender script: {}", e)),
            });
        }
    };
    
    // Get scriptPubKey for recipient address
    let recipient_script = match BsvService::create_p2pkh_script(&req.to_address) {
        Ok(s) => s,
        Err(e) => {
            return Json(SendResponse {
                success: false,
                txid: None,
                error: Some(format!("Invalid recipient address: {}", e)),
            });
        }
    };
    
    // Prepare UTXOs for transaction
    let utxo_inputs: Vec<(String, u32, i64, Vec<u8>)> = utxos
        .iter()
        .map(|u| (u.txid.clone(), u.vout, u.satoshis, sender_script.clone()))
        .collect();
    
    // Calculate fee (estimate ~250 bytes for a simple tx)
    let fee = (250.0 * state_guard.bsv.fee_rate).ceil() as i64;
    
    // Check if we have enough funds
    if total_input < req.amount_satoshis + fee {
        return Json(SendResponse {
            success: false,
            txid: None,
            error: Some(format!(
                "Insufficient funds: have {} sats, need {} sats (including {} fee)",
                total_input,
                req.amount_satoshis + fee,
                fee
            )),
        });
    }
    
    // Calculate change
    let change = total_input - req.amount_satoshis - fee;
    
    // Create outputs
    let mut outputs: Vec<(Vec<u8>, i64)> = vec![
        (recipient_script, req.amount_satoshis),
    ];
    
    // Add change output if significant (> dust limit)
    if change > 546 {
        outputs.push((sender_script.clone(), change));
    }
    
    // Create transaction
    let raw_tx = match state_guard.bsv.create_transaction(&req.wif, &utxo_inputs, &outputs) {
        Ok(tx) => tx,
        Err(e) => {
            return Json(SendResponse {
                success: false,
                txid: None,
                error: Some(format!("Failed to create transaction: {}", e)),
            });
        }
    };
    
    // Broadcast transaction based on network
    if network == "testnet" {
        match broadcast_testnet_transaction(&raw_tx).await {
            Ok(txid) => Json(SendResponse {
                success: true,
                txid: Some(txid),
                error: None,
            }),
            Err(e) => Json(SendResponse {
                success: false,
                txid: None,
                error: Some(format!("Failed to broadcast: {}", e)),
            }),
        }
    } else {
        match state_guard.bitails.broadcast_transaction(&raw_tx).await {
            Ok(txid) => Json(SendResponse {
                success: true,
                txid: Some(txid),
                error: None,
            }),
            Err(e) => Json(SendResponse {
                success: false,
                txid: None,
                error: Some(format!("Failed to broadcast: {}", e)),
            }),
        }
    }
}

#[derive(Debug)]
struct TestnetUtxo {
    txid: String,
    vout: u32,
    satoshis: i64,
}

/// Get testnet UTXOs using WhatsOnChain API
async fn get_testnet_utxos(address: &str) -> Result<Vec<TestnetUtxo>, String> {
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
    
    let utxos: Vec<TestnetUtxo> = json
        .iter()
        .filter_map(|v| {
            let txid = v.get("tx_hash")?.as_str()?.to_string();
            let vout = v.get("tx_pos")?.as_u64()? as u32;
            let satoshis = v.get("value")?.as_i64()?;
            Some(TestnetUtxo { txid, vout, satoshis })
        })
        .collect();
    
    Ok(utxos)
}

/// Broadcast transaction to testnet using WhatsOnChain API
async fn broadcast_testnet_transaction(raw_tx: &str) -> Result<String, String> {
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
    
    // Remove quotes if present
    Ok(txid.trim_matches('"').to_string())
}
