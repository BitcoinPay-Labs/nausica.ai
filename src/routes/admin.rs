use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::db::AdminConfig;
use crate::services::bsv::BsvService;
use crate::AppState;

// Admin key for authentication (should be set via environment variable)
fn get_admin_key() -> String {
    std::env::var("ADMIN_KEY").unwrap_or_else(|_| "nausica-admin-2024".to_string())
}

/// Admin panel page
pub async fn admin_page() -> Html<String> {
    let html = include_str!("../../templates/admin.html");
    Html(html.to_string())
}

#[derive(Deserialize)]
pub struct AdminAuthRequest {
    pub key: String,
}

#[derive(Serialize)]
pub struct AdminAuthResponse {
    pub success: bool,
    pub error: Option<String>,
}

/// Verify admin key
pub async fn verify_admin_key(
    Json(req): Json<AdminAuthRequest>,
) -> Json<AdminAuthResponse> {
    let admin_key = get_admin_key();
    
    if req.key == admin_key {
        Json(AdminAuthResponse {
            success: true,
            error: None,
        })
    } else {
        Json(AdminAuthResponse {
            success: false,
            error: Some("Invalid admin key".to_string()),
        })
    }
}

#[derive(Serialize)]
pub struct AdminConfigResponse {
    pub success: bool,
    pub admin_pay_mainnet: bool,
    pub admin_pay_testnet: bool,
    pub mainnet_address: Option<String>,
    pub testnet_address: Option<String>,
    pub mainnet_balance: Option<i64>,
    pub testnet_balance: Option<i64>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct GetAdminConfigRequest {
    pub key: String,
}

/// Get admin configuration
pub async fn get_admin_config(
    State(state): State<Arc<RwLock<AppState>>>,
    Json(req): Json<GetAdminConfigRequest>,
) -> impl IntoResponse {
    let admin_key = get_admin_key();
    
    if req.key != admin_key {
        return (
            StatusCode::UNAUTHORIZED,
            Json(AdminConfigResponse {
                success: false,
                admin_pay_mainnet: false,
                admin_pay_testnet: false,
                mainnet_address: None,
                testnet_address: None,
                mainnet_balance: None,
                testnet_balance: None,
                error: Some("Invalid admin key".to_string()),
            }),
        ).into_response();
    }

    let state = state.read().await;
    
    match state.db.get_admin_config() {
        Ok(config) => {
            // Get addresses from WIFs
            let mainnet_address = config.mainnet_wif.as_ref().and_then(|wif| {
                BsvService::wif_to_address(wif, "mainnet").ok()
            });
            let testnet_address = config.testnet_wif.as_ref().and_then(|wif| {
                BsvService::wif_to_address(wif, "testnet").ok()
            });

            Json(AdminConfigResponse {
                success: true,
                admin_pay_mainnet: config.admin_pay_mainnet,
                admin_pay_testnet: config.admin_pay_testnet,
                mainnet_address,
                testnet_address,
                mainnet_balance: None, // Will be fetched separately
                testnet_balance: None, // Will be fetched separately
                error: None,
            }).into_response()
        }
        Err(e) => {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(AdminConfigResponse {
                    success: false,
                    admin_pay_mainnet: false,
                    admin_pay_testnet: false,
                    mainnet_address: None,
                    testnet_address: None,
                    mainnet_balance: None,
                    testnet_balance: None,
                    error: Some(format!("Database error: {}", e)),
                }),
            ).into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct UpdateAdminConfigRequest {
    pub key: String,
    pub admin_pay_mainnet: Option<bool>,
    pub admin_pay_testnet: Option<bool>,
    pub mainnet_wif: Option<String>,
    pub testnet_wif: Option<String>,
}

#[derive(Serialize)]
pub struct UpdateAdminConfigResponse {
    pub success: bool,
    pub error: Option<String>,
}

/// Update admin configuration
pub async fn update_admin_config(
    State(state): State<Arc<RwLock<AppState>>>,
    Json(req): Json<UpdateAdminConfigRequest>,
) -> impl IntoResponse {
    let admin_key = get_admin_key();
    
    if req.key != admin_key {
        return (
            StatusCode::UNAUTHORIZED,
            Json(UpdateAdminConfigResponse {
                success: false,
                error: Some("Invalid admin key".to_string()),
            }),
        ).into_response();
    }

    let state = state.read().await;
    
    // Get current config
    let current_config = match state.db.get_admin_config() {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(UpdateAdminConfigResponse {
                    success: false,
                    error: Some(format!("Database error: {}", e)),
                }),
            ).into_response();
        }
    };

    // Update config with new values
    let new_config = AdminConfig {
        admin_pay_mainnet: req.admin_pay_mainnet.unwrap_or(current_config.admin_pay_mainnet),
        admin_pay_testnet: req.admin_pay_testnet.unwrap_or(current_config.admin_pay_testnet),
        mainnet_wif: req.mainnet_wif.or(current_config.mainnet_wif),
        testnet_wif: req.testnet_wif.or(current_config.testnet_wif),
    };

    match state.db.update_admin_config(&new_config) {
        Ok(_) => Json(UpdateAdminConfigResponse {
            success: true,
            error: None,
        }).into_response(),
        Err(e) => {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(UpdateAdminConfigResponse {
                    success: false,
                    error: Some(format!("Failed to update config: {}", e)),
                }),
            ).into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct GetWalletBalanceRequest {
    pub key: String,
    pub network: String,
}

#[derive(Serialize)]
pub struct GetWalletBalanceResponse {
    pub success: bool,
    pub address: Option<String>,
    pub balance: Option<i64>,
    pub error: Option<String>,
}

/// Get admin wallet balance
pub async fn get_admin_wallet_balance(
    State(state): State<Arc<RwLock<AppState>>>,
    Json(req): Json<GetWalletBalanceRequest>,
) -> impl IntoResponse {
    let admin_key = get_admin_key();
    
    if req.key != admin_key {
        return (
            StatusCode::UNAUTHORIZED,
            Json(GetWalletBalanceResponse {
                success: false,
                address: None,
                balance: None,
                error: Some("Invalid admin key".to_string()),
            }),
        ).into_response();
    }

    let config = {
        let state = state.read().await;
        match state.db.get_admin_config() {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(GetWalletBalanceResponse {
                        success: false,
                        address: None,
                        balance: None,
                        error: Some(format!("Database error: {}", e)),
                    }),
                ).into_response();
            }
        }
    };

    let wif = if req.network == "testnet" {
        config.testnet_wif
    } else {
        config.mainnet_wif
    };

    let wif = match wif {
        Some(w) => w,
        None => {
            return Json(GetWalletBalanceResponse {
                success: true,
                address: None,
                balance: None,
                error: Some("No wallet configured for this network".to_string()),
            }).into_response();
        }
    };

    let address = match BsvService::wif_to_address(&wif, &req.network) {
        Ok(addr) => addr,
        Err(e) => {
            return Json(GetWalletBalanceResponse {
                success: false,
                address: None,
                balance: None,
                error: Some(format!("Invalid WIF: {}", e)),
            }).into_response();
        }
    };

    // Fetch balance based on network
    let balance = if req.network == "testnet" {
        // Use WhatsOnChain API for testnet
        match fetch_testnet_balance(&address).await {
            Ok(b) => Some(b),
            Err(_) => None,
        }
    } else {
        // Use Bitails API for mainnet
        let state = state.read().await;
        match state.bitails.get_address_balance(&address).await {
            Ok(b) => Some(b.confirmed + b.unconfirmed),
            Err(_) => None,
        }
    };

    Json(GetWalletBalanceResponse {
        success: true,
        address: Some(address),
        balance,
        error: None,
    }).into_response()
}

async fn fetch_testnet_balance(address: &str) -> Result<i64, String> {
    let url = format!("https://api.whatsonchain.com/v1/bsv/test/address/{}/balance", address);
    
    let client = reqwest::Client::new();
    let response = client.get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let json: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    
    let confirmed = json["confirmed"].as_i64().unwrap_or(0);
    let unconfirmed = json["unconfirmed"].as_i64().unwrap_or(0);
    
    Ok(confirmed + unconfirmed)
}
