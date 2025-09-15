// main.rs
use anyhow::{anyhow, Result};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use ordered_float::OrderedFloat;
use prost::Message;
use reqwest;
use serde::Deserialize;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashSet, VecDeque},
    hash::{Hash, Hasher},
    time::{Duration, Instant},
};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMsg};
use std::sync::Arc;

type BookSide = BTreeMap<OrderedFloat<f64>, f64>;
type RevSide = BTreeMap<Reverse<OrderedFloat<f64>>, f64>;

pub mod mexc_pb { include!(concat!(env!("OUT_DIR"), "/mexc.pb.rs")); }
use mexc_pb::PushDataV3ApiWrapper;

mod types;
mod telemetry;
mod store;
use telemetry::Telemetry;
use types::{DepthSnapshot, DepthDelta, TradeEvent, ClockSkewSample};
use store::DataStore;

#[derive(Deserialize)]
struct Snapshot {
    #[serde(rename = "lastUpdateId")]
    last_update_id: u64,
    bids: Vec<[String; 2]>,
    asks: Vec<[String; 2]>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let symbol = std::env::args().nth(1).unwrap_or_else(|| "BTCUSDT".to_string());
    let outdir = std::env::args().nth(2).unwrap_or_else(|| "data".to_string());

    let store = DataStore::new(&outdir)?;
    let telem = Arc::new(Telemetry::new());

    let mut asks: BookSide = BTreeMap::new();
    let mut bids: RevSide = BTreeMap::new();

    // REST snapshot + persist (in eine Datei)
    let t0 = Instant::now();
    let mut snap_ver = reload_snapshot(&symbol, &mut asks, &mut bids).await?;
    let rtt_ms = t0.elapsed().as_millis() as u64;
    telem.record_rest_rtt_ms(rtt_ms).await;

    let ts_now = epoch_ms();
    store.append_event_json(
        &symbol,
        ts_now,
        "depth_snapshot",
        &DepthSnapshot {
            symbol: symbol.clone(),
            ts_recv_ms: ts_now,
            last_update_id: snap_ver,
            bids: bids.iter().take(50).map(|(k,q)| [ (k.0).0, *q ]).collect(),
            asks: asks.iter().take(50).map(|(k,q)| [ k.0, *q ]).collect(),
        },
    )?;

    println!("REST snapshot loaded (lastUpdateId={})", snap_ver);
    println!("levels: asks={} bids={}", asks.len(), bids.len());

    // Telemetry: clock skew sampler
    let telem_clone = telem.clone();
    tokio::spawn(clock_skew_task(telem_clone));

    // Trades collector (optional)
    let store_tr = DataStore::new(&outdir)?;
    let symbol_tr = symbol.clone();
    tokio::spawn(async move {
        if let Err(e) = trades_poller_rest(symbol_tr, store_tr).await {
            eprintln!("trades poller stopped: {e}");
        }
    });

    // Depth WS consumer
    depth_ws_loop(symbol, asks, bids, snap_ver, &store, telem).await?;
    Ok(())
}

