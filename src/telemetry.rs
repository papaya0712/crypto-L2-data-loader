//telemetry.rs
use hdrhistogram::Histogram;
use tokio::sync::Mutex;

pub struct Telemetry {
    ws_rtt: Mutex<Histogram<u64>>,
    rest_rtt: Mutex<Histogram<u64>>,
    pub gap_counter: Mutex<u64>,
    pub resync_counter: Mutex<u64>,
}

impl Telemetry {
    pub fn new() -> Self {
        Self {
            ws_rtt: Mutex::new(Histogram::new_with_max(10_000, 3).unwrap()),     // up to 10s in ms
            rest_rtt: Mutex::new(Histogram::new_with_max(60_000, 3).unwrap()),
            gap_counter: Mutex::new(0),
            resync_counter: Mutex::new(0),
        }
    }

    pub async fn record_ws_rtt_ms(&self, v_ms: u64) {
        let mut h = self.ws_rtt.lock().await;
        let _ = h.record(v_ms);
    }
    pub async fn record_rest_rtt_ms(&self, v_ms: u64) {
        let mut h = self.rest_rtt.lock().await;
        let _ = h.record(v_ms);
    }

    pub async fn snapshot(&self) -> ((f64,f64,f64,u64), (f64,f64,f64,u64), u64, u64) {
        let w = self.ws_rtt.lock().await;
        let r = self.rest_rtt.lock().await;
        let ws = (w.value_at_quantile(0.50) as f64,
                  w.value_at_quantile(0.95) as f64,
                  w.value_at_quantile(0.99) as f64,
                  w.len() as u64);
        let rr = (r.value_at_quantile(0.50) as f64,
                  r.value_at_quantile(0.95) as f64,
                  r.value_at_quantile(0.99) as f64,
                  r.len() as u64);
        let gaps = *self.gap_counter.lock().await;
        let resyncs = *self.resync_counter.lock().await;
        (ws, rr, gaps, resyncs)
    }
}
