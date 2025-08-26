// main.rs
use anyhow::Result;
use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMsg};

mod orderbook;
use orderbook::{reload_snapshot, handle_diff_update, BookSide, RevSide};

#[tokio::main]
async fn main() -> Result<()> {
    let symbol = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "BTCUSDT".to_string());

    let mut asks: BookSide = BookSide::new();
    let mut bids: RevSide = RevSide::new();

    let mut snap_ver = reload_snapshot(&symbol, &mut asks, &mut bids).await?;
    println!("REST snapshot loaded (lastUpdateId={})", snap_ver);
    println!("levels: asks={} bids={}", asks.len(), bids.len());

    for (i, (k, q)) in asks.iter().take(5).enumerate() {
        println!("SNAP ASK[{}] price={:.12} qty={:.8}", i, k.0, q);
    }
    for (i, (k, q)) in bids.iter().take(5).enumerate() {
        println!("SNAP BID[{}] price={:.12} qty={:.8}", i, (k.0).0, q);
    }

    let (mut ws, _) = connect_async("wss://wbs-api.mexc.com/ws").await?;
    let chan = format!("spot@public.aggre.depth.v3.api.pb@10ms@{symbol}");
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

//