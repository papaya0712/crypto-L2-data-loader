use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ExchangeConfig {
    pub symbols: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct WebSocketConfig {
    pub url: String,
    pub ping_interval_secs: u64,
}

#[derive(Debug, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub save_logs: bool,
    pub log_file_path: String,
    pub rewrite_last_logs: bool,
}

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub exchange: ExchangeConfig,
    pub websocket: WebSocketConfig,
    pub logging: LoggingConfig, 
}

pub fn load_config() -> AppConfig {
    config::Config::builder()
        .add_source(config::File::with_name("config"))
        .build()
        .expect("❌ Error while loading config")
        .try_deserialize()
        .expect("❌ Error when deserializing the configuration")
}
