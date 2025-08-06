use crate::config::WebSocketConfig;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::select;
use tokio::time::{interval, Duration};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tokio_tungstenite::tungstenite;
use bytes::Bytes;

pub async fn connect_ws(config: &WebSocketConfig) {
    println!("Connect to WebSocket @ {}", config.url);
    let (ws_stream, _) = connect_async(&config.url)
        .await
        .expect("❌ Connection error");

    println!("✅ Connected.");

    let (mut write, mut read) = ws_stream.split();

    let subscribe_msg = json!({
        "op": "subscribe",
        "args": ["orderbook.200.BTCUSDT"]
    });

    write
        .send(Message::Text(subscribe_msg.to_string().into()))
        .await
        .unwrap();

    println!("✅ Subscribed");

    let mut ping_interval = interval(Duration::from_secs(config.ping_interval_secs));

    loop {
        select! {
            _ = ping_interval.tick() => {
                if let Err(e) = send_ping(&mut write).await {
                    eprintln!("⚠️ Ping failed: {e}");
                    break;
                } else {
                    println!("Ping sent");
                }
            }
            Some(msg) = read.next() => {
                match msg {
                    Ok(Message::Text(text)) => println!("📥 {text}"),
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("⚠️ Error: {e}");
                        break;
                    }
                }
            }
        }
    }
}

async fn send_ping(
    write: &mut (impl SinkExt<Message, Error = tungstenite::Error> + Unpin),
) -> Result<(), tungstenite::Error> {
    write.send(Message::Ping(Bytes::new())).await
}