// main.rs
use anyhow::{anyhow, Result};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use ordered_float::OrderedFloat;
use prost::Message;
use std::{cmp::Reverse, collections::BTreeMap, time::Duration};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMsg};

type BookSide = BTreeMap<OrderedFloat<f64>, f64>;
type RevSide  = BTreeMap<Reverse<OrderedFloat<f64>>, f64>;


pub mod mexc_pb {
    include!(concat!(env!("OUT_DIR"), "/mexc.pb.rs"));
}
use mexc_pb::PushDataV3ApiWrapper;

#[derive(serde::Deserialize)]
struct Snapshot {
    #[serde(rename = "lastUpdateId")]
    last_update_id: u64,
    bids: Vec<[String; 2]>,
    asks: Vec<[String; 2]>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let symbol = std::env::args().nth(1).unwrap_or_else(|| "BTCUSDT".to_string());

    // Snapshot 1000 Level
    let mut asks: BookSide = BTreeMap::new();
    let mut bids: RevSide = BTreeMap::new();
    let mut snap_ver = reload_snapshot(&symbol, &mut asks, &mut bids).await?;
    println!("REST snapshot loaded (lastUpdateId={})", snap_ver);
    println!("levels: asks={} bids={}", asks.len(), bids.len());




    // WS: aggre.depth @10ms
    let (mut ws, _) = connect_async("wss://wbs-api.mexc.com/ws").await?;
    let chan = format!("spot@public.aggre.depth.v3.api.pb@100ms@{symbol}");
    ws.send(WsMsg::Text(
        serde_json::json!({ "method": "SUBSCRIPTION", "params": [chan] }).to_string(),
    ))
    .await?;

    let mut ping_tick = tokio::time::interval(Duration::from_secs(30));
    let mut last_to_ver: Option<u64> = None;

    loop {
        tokio::select! {
            _ = ping_tick.tick() => { let _ = ws.send(WsMsg::Ping(Vec::new())).await; }
            msg = ws.next() => {
                match msg {
                    Some(Ok(WsMsg::Binary(buf))) => {
                        match handle_diff_update(buf.into(), &mut asks, &mut bids, &mut snap_ver, &mut last_to_ver) {
                            Ok(()) => {}
                            Err(_) => {
                                if let Ok(new_ver) = reload_snapshot(&symbol, &mut asks, &mut bids).await {
                                    snap_ver = new_ver;
                                    last_to_ver = None;
                                    eprintln!("resynced via REST (lastUpdateId={})", snap_ver);
                                }
                            }
                        }
                    }
                    Some(Ok(WsMsg::Ping(p))) => { let _ = ws.send(WsMsg::Pong(p)).await; }
                    Some(Ok(WsMsg::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(e)) => { eprintln!("ws error: {e}"); break; }
                }
            }
        }
    }
    Ok(())
}

async fn reload_snapshot(symbol: &str, asks: &mut BookSide, bids: &mut RevSide) -> Result<u64> {
    let snap: Snapshot = reqwest::get(format!(
        "https://api.mexc.com/api/v3/depth?symbol={symbol}&limit=1000"
    ))
    .await?
    .error_for_status()?
    .json()
    .await?;

    asks.clear();
    for [p, q] in snap.asks {
        asks.insert(OrderedFloat(p.parse::<f64>()?), q.parse::<f64>()?);
    }
    bids.clear();
    for [p, q] in snap.bids {
        bids.insert(Reverse(OrderedFloat(p.parse::<f64>()?)), q.parse::<f64>()?);
    }
    Ok(snap.last_update_id)
}

fn handle_diff_update(
    buf: Bytes,
    asks: &mut BookSide,
    bids: &mut RevSide,
    snap_ver: &mut u64,
    last_to_ver: &mut Option<u64>,
) -> Result<()> {
    use mexc_pb::push_data_v3_api_wrapper::Body;

    let wrapper = PushDataV3ApiWrapper::decode(buf)?;
    let Some(body) = wrapper.body else { return Ok(()); };
    let Body::PublicAggreDepths(delta) = body else { return Ok(()); };

    let from_v: u64 = delta.from_version.parse()?;
    let to_v: u64   = delta.to_version.parse()?;

    if to_v < *snap_ver { return Ok(()); }

    if last_to_ver.is_none() {
        if from_v > *snap_ver + 1 { return Err(anyhow!("gap after snapshot")); }
    } else if let Some(prev) = *last_to_ver {
        if from_v != prev + 1 { return Err(anyhow!("sequence gap")); }
    }

    // Felder heißen asks / bids (nicht *_list)
    for it in delta.asks {
        let p = OrderedFloat(it.price.parse::<f64>()?);
        let q: f64 = it.quantity.parse()?;
        if q == 0.0 { asks.remove(&p); } else { asks.insert(p, q); }
    }
    for it in delta.bids {
        let p = Reverse(OrderedFloat(it.price.parse::<f64>()?));
        let q: f64 = it.quantity.parse()?;
        if q == 0.0 { bids.remove(&p); } else { bids.insert(p, q); }
    }

    *snap_ver = to_v;
    *last_to_ver = Some(to_v);

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
