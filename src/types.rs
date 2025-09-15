//types.rs
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthSnapshot {
    pub symbol: String,
    pub ts_recv_ms: i64,
    pub last_update_id: u64,
    pub bids: Vec<[f64;2]>,
    pub asks: Vec<[f64;2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthDelta {
    pub symbol: String,
    pub ts_recv_ms: i64,
    pub from_version: u64,
    pub to_version: u64,
    pub bids: Vec<[f64;2]>,
    pub asks: Vec<[f64;2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeEvent {
    pub symbol: String,
    pub ts_recv_ms: i64,
    pub id: Option<u64>,
    pub price: f64,
    pub qty: f64,
    pub side: Option<String>,
    pub ts_exch_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClockSkewSample {
    pub ts_local_ms: i64,
    pub server_time_ms: i64,
    pub offset_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetrySample {
    pub ts_ms: i64,
    pub kind: &'static str,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub count: u64,
}
