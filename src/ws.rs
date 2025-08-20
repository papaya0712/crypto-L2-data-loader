use crate::config::WebSocketConfig;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use tokio::{
    select,
    time::{interval, sleep, Duration},
};
use tokio_tungstenite::{connect_async, tungstenite};
use tokio_tungstenite::tungstenite::protocol::Message;

/// Endlosschleife mit einfachem Backoff für Reconnects
pub async fn connect_ws(config: &WebSocketConfig) {
    let mut attempt: u32 = 0;
    loop {
        match run_session(config).await {
            Ok(_) => break, // normaler Exit (CTRL-C)
            Err(e) => {
                eprintln!("WS session ended with error: {e}");
                attempt = attempt.saturating_add(1);
                let backoff_ms = (500u64).saturating_mul(1u64 << attempt.min(6));
                sleep(Duration::from_millis(backoff_ms)).await;
            }
        }
    }
}

async fn run_session(config: &WebSocketConfig) -> Result<(), tungstenite::Error> {
    println!("Connect to WebSocket @ {}", config.url);
    let (ws_stream, _) = connect_async(&config.url).await?;
    println!("✅ Connected.");

    let (mut write, mut read) = ws_stream.split();

    // Logdatei für Public Streams
    let mut file = open_log_file("public")?;

    let subscribe_msg = json!({
        "op": "subscribe",
        "args": [
            "orderbook.50.BTCUSDT",
            "publicTrade.BTCUSDT"
        ]
    });
    write.send(Message::Text(subscribe_msg.to_string().into())).await?;
    println!("✅ Subscribed");
    

    let mut ping_interval = interval(Duration::from_secs(config.ping_interval_secs));
    let mut last_seq: Option<i64> = None;

    loop {
        select! {
            _ = ping_interval.tick() => {
                if let Err(e) = send_ping(&mut write).await {
                    eprintln!("Ping failed: {e}");
                    break;
                } else {
                    println!("Ping sent");
                }
            }
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        handle_text(&text, &mut last_seq, &mut file);
                    }
                    Some(Ok(Message::Binary(_))) => {}
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = write.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Close(frame))) => {
                        eprintln!("WS close: {:?}", frame);
                        break;
                    }
                    Some(Ok(Message::Frame(_))) => {}
                    Some(Err(e)) => {
                        eprintln!("WS read error: {e}");
                        break;
                    }
                    None => {
                        eprintln!("WS stream ended");
                        break;
                    }
                }
            }
        }
    }

    let _ = write.send(Message::Close(None)).await;
    Ok(())
}

async fn send_ping(
    write: &mut (impl SinkExt<Message, Error = tungstenite::Error> + Unpin),
) -> Result<(), tungstenite::Error> {
    write.send(Message::Ping(Bytes::new())).await
}

/// Öffnet eine neue Log-Datei pro Session
fn open_log_file(prefix: &str) -> std::io::Result<File> {
    use chrono::Utc;

    let mut path = PathBuf::from("logs");
    std::fs::create_dir_all(&path)?;
    let ts = Utc::now().format("%Y%m%d_%H%M%S");
    path.push(format!("{prefix}_{ts}.log"));

    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct WsEnvelope {
    #[serde(default)] topic: Option<String>,
    #[serde(rename="type", default)] kind: Option<String>,
    #[serde(default)] ts: Option<i64>,
    #[serde(default)] cts: Option<i64>,
    #[serde(default)] data: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ObData {
    s: String,
    #[serde(default)] b: Vec<[String; 2]>, // bids: [price, size]
    #[serde(default)] a: Vec<[String; 2]>, // asks: [price, size]
    #[serde(default)] u: Option<i64>,      // update id
    #[serde(default)] seq: Option<i64>,    // sequence
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TradeData {
    s: String,
    #[serde(default)] v: Option<String>, // size
    #[serde(default)] p: Option<String>, // price
    #[serde(default)] T: Option<i64>,    // trade ts
    #[serde(default)] S: Option<String>, // side ("Buy"/"Sell")
}

fn handle_text(text: &str, last_seq: &mut Option<i64>, file: &mut File) {
    // System / Ack Messages abfangen
    let Ok(env) = serde_json::from_str::<WsEnvelope>(text) else {
        println!("{text}");
        let _ = writeln!(file, "{text}");
        return;
    };

    let Some(topic) = env.topic.as_deref() else {
        println!("{text}");
        let _ = writeln!(file, "{text}");
        return;
    };

    match topic {
        t if t.starts_with("orderbook.") => {
            if let Some(kind) = env.kind.as_deref() {
                match kind {
                    "snapshot" => {
                        if let Some(data) = &env.data {
                            if let Ok(ob) = serde_json::from_value::<ObData>(data.clone()) {
                                *last_seq = ob.seq;
                                println!("OB snapshot {} seq={:?}", ob.s, ob.seq);
                                let _ = writeln!(file, "SNAPSHOT: {}", text);
                            }
                        }
                    }
                    "delta" => {
                        if let Some(data) = &env.data {
                            if let Ok(ob) = serde_json::from_value::<ObData>(data.clone()) {
                                if let Some(seq) = ob.seq {
                                    if last_seq.map_or(true, |ls| seq > ls) {
                                        *last_seq = Some(seq);
                                        let _ = writeln!(file, "DELTA: {}", text);
                                    } else {
                                        eprintln!("Stale/duplicate delta ignored: seq={seq} last_seq={:?}", last_seq);
                                    }
                                }
                            }
                        }
                    }
                    other => {
                        println!("OB other type={other}: {text}");
                        let _ = writeln!(file, "{text}");
                    }
                }
            }
        }
        t if t.starts_with("publicTrade.") => {
            if let Some(data) = &env.data {
                println!("TRADE: {}", data);
                let _ = writeln!(file, "TRADE: {}", data);
            }
        }
        _ => {
            println!("{text}");
            let _ = writeln!(file, "{text}");
        }
    }
}
