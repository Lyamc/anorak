use once_cell::sync::Lazy;
use std::env;

pub struct Config {
    pub port: u16,
    /// Torznab results URL, for example Lodestarr's
    /// `/api/v2.0/indexers/all/results/torznab`.
    pub jackett_url: String,
    pub jackett_apikey: String,
    /// rqbit HTTP API, for example `http://127.0.0.1:9030`.
    pub rqbit_url: String,
}

pub static CONFIG: Lazy<Config> = Lazy::new(|| {
    Config { 
        port: env::var("ANORAK_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(9341),
        jackett_url: env::var("JACKETT_URL").expect("Define the JACKETT_URL environment variable"),
        jackett_apikey: env::var("JACKETT_APIKEY").expect("Define the JACKETT_APIKEY environment variable"),
        rqbit_url: env::var("RQBIT_URL").unwrap_or_else(|_| "http://127.0.0.1:9030".to_string()),
    }
});
