A Rust repository for automatic and efficient L2 crypto market data streaming, processing, and storage.

## 🧩 Description

This project provides a high-performance, asynchronous data pipeline for real-time **Level 2 (order book)** data from various cryptocurrency exchanges. It connects to WebSocket APIs, processes incoming data, and stores it for downstream analysis or archival.

## 🚀 Features

- ⚡ Efficient WebSocket connections using `tokio`
- 📥 Subscription to multiple L2 data feeds
- 🧠 Real-time parsing and processing of order book updates
- 💾 Optional persistent storage (e.g., local file, database) – *[placeholder]*
- 🔧 Exchange-agnostic design – easily extendable for multiple APIs
- 🧪 Unit-tested components for stability

## 📦 Planned (TODO)

- [ ] Add support for multiple exchanges (e.g., Binance, Kraken, Coinbase)
- [ ] Implement pluggable storage backends (SQLite, Parquet, etc.)
- [ ] Add data aggregation or analytics modules
- [ ] CLI interface for easy control & monitoring
- [ ] Docker support

## 🛠️ Installation

### Prerequisites

- [Rust (stable)](https://www.rust-lang.org/tools/install)
- Cargo (included with Rust)
- Optionally: `sqlite3`, `docker`, etc. depending on storage backend

### Clone and build

```bash
git clone https://github.com/papaya0712/rust_l2_fetcher.git
cd rust_l2_fetcher
cargo build --release