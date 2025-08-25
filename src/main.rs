// main.rs
use anyhow::Result;
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use ordered_float::OrderedFloat;
use prost::Message;
use std::{cmp::Reverse, collections::BTreeMap};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMsg};

type BookSide = BTreeMap<OrderedFloat<f64>, f64>;
type RevSide = BTreeMap<Reverse<OrderedFloat<f64>>, f64>;

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
    let snap: Snapshot = reqwest::get(format!(
        "https://api.mexc.com/api/v3/depth?symbol={symbol}&limit=1000"
    ))
    .await?
    .json()
    .await?;

    let mut asks: BookSide = snap
        .asks
        .into_iter()
        .map(|[p, q]| (OrderedFloat(p.parse().unwrap()), q.parse().unwrap()))
        .collect();

    let mut bids: RevSide = snap
        .bids
        .into_iter()
        .map(|[p, q]| (Reverse(OrderedFloat(p.parse().unwrap())), q.parse().unwrap()))
        .collect();

    let mut last_ver = snap.last_update_id;
    println!("snapshot version {last_ver}");

    let (mut ws, _) = connect_async("wss://wbs-api.mexc.com/ws").await?;
    let chan = format!("spot@public.limit.depth.v3.api.pb@{symbol}@20");
    ws.send(WsMsg::Text(
        serde_json::json!({ "method": "SUBSCRIPTION", "params": [chan] }).to_string(),
    ))
    .await?;

    while let Some(msg) = ws.next().await {
        match msg? {
            WsMsg::Binary(buf) => {
                if let Err(e) = handle_delta(buf.into(), &mut asks, &mut bids, &mut last_ver) {
                    eprintln!("decode error: {e}");
                }
            }
            WsMsg::Ping(p) => ws.send(WsMsg::Pong(p)).await?,
            _ => {}
        }
    }
    Ok(())
}

fn handle_delta(
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

    for item in delta.asks {
        let p = OrderedFloat(item.price.parse()?);
        let q: f64 = item.quantity.parse()?;
        if q == 0.0 {
            asks.remove(&p);
        } else {
            asks.insert(p, q);
        }
    }
    for item in delta.bids {
        let p = Reverse(OrderedFloat(item.price.parse()?));
        let q: f64 = item.quantity.parse()?;
        if q == 0.0 {
            bids.remove(&p);
        } else {
            bids.insert(p, q);
        }
    }

    if let (Some((ask, _)), Some((bid, _))) = (asks.first_key_value(), bids.first_key_value()) {
        let spread = ask.0 - bid.0.0;
        println!(
            "bid {:>12.2} | ask {:>12.2} | spread {:.5} %",
            bid.0.0,
            ask.0,
            spread / ask.0 * 100.0
        );
    }
    Ok(())
}
