use anyhow::{anyhow, Result};
use bytes::Bytes;
use ordered_float::OrderedFloat;
use prost::Message;
use serde::Deserialize;
use std::{cmp::Reverse, collections::BTreeMap};

pub mod mexc_pb {
    include!(concat!(env!("OUT_DIR"), "/mexc.pb.rs"));
}
use mexc_pb::PushDataV3ApiWrapper;

pub type BookSide = BTreeMap<OrderedFloat<f64>, f64>;
pub type RevSide = BTreeMap<Reverse<OrderedFloat<f64>>, f64>;

#[derive(Deserialize)]
pub struct Snapshot {
    #[serde(rename = "lastUpdateId")]
    pub last_update_id: u64,
    pub bids: Vec<[String; 2]>,
    pub asks: Vec<[String; 2]>,
}

pub async fn reload_snapshot(symbol: &str, asks: &mut BookSide, bids: &mut RevSide) -> Result<u64> {
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

pub fn handle_diff_update(
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
    let to_v: u64 = delta.to_version.parse()?;

    if to_v < *snap_ver { return Ok(()); }

    if last_to_ver.is_none() {
        if from_v > *snap_ver + 1 { return Err(anyhow!("gap after snapshot")); }
    } else if let Some(prev) = *last_to_ver {
        if from_v != prev + 1 { return Err(anyhow!("sequence gap")); }
    }

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

    Ok(())
}

//