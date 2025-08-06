mod ws;

#[tokio::main]
async fn main() {
    ws::connect_ws().await;
}
