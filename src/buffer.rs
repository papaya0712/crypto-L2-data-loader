// buffer.rs 
use tokio::sync::mpsc;
use serde::{Serialize, Deserialize};
use std::fs::OpenOptions;
use std::io::Write;
use tokio::time::{interval, Duration};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MarketEvent {
    OrderBookSnapshot {
        symbol: String,
        bids: Vec<(f64, f64)>,
        asks: Vec<(f64, f64)>,
        seq: i64,
    },
    OrderBookDelta {
        symbol: String,
        bids: Vec<(f64, f64)>,
        asks: Vec<(f64, f64)>,
        seq: i64,
    },
    Trade {
        symbol: String,
        price: f64,
        size: f64,
        side: String, // "Buy"/"Sell"
        ts: i64,
    },
    Funding {
        symbol: String,
        rate: f64,
        ts: i64,
    },
}

pub type EventSender = mpsc::Sender<MarketEvent>;
pub type EventReceiver = mpsc::Receiver<MarketEvent>;

pub fn create_buffer(capacity: usize) -> (EventSender, EventReceiver) {
    mpsc::channel(capacity)
}

pub async fn writer_task(mut rx: EventReceiver, file_prefix: &str) {
    let mut buffer = Vec::new();
    let mut ticker = interval(Duration::from_secs(30)); // alle 30s flush

    loop {
        tokio::select! {
            Some(event) = rx.recv() => {
                buffer.push(event);
            }
            _ = ticker.tick() => {
                if !buffer.is_empty() {
                    // Eine Datei pro Tag
                    let fname = format!("logs/{}_{}.jsonl",
                        file_prefix,
                        chrono::Utc::now().format("%Y%m%d")
                    );
                    let mut file = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(fname)
                        .unwrap();

                    for ev in buffer.drain(..) {
                        let line = serde_json::to_string(&ev).unwrap();
                        writeln!(file, "{}", line).unwrap();
                    }
                }
            }
        }
    }
}

pub fn replay_from_file(path: &str) -> Vec<MarketEvent> {
    use std::io::{BufRead, BufReader};
    use std::fs::File;

    let file = File::open(path).unwrap();
    let reader = BufReader::new(file);
    reader.lines()
        .filter_map(|line| {
            line.ok().and_then(|l| serde_json::from_str::<MarketEvent>(&l).ok())
        })
        .collect()
}