mod config;
mod ws;
mod logging;
mod rest;
mod buffer; 

use config::load_config;
use logging::setup_logging;
use tracing::info;
use buffer::{create_buffer, writer_task, MarketEvent};

#[tokio::main]
async fn main() {
    let config = load_config();

    setup_logging(&config.logging);
    info!("Logger initialized successfully");
    info!("Config loaded: {:?}", config);

    // zentralen Buffer erstellen
    let (tx, rx) = create_buffer(10_000);

    // Writer Task starten (persistiert alles als JSONL)
    tokio::spawn(writer_task(rx, "bybit"));

    // REST: FundingRate holen und in Buffer pushen
    let rest_client = rest::RestClient::new("https://api.bybit.com");
    match rest_client.funding_rate("BTCUSDT").await {
        Ok(entry) => {
            let rate = entry.fundingRate.parse().unwrap_or(0.0);
            let ts = entry.fundingRateTimestamp.parse().unwrap_or(0);
            let _ = tx.send(MarketEvent::Funding {
                symbol: entry.symbol.clone(),
                rate,
                ts,
            }).await;

            println!("FundingRate {} = {} @ {}", entry.symbol, rate, ts);
        }
        Err(e) => eprintln!("Failed to fetch funding rate: {e}"),
    }

    ws::connect_ws(&config.websocket, tx).await;
}
