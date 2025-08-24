use anyhow::Result;
use futures::{SinkExt, StreamExt};
use ordered_float::OrderedFloat;
use serde::Deserialize;
use std::{cmp::Reverse, collections::BTreeMap};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

#[derive(Deserialize)]
struct Snapshot {
    lastUpdateId: u64,
    bids: Vec<[String; 2]>,
    asks: Vec<[String; 2]>,
}

#[derive(Deserialize)]
struct WsMsg {
    #[serde(default)]                 
    publiclimitdepths: Option<Delta>,
}

#[derive(Deserialize)]
struct Delta {
    asksList: Vec<[String; 2]>,
    bidsList: Vec<[String; 2]>,
    version:  u64,
}

type BookSide = BTreeMap<OrderedFloat<f64>, f64>;
type RevSide  = BTreeMap<Reverse<OrderedFloat<f64>>, f64>;

#[tokio::main]
async fn main() -> Result<()> {
    let symbol = "BTCUSDT";

    let snap: Snapshot = reqwest::get(
        format!("https://api.mexc.com/api/v3/depth?symbol={symbol}&limit=1000")
    )
    .await?
    .json()
    .await?;

    let mut asks: BookSide = snap.asks.into_iter()
        .map(|[p,q]| (OrderedFloat(p.parse::<f64>().unwrap()), q.parse::<f64>().unwrap()))
        .collect();

    let mut bids: RevSide = snap.bids.into_iter()
        .map(|[p,q]| (Reverse(OrderedFloat(p.parse::<f64>().unwrap())), q.parse::<f64>().unwrap()))
        .collect();

    let mut last_ver = snap.lastUpdateId;
    println!("Snapshot OK – version {last_ver}");

    /* ===== 2. WebSocket verbinden + abonnieren ===== */
    let (mut ws, _) = connect_async("wss://wbs-api.mexc.com/ws").await?;
    let chan = format!("spot@public.limit.depth.v3.api.json@{symbol}@20");
    ws.send(Message::Text(
        serde_json::json!({ "method":"SUBSCRIPTION", "params":[chan] }).to_string()
    )).await?;

    /* ===== 3. Endlosschleife ===== */
    while let Some(msg) = ws.next().await {
        match msg? {
            /* ---- TEXT-Nachricht ---- */
            Message::Text(txt) => {
                println!("RAW: {txt}");                           // komplette Zeile loggen

                // Nur Tiefen-Pakete weiterverarbeiten
                let WsMsg { publiclimitdepths } = serde_json::from_str(&txt)?;
                let Some(d) = publiclimitdepths else { continue }; // Ack → skip

                // Versions-Check
                if d.version <= last_ver { continue; }
                if d.version != last_ver + 1 {
                    println!("Gap {last_ver} → {}", d.version);
                    break;                                        // Zum Demo-Zweck abbrechen
                }

                // Asks einarbeiten
                for [p,q] in d.asksList {
                    let price = OrderedFloat(p.parse::<f64>().unwrap());
                    let qty   = q.parse::<f64>().unwrap();
                    if qty == 0.0 { asks.remove(&price); } else { asks.insert(price, qty); }
                }
                // Bids einarbeiten
                for [p,q] in d.bidsList {
                    let price = Reverse(OrderedFloat(p.parse::<f64>().unwrap()));
                    let qty   = q.parse::<f64>().unwrap();
                    if qty == 0.0 { bids.remove(&price); } else { bids.insert(price, qty); }
                }
                last_ver = d.version;

                // Spread ausgeben
                if let (Some((ask,_)), Some((bid,_))) = (asks.first_key_value(), bids.first_key_value()) {
                    let spread = ask.0 - bid.0.0;
                    println!("bid {:>12.2} | ask {:>12.2} | spread {:.5}%",
                             bid.0.0, ask.0, spread / ask.0 * 100.0);
                }
            }

            /* ---- Ping vom Server ---- */
            Message::Ping(payload) => {
                println!("Ping vom Server");
                ws.send(Message::Pong(payload)).await?;
            }

            /* ---- alles andere ignorieren ---- */
            _ => {}
        }
    }

    Ok(())
}