// src/ws.rs

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

pub async fn connect_ws() {
    let ws_url = "wss://stream.bybit.com/v5/public/linear";
    
    println!("📡 Verbinde zu Bybit WebSocket...");
    let (ws_stream, _) = connect_async(ws_url).await.expect("❌ Connection error");
    println!("✅ Connected.");
    
    let (mut write, mut read) = ws_stream.split();
    
    let subscribe_msg = json!({
        "op": "subscribe",
        "args": ["orderbook.1.BTCUSDT"]
    });
    
    write.send(Message::Text(subscribe_msg.to_string().into())).await.unwrap();
    println!("📨 Subscribed");
    
    while let Some(msg) = read.next().await {
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
