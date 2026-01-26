use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub database_path: String,
    pub bsv_private_key: Option<String>,
    pub bsv_fee_rate: f64,
    pub bitails_api_url: String,
    pub bitails_api_key: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        Config {
            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: env::var("PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()
                .unwrap_or(8080),
            database_path: env::var("DATABASE_PATH")
                .unwrap_or_else(|_| "./data/upfile.db".to_string()),
            bsv_private_key: env::var("BSV_PRIVATE_KEY").ok(),
            bsv_fee_rate: env::var("BSV_FEE_RATE")
                .unwrap_or_else(|_| "0.002".to_string())
                .parse()
                .unwrap_or(0.002),
            bitails_api_url: env::var("BITAILS_API_URL")
                .unwrap_or_else(|_| "https://api.bitails.io".to_string()),
            bitails_api_key: env::var("BITAILS_API_KEY").ok(),
        }
    }
}
