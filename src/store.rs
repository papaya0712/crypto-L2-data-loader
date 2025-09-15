// store.rs
use anyhow::Result;
use serde::Serialize;
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use base64::Engine as _;

pub struct DataStore {
    base: PathBuf,
}

impl DataStore {
    pub fn new<P: AsRef<Path>>(base: P) -> Result<Self> {
        let base = base.as_ref().to_path_buf();
        create_dir_all(&base)?;
        Ok(Self { base })
    }

    fn part_dir(&self, symbol: &str, ts_ms: i64) -> PathBuf {
        let t = OffsetDateTime::from_unix_timestamp_nanos((ts_ms as i128) * 1_000_000)
            .unwrap_or_else(|_| OffsetDateTime::UNIX_EPOCH);
        let date = t.date();
        let hour = t.hour();
        self.base
            .join(format!("symbol={}", symbol))
            .join(format!("date={:04}-{:02}-{:02}", date.year(), u8::from(date.month()), date.day()))
            .join(format!("hour={:02}", hour))
    }

    fn events_path(&self, symbol: &str, ts_ms: i64) -> PathBuf {
        self.part_dir(symbol, ts_ms).join("events.ndjson.zst")
    }

    fn open_zstd<P: AsRef<Path>>(p: P) -> Result<zstd::stream::write::Encoder<'static, std::fs::File>> {
        let path = p.as_ref();
        if let Some(dir) = path.parent() {
            create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let enc = zstd::stream::write::Encoder::new(file, 3)?; // level 3
        Ok(enc)
    }

    pub fn append_event_json<T: Serialize>(&self, symbol: &str, ts_ms: i64, kind: &str, payload: &T) -> Result<()> {
        let p = self.events_path(symbol, ts_ms);
        let mut enc = Self::open_zstd(p)?;
        let line = serde_json::json!({
            "ts_ms": ts_ms,
            "symbol": symbol,
            "kind": kind,
            "payload": payload
        })
        .to_string();
        enc.write_all(line.as_bytes())?;
        enc.write_all(b"\n")?;
        enc.finish()?;
        Ok(())
    }

    pub fn append_event_raw_b64(&self, symbol: &str, ts_ms: i64, kind: &str, raw: &[u8]) -> Result<()> {
        let p = self.events_path(symbol, ts_ms);
        let mut enc = Self::open_zstd(p)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(raw);
        let line = serde_json::json!({
            "ts_ms": ts_ms,
            "symbol": symbol,
            "kind": kind,
            "payload_b64": b64
        })
        .to_string();
        enc.write_all(line.as_bytes())?;
        enc.write_all(b"\n")?;
        enc.finish()?;
        Ok(())
    }
}