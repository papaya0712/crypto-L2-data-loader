// main.rs
use anyhow::Result;
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use ordered_float::OrderedFloat;
use prost::Message;
use std::{cmp::Reverse, collections::BTreeMap};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMsg};

type BookSide = BTreeMap<OrderedFloat<f64>, f64>; // asks: aufsteigend
type RevSide = BTreeMap<Reverse<OrderedFloat<f64>>, f64>; // bids: absteigend

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

    let symbol = "BTCUSDT";

    // REST-Snapshot (groß) einmalig laden
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

    let mut last_ver = snap.last_update_id;
    println!("snapshot version {last_ver}");

    // WS verbinden und Top-N-Stream abonnieren
    let (mut ws, _) = connect_async("wss://wbs-api.mexc.com/ws").await?;
    let chan = format!("spot@public.limit.depth.v3.api.pb@{symbol}@20"); // Top-20
    ws.send(WsMsg::Text(
        serde_json::json!({ "method": "SUBSCRIPTION", "params": [chan] }).to_string(),
    ))
    .await?;

    // WS-Loop
    while let Some(msg) = ws.next().await {
        match msg? {
            WsMsg::Binary(buf) => {
                if let Err(e) = handle_snapshot_update(buf.into(), &mut asks, &mut bids, &mut last_ver) {
                    eprintln!("decode error: {e}");
                }
            }
            WsMsg::Ping(p) => ws.send(WsMsg::Pong(p)).await?,
            _ => {}
        }
    }
    Ok(())
}

fn handle_snapshot_update(
    buf: Bytes,
    asks: &mut BookSide,
    bids: &mut RevSide,
    last_ver: &mut u64,
) -> Result<()> {
    use mexc_pb::push_data_v3_api_wrapper::Body;

    let wrapper = PushDataV3ApiWrapper::decode(buf)?;
    let Some(body) = wrapper.body else { return Ok(()); };
    let Body::PublicLimitDepths(delta) = body else { return Ok(()); };

    let ver: u64 = delta.version.parse()?;
    if ver <= *last_ver {
        return Ok(());
    }
    *last_ver = ver;

    // Seiten ersetzen
    asks.clear();
    for item in delta.asks {
        let p = OrderedFloat(item.price.parse::<f64>()?);
        let q: f64 = item.quantity.parse()?;
        if q != 0.0 {
            asks.insert(p, q);
        }
    }
    bids.clear();
    for item in delta.bids {
        let p = Reverse(OrderedFloat(item.price.parse::<f64>()?));
        let q: f64 = item.quantity.parse()?;
        if q != 0.0 {
            bids.insert(p, q);
        }
    }

    if let (Some((ask, _)), Some((bid, _))) = (asks.first_key_value(), bids.first_key_value()) {
        if bid.0.0 <= ask.0 {
            let spread = ask.0 - bid.0.0;
            println!(
                "bid {:>12.2} | ask {:>12.2} | spread {:.5} %",
                bid.0.0,
                ask.0,
                spread / ask.0 * 100.0
            );
        } else {
            eprintln!("crossed book (ignoriere tick)");
        }
    }
    Ok(())
}
