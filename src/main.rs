// main.rs
use anyhow::Result;
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use ordered_float::OrderedFloat;
use prost::Message;
use std::{cmp::Reverse, collections::BTreeMap, time::Duration};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMsg};

type BookSide = BTreeMap<OrderedFloat<f64>, f64>;          // asks ↑
type RevSide  = BTreeMap<Reverse<OrderedFloat<f64>>, f64>; // bids ↓

pub mod mexc_pb {
    include!(concat!(env!("OUT_DIR"), "/mexc.pb.rs"));
}
use mexc_pb::PushDataV3ApiWrapper;

#[tokio::main]
async fn main() -> Result<()> {
    #[derive(serde::Deserialize)]
    struct Snapshot {
        #[serde(rename = "lastUpdateId")]
        last_update_id: u64,
        bids: Vec<[String; 2]>,
        asks: Vec<[String; 2]>,
    }

    let symbol = std::env::args().nth(1).unwrap_or_else(|| "BTCUSDT".to_string());
    let levels: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    // REST-Snapshot
    let snap: Snapshot = reqwest::get(format!(
        "https://api.mexc.com/api/v3/depth?symbol={symbol}&limit=1000"
    ))
    .await?
    .error_for_status()?
    .json()
    .await?;

    let mut asks: BookSide = BTreeMap::new();
    for [p, q] in snap.asks {
        asks.insert(OrderedFloat(p.parse::<f64>()?), q.parse::<f64>()?);
    }
    let mut bids: RevSide = BTreeMap::new();
    for [p, q] in snap.bids {
        bids.insert(Reverse(OrderedFloat(p.parse::<f64>()?)), q.parse::<f64>()?);
    }
    println!("REST snapshot loaded (lastUpdateId={})", snap.last_update_id);

    // WS verbinden
    let (mut ws, _) = connect_async("wss://wbs-api.mexc.com/ws").await?;
    let chan = format!("spot@public.limit.depth.v3.api.pb@{symbol}@{levels}");
    ws.send(WsMsg::Text(
        serde_json::json!({ "method": "SUBSCRIPTION", "params": [chan] }).to_string(),
    ))
    .await?;

    let mut ping_tick = tokio::time::interval(Duration::from_secs(30));
    let mut last_ws_ver: Option<u64> = None;

    loop {
        tokio::select! {
            _ = ping_tick.tick() => {
                let _ = ws.send(WsMsg::Ping(Vec::new())).await; // kein chrono nötig
            }
            msg = ws.next() => {
                match msg {
                    Some(Ok(WsMsg::Binary(buf))) => {
                        if let Err(e) = handle_snapshot_update(buf.into(), &mut asks, &mut bids, &mut last_ws_ver) {
                            eprintln!("decode error: {e}");
                        }
                    }
                    Some(Ok(WsMsg::Ping(p))) => { let _ = ws.send(WsMsg::Pong(p)).await; }
                    Some(Ok(WsMsg::Close(_))) => break,
                    Some(Ok(_)) => {}
                    Some(Err(e)) => { eprintln!("ws error: {e}"); break; }
                    None => break,
                }
            }
        }
    }
    Ok(())
}

// Jede WS-Nachricht = kompletter Top-N-Snapshot → Seiten ersetzen
fn handle_snapshot_update(
    buf: Bytes,
    asks: &mut BookSide,
    bids: &mut RevSide,
    last_ws_ver: &mut Option<u64>,
) -> Result<()> {
    use mexc_pb::push_data_v3_api_wrapper::Body;

    let wrapper = PushDataV3ApiWrapper::decode(buf)?;
    let Some(body) = wrapper.body else { return Ok(()); };
    let Body::PublicLimitDepths(delta) = body else { return Ok(()); };

    let ver: u64 = delta.version.parse()?;
    if let Some(prev) = *last_ws_ver {
        if ver <= prev { return Ok(()); }
    }
    *last_ws_ver = Some(ver);

    asks.clear();
    for item in delta.asks {
        let p = OrderedFloat(item.price.parse::<f64>()?);
        let q: f64 = item.quantity.parse()?;
        if q != 0.0 { asks.insert(p, q); }
    }
    bids.clear();
    for item in delta.bids {
        let p = Reverse(OrderedFloat(item.price.parse::<f64>()?));
        let q: f64 = item.quantity.parse()?;
        if q != 0.0 { bids.insert(p, q); }
    }

    if let (Some((ask, _)), Some((bid, _))) = (asks.first_key_value(), bids.first_key_value()) {
        if bid.0.0 <= ask.0 {
            let spread = ask.0 - bid.0.0;
            println!(
                "bid {:>12.2} | ask {:>12.2} | spread {:>8.5} %",
                bid.0.0, ask.0, spread / ask.0 * 100.0
            );
        } else {
            eprintln!("crossed book (tick verworfen)");
        }
    }
    Ok(())
}