async fn clock_skew_task(_telem: Arc<Telemetry>) {
    loop {
        match reqwest::get("https://api.mexc.com/api/v3/time").await {
            Ok(resp) => {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    let ts_local = epoch_ms();
                    if let Some(server) = json.get("serverTime").and_then(|v| v.as_i64()) {
                        let _sample = ClockSkewSample {
                            ts_local_ms: ts_local,
                            server_time_ms: server,
                            offset_ms: server - ts_local,
                        };
                        // optional: persist as event if gewünscht
                    }
                }
            }
            Err(e) => eprintln!("clock skew req error: {e}"),
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

async fn depth_ws_loop(
    symbol: String,
    mut asks: BookSide,
    mut bids: RevSide,
    mut snap_ver: u64,
    store: &DataStore,
    telem: Arc<Telemetry>,
) -> Result<()> {
    let url = "wss://wbs-api.mexc.com/ws";
    let (mut ws, _) = connect_async(url).await?;
    let chan = format!("spot@public.aggre.depth.v3.api.pb@10ms@{symbol}");
    ws.send(WsMsg::Text(
        serde_json::json!({
            "method": "SUBSCRIPTION",
            "params": [chan]
        }).to_string(),
    ))
    .await?;

    let mut ping_tick = tokio::time::interval(Duration::from_secs(30));
    let mut last_to_ver: Option<u64> = None;
    let mut last_ping_sent: Option<Instant> = None;

    loop {
        tokio::select! {
            _ = ping_tick.tick() => {
                last_ping_sent = Some(Instant::now());
                let _ = ws.send(WsMsg::Ping(Vec::new())).await;
            }
            msg = ws.next() => {
                match msg {
                    Some(Ok(WsMsg::Binary(buf))) => {
                        let recv_ts = epoch_ms();

                        // Rohpayload in EINER Datei ablegen
                        let _ = store.append_event_raw_b64(&symbol, recv_ts, "depth_pb_raw", &buf);

                        match handle_diff_update(buf.into(), &mut asks, &mut bids, &mut snap_ver, &mut last_to_ver) {
                            Ok(()) => {
                                // optional: strukturierte Top-50 als Event (ebenfalls in derselben Datei)
                                let _ = store.append_event_json(&symbol, recv_ts, "depth_delta", &DepthDelta{
                                    symbol: symbol.clone(),
                                    ts_recv_ms: recv_ts,
                                    from_version: last_to_ver.unwrap_or(snap_ver),
                                    to_version: snap_ver,
                                    bids: bids.iter().take(50).map(|(k,q)| [ (k.0).0, *q ]).collect(),
                                    asks: asks.iter().take(50).map(|(k,q)| [ k.0, *q ]).collect(),
                                });
                            }
                            Err(e) => {
                                {
                                    let mut gaps = telem.gap_counter.lock().await;
                                    *gaps += 1;
                                }
                                eprintln!("delta error ({e}), trying resync");
                                if let Ok(new_ver) = reload_snapshot(&symbol, &mut asks, &mut bids).await {
                                    snap_ver = new_ver;
                                    last_to_ver = None;
                                    {
                                        let mut r = telem.resync_counter.lock().await;
                                        *r += 1;
                                    }
                                    eprintln!("resynced via REST (lastUpdateId={})", snap_ver);
                                    // optional: snapshot als Event persistieren
                                    let ts_now = epoch_ms();
                                    let _ = store.append_event_json(
                                        &symbol, ts_now, "depth_snapshot",
                                        &DepthSnapshot {
                                            symbol: symbol.clone(),
                                            ts_recv_ms: ts_now,
                                            last_update_id: snap_ver,
                                            bids: bids.iter().take(50).map(|(k,q)| [ (k.0).0, *q ]).collect(),
                                            asks: asks.iter().take(50).map(|(k,q)| [ k.0, *q ]).collect(),
                                        }
                                    );
                                }
                            }
                        }
                    }
                    Some(Ok(WsMsg::Pong(_))) => {
                        if let Some(t0) = last_ping_sent.take() {
                            let rtt = t0.elapsed().as_millis() as u64;
                            telem.record_ws_rtt_ms(rtt).await;
                        }
                    }
                    Some(Ok(WsMsg::Ping(p))) => {
                        let _ = ws.send(WsMsg::Pong(p)).await;
                    }
                    Some(Ok(WsMsg::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        eprintln!("ws error: {e}");
                        break;
                    }
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
    .json()
    .await?;

    asks.clear();
    bids.clear();
    for [p, q] in snap.asks.iter() {
        asks.insert(OrderedFloat(p.parse::<f64>()?), q.parse()?);
    }
    for [p, q] in snap.bids.iter() {
        bids.insert(Reverse(OrderedFloat(p.parse::<f64>()?)), q.parse()?);
    }
    Ok(snap.last_update_id)
}

// Sequenzlogik mit sanftem Fast-Forward
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
    let to_v: u64 = delta.to_version.parse()?;

    if to_v <= *snap_ver {
        return Ok(());
    }

    let needed = *snap_ver + 1;
    if !(from_v <= needed && needed <= to_v) {
        if from_v > needed {
            let gap = from_v.saturating_sub(needed);
            if last_to_ver.is_none() || gap <= 1000 {
                *snap_ver = from_v - 1;
            } else {
                return Err(anyhow!("sequence gap: need {}, got {}..{}", needed, from_v, to_v));
            }
        } else {
            return Ok(());
        }
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

    if let (Some((ask, _)), Some((bid, _))) = (asks.first_key_value(), bids.first_key_value()) {
        if bid.0 .0 <= ask.0 {
            let spread = ask.0 - bid.0 .0;
            println!(
                "bid {:>12.6} | ask {:>12.6} | spread {:>8.5} %",
                bid.0 .0, ask.0, spread / ask.0 * 100.0
            );
        } else {
            eprintln!("crossed book (tick verworfen)");
        }
    }
    Ok(())
}

// ===== Trades via public /trades; robust bei null-IDs mit Dedup =====
async fn trades_poller_rest(symbol: String, store: DataStore) -> Result<()> {
    #[derive(Deserialize)]
    struct RespTrade {
        id: Option<u64>,
        price: String,
        qty: String,
        #[serde(default)]
        quoteQty: Option<String>,
        time: Option<i64>,
        #[serde(default)]
        isBuyerMaker: Option<bool>,
        #[serde(default)]
        isBestMatch: Option<bool>,
        #[serde(default)]
        tradeType: Option<String>,
    }

    struct Dedup {
        set: HashSet<u64>,
        q: VecDeque<u64>,
        cap: usize,
    }
    impl Dedup {
        fn new(cap: usize) -> Self { Self { set: HashSet::new(), q: VecDeque::new(), cap } }
        fn insert(&mut self, k: u64) -> bool {
            if self.set.insert(k) {
                self.q.push_back(k);
                if self.q.len() > self.cap {
                    if let Some(old) = self.q.pop_front() { self.set.remove(&old); }
                }
                true
            } else { false }
        }
    }
    fn key_hash(t: &RespTrade) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        let mut h = DefaultHasher::new();
        let ts = t.time.unwrap_or(0);
        ts.hash(&mut h);
        let p = t.price.parse::<f64>().unwrap_or(0.0).to_bits();
        let q = t.qty.parse::<f64>().unwrap_or(0.0).to_bits();
        p.hash(&mut h); q.hash(&mut h);
        let side = t.isBuyerMaker.unwrap_or(false);
        side.hash(&mut h);
        h.finish()
    }

    let client = reqwest::Client::new();
    let mut last_ts: i64 = 0;
    let mut dedup = Dedup::new(10_000);

    loop {
        let url = format!("https://api.mexc.com/api/v3/trades?symbol={}&limit=1000", symbol);
        let mut req = client.get(&url);
        if let Ok(k) = std::env::var("MEXC_API_KEY") { req = req.header("X-MEXC-APIKEY", k); }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => { eprintln!("trades http err: {e}"); tokio::time::sleep(Duration::from_millis(500)).await; continue; }
        };
        let status = resp.status();
        let body = match resp.text().await {
            Ok(b) => b,
            Err(e) => { eprintln!("trades body err: {e}"); tokio::time::sleep(Duration::from_millis(400)).await; continue; }
        };
        if !status.is_success() {
            eprintln!("trades http {} body: {}", status, &body.chars().take(200).collect::<String>());
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        }

        let mut v: Vec<RespTrade> = match serde_json::from_str(&body) {
            Ok(x) => x,
            Err(e) => { eprintln!("trades json err: {e} body: {}", &body.chars().take(200).collect::<String>()); tokio::time::sleep(Duration::from_millis(400)).await; continue; }
        };

        v.sort_by(|a, b| {
            let ta = a.time.unwrap_or(0);
            let tb = b.time.unwrap_or(0);
            ta.cmp(&tb).then_with(|| a.id.cmp(&b.id))
        });

        for t in v {
            let tts = t.time.unwrap_or(0);
            if tts < last_ts { continue; }
            let k = key_hash(&t);
            if !dedup.insert(k) { continue; }
            if tts > last_ts { last_ts = tts; }

            let ts_recv = epoch_ms();
            let evt = TradeEvent{
                symbol: symbol.clone(),
                ts_recv_ms: ts_recv,
                id: t.id,
                price: t.price.parse().unwrap_or(0.0),
                qty: t.qty.parse().unwrap_or(0.0),
                side: t.isBuyerMaker.map(|b| if b {"SELL".into()} else {"BUY".into()}),
                ts_exch_ms: t.time,
            };
            let _ = store.append_event_json(&symbol, ts_recv, "trade", &evt);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[inline]
fn epoch_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now();
    let ms = now.duration_since(UNIX_EPOCH).unwrap();
    (ms.as_secs() as i64) * 1000 + (ms.subsec_millis() as i64)
}
