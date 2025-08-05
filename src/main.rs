use tokio::time::{interval, Duration};
use chrono::Utc;

#[tokio::main]
async fn main() {
    let mut ticker = interval(Duration::from_secs(1));

    loop {
        ticker.tick().await;

        let now = Utc::now();
        println!("Tick at {}", now);
    }
}
