mod config;
mod ws;
mod logging;
mod rest;

use config::load_config;
use logging::setup_logging;
use tracing::info;

#[tokio::main]
async fn main() {

    let config = load_config();

    setup_logging(&config.logging);
    info!("Logger initialized successfully");
    info!("Config loaded: {:?}", config);

    let rest_client = rest::RestClient::new("https://api.bybit.com");

    match rest_client.funding_rate("BTCUSDT").await {
        Ok(entry) => {
            println!(
                "FundingRate {} = {} @ {}",
                entry.symbol, entry.fundingRate, entry.fundingRateTimestamp
            );
        }
        Err(e) => eprintln!("Failed to fetch funding rate: {e}"),
    }

    ws::connect_ws(&config.websocket).await;
}
