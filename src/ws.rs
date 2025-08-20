// ws.rs
use crate::config::WebSocketConfig;
use crate::buffer::{EventSender, MarketEvent};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::{
    select,
    time::{interval, sleep, Duration},
};
use tokio_tungstenite::{connect_async, tungstenite};
use tokio_tungstenite::tungstenite::protocol::Message;

use std::sync::Arc;

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

/// Connect entry: nimmt jetzt den EventSender entgegen
pub async fn connect_ws(config: &WebSocketConfig, tx: EventSender) {
    let tx = Arc::new(tx);
    let mut attempt: u32 = 0;
    loop {
        match run_session(config, tx.clone()).await {
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

async fn run_session(config: &WebSocketConfig, tx: Arc<EventSender>) -> Result<(), tungstenite::Error> {
    println!("Connect to WebSocket @ {}", config.url);
    let (ws_stream, _) = connect_async(&config.url).await?;
    println!("✅ Connected.");

    let (mut write, mut read) = ws_stream.split();

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
                        handle_text(&text, &mut last_seq, &tx);
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

fn handle_text(text: &str, last_seq: &mut Option<i64>, tx: &EventSender) {
    // Versuche, das Envelope zu parsen
    let env = match serde_json::from_str::<WsEnvelope>(text) {
        Ok(v) => v,
        Err(_) => {
            // Nicht-JSON oder unbekanntes Format — minimal loggen und return
            println!("{text}");
            return;
        }
    };

    let topic = match env.topic.as_deref() {
        Some(t) => t,
        None => {
            println!("{text}");
            return;
        }
    };

    match topic {
        t if t.starts_with("orderbook.") => {
            if let Some(kind) = env.kind.as_deref() {
                match kind {
                    "snapshot" => {
                        if let Some(data) = &env.data {
                            if let Ok(ob) = serde_json::from_value::<ObData>(data.clone()) {
                                let seq = ob.seq.unwrap_or(0);
                                *last_seq = Some(seq);

                                // Parse bids & asks
                                let bids = ob.b.into_iter().map(|p| {
                                    let price = p[0].parse::<f64>().unwrap_or(0.0);
                                    let size  = p[1].parse::<f64>().unwrap_or(0.0);
                                    (price, size)
                                }).collect::<Vec<_>>();

                                let asks = ob.a.into_iter().map(|p| {
                                    let price = p[0].parse::<f64>().unwrap_or(0.0);
                                    let size  = p[1].parse::<f64>().unwrap_or(0.0);
                                    (price, size)
                                }).collect::<Vec<_>>();

                                // LOG vor dem Move, damit ob.s noch verfügbar ist
                                println!("OB snapshot {} seq={:?}", ob.s, ob.seq);

                                // symbol wird hier bewegt (ob.s)
                                if let Err(e) = tx.try_send(MarketEvent::OrderBookSnapshot {
                                    symbol: ob.s,
                                    bids,
                                    asks,
                                    seq,
                                }) {
                                    eprintln!("Failed to send snapshot to buffer: {e}");
                                }
                            }
                        }
                    }

                    "delta" => {
                        if let Some(data) = &env.data {
                            if let Ok(ob) = serde_json::from_value::<ObData>(data.clone()) {
                                if let Some(seq) = ob.seq {
                                    if last_seq.map_or(true, |ls| seq > ls) {
                                        *last_seq = Some(seq);

                                        let bids = ob.b.into_iter().map(|p| {
                                            let price = p[0].parse::<f64>().unwrap_or(0.0);
                                            let size  = p[1].parse::<f64>().unwrap_or(0.0);
                                            (price, size)
                                        }).collect::<Vec<_>>();

                                        let asks = ob.a.into_iter().map(|p| {
                                            let price = p[0].parse::<f64>().unwrap_or(0.0);
                                            let size  = p[1].parse::<f64>().unwrap_or(0.0);
                                            (price, size)
                                        }).collect::<Vec<_>>();

                                        // Clone symbol hier, um keine Ownership-Probleme zu riskieren
                                        let symbol = ob.s.clone();
                                        if let Err(e) = tx.try_send(MarketEvent::OrderBookDelta {
                                            symbol,
                                            bids,
                                            asks,
                                            seq,
                                        }) {
                                            eprintln!("Buffer full or error while sending delta: {e}");
                                        }
                                    } else {
                                        eprintln!("Stale/duplicate delta ignored: seq={} last_seq={:?}", seq, last_seq);
                                    }
                                }
                            }
                        }
                    }

                    other => {
                        println!("OB other type={other}: {text}");
                    }
                }
            }
        }

        t if t.starts_with("publicTrade.") => {
            if let Some(data) = &env.data {
                // Bybit sendet oft ein Array von Trades
                if let Ok(trades) = serde_json::from_value::<Vec<TradeData>>(data.clone()) {
                    for tr in trades {
                        let price = tr.p.unwrap_or_else(|| "0".into()).parse::<f64>().unwrap_or(0.0);
                        let size  = tr.v.unwrap_or_else(|| "0".into()).parse::<f64>().unwrap_or(0.0);
                        let side  = tr.S.unwrap_or_default();
                        let ts    = tr.T.unwrap_or(0);
                        let symbol = tr.s.clone(); // clone, weil tr in den folgenden Aufruf gezogen wird

                        if let Err(e) = tx.try_send(MarketEvent::Trade {
                            symbol,
                            price,
                            size,
                            side,
                            ts,
                        }) {
                            eprintln!("Buffer full or error while sending trade: {e}");
                        }
                    }
                } else {
                    // fallback: unparsed payload loggen
                    println!("TRADE (unparsed): {}", data);
                }
            }
        }

        _ => {
            // Unbekanntes Topic: minimal loggen
            println!("{text}");
        }
    }
}
