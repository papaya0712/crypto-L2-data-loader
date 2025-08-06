mod config;
mod ws;
mod logging;

use config::load_config;
use logging::setup_logging;
use tracing::info;

#[tokio::main]
async fn main() {
    // config 
    let config = load_config();
    
    // logging
    setup_logging(&config.logging);
    info!("Logger initialized successfully");
    info!("Config loaded: {:?}", config);

    // WebSocket-Client
    ws::connect_ws(&config.websocket).await;
}
