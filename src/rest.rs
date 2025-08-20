// rest.rs
use reqwest::Client;
use serde::Deserialize;
use anyhow::Result;

#[derive(Clone)]
pub struct RestClient {
    client: Client,
    base_url: String,
}

impl RestClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.to_string(),
        }
    }

    pub async fn instruments_info(&self, symbol: &str) -> Result<serde_json::Value> {
        let url = format!(
            "{}/v5/market/instruments-info?category=linear&symbol={}",
            self.base_url, symbol
        );
        let resp = self.client.get(&url).send().await?;
        let json: serde_json::Value = resp.json().await?;
        Ok(json)
    }

    pub async fn funding_rate(&self, symbol: &str) -> Result<FundingRateEntry> {
        let url = format!(
            "{}/v5/market/funding/history?category=linear&symbol={}&limit=1",
            self.base_url, symbol
        );
        let resp = self.client.get(&url).send().await?;
        let parsed: FundingRateResponse = resp.json().await?;
        if parsed.retCode != 0 {
            anyhow::bail!("API error: {}", parsed.retMsg);
        }
        let entry = parsed
            .result
            .list
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No funding rate returned"))?;
        Ok(entry)
    }
}

#[derive(Debug, Deserialize)]
pub struct FundingRateResponse {
    pub retCode: i32,
    pub retMsg: String,
    pub result: FundingRateResult,
}

#[derive(Debug, Deserialize)]
pub struct FundingRateResult {
    pub category: String,
    pub list: Vec<FundingRateEntry>,
}

#[derive(Debug, Deserialize)]
pub struct FundingRateEntry {
    pub symbol: String,
    pub fundingRate: String,
    pub fundingRateTimestamp: String,
}
