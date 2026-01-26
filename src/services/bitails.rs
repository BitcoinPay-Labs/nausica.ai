use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct AddressBalance {
    pub address: String,
    pub confirmed: i64,
    pub unconfirmed: i64,
    pub summary: i64,
    pub count: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Utxo {
    pub txid: String,
    pub vout: u32,
    pub satoshis: i64,
    pub blockheight: Option<i64>,
    pub confirmations: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UnspentResponse {
    pub address: String,
    pub unspent: Vec<Utxo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BroadcastResponse {
    pub txid: Option<String>,
    pub error: Option<BroadcastError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BroadcastError {
    pub code: Option<i32>,
    pub message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TransactionOutput {
    pub index: u32,
    #[serde(rename = "type")]
    pub output_type: Option<String>,
    pub satoshis: Option<i64>,
    pub scripthash: Option<String>,
    #[serde(rename = "scriptSize")]
    pub script_size: Option<i64>,
    pub script: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Transaction {
    pub txid: String,
    pub blockhash: Option<String>,
    pub blockheight: Option<i64>,
    pub confirmations: Option<i64>,
    pub time: Option<i64>,
    pub size: Option<i64>,
    pub fee: Option<i64>,
    #[serde(rename = "inputsCount")]
    pub inputs_count: Option<i64>,
    #[serde(rename = "outputsCount")]
    pub outputs_count: Option<i64>,
    pub outputs: Option<Vec<TransactionOutput>>,
}

pub struct BitailsClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
}

impl BitailsClient {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        BitailsClient {
            client: Client::new(),
            base_url,
            api_key,
        }
    }

    fn build_request(&self, url: &str) -> reqwest::RequestBuilder {
        let mut req = self.client.get(url);
        if let Some(ref key) = self.api_key {
            req = req.header("apikey", key);
        }
        req
    }

    fn build_post_request(&self, url: &str) -> reqwest::RequestBuilder {
        let mut req = self.client.post(url);
        if let Some(ref key) = self.api_key {
            req = req.header("apikey", key);
        }
        req
    }

    pub async fn get_address_balance(&self, address: &str) -> Result<AddressBalance, String> {
        let url = format!("{}/address/{}/balance", self.base_url, address);
        let response = self
            .build_request(&url)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("API error: {}", response.status()));
        }

        response
            .json::<AddressBalance>()
            .await
            .map_err(|e| format!("Parse error: {}", e))
    }

    pub async fn get_address_unspent(&self, address: &str) -> Result<Vec<Utxo>, String> {
        let url = format!("{}/address/{}/unspent", self.base_url, address);
        let response = self
            .build_request(&url)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("API error: {}", response.status()));
        }

        // Bitails returns a single object, not an array
        let result: UnspentResponse = response
            .json()
            .await
            .map_err(|e| format!("Parse error: {}", e))?;

        Ok(result.unspent)
    }

    pub async fn broadcast_transaction(&self, raw_tx_hex: &str) -> Result<String, String> {
        let url = format!("{}/tx/broadcast", self.base_url);
        let response = self
            .build_post_request(&url)
            .header("Content-Type", "application/json")
            .body(format!(r#"{{"raw":"{}"}}"#, raw_tx_hex))
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        let response_text = response.text().await.unwrap_or_default();
        
        // Try to parse as JSON
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response_text) {
            // Check for error
            if let Some(error) = json.get("error") {
                let error_msg = error.get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown error");
                return Err(format!("Broadcast failed: {}", error_msg));
            }
            
            // Get txid
            if let Some(txid) = json.get("txid").and_then(|t| t.as_str()) {
                return Ok(txid.to_string());
            }
        }
        
        // If response looks like a txid (64 hex chars), return it
        let trimmed = response_text.trim().trim_matches('"');
        if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(trimmed.to_string());
        }
        
        Err(format!("Unexpected response: {}", response_text))
    }
    pub async fn get_transaction(&self, txid: &str) -> Result<Transaction, String> {
        let url = format!("{}/tx/{}", self.base_url, txid);
        let response = self
            .build_request(&url)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("API error: {}", response.status()));
        }

        response
            .json::<Transaction>()
            .await
            .map_err(|e| format!("Parse error: {}", e))
    }

    pub async fn download_tx_output(&self, txid: &str, output_index: u32) -> Result<Vec<u8>, String> {
        let url = format!("{}/download/tx/{}/output/{}", self.base_url, txid, output_index);
        let response = self
            .build_request(&url)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("API error: {}", response.status()));
        }

        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("Download error: {}", e))
    }

    pub async fn download_tx_raw(&self, txid: &str) -> Result<Vec<u8>, String> {
        let url = format!("{}/download/tx/{}", self.base_url, txid);
        let response = self
            .build_request(&url)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("API error: {}", response.status()));
        }

        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("Download error: {}", e))
    }
}
