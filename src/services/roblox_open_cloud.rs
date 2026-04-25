//! Roblox Open Cloud DataStore + universe lookup. Auth via `x-api-key` header
//! using a per-universe API key issued at create.roblox.com → Credentials.

use std::num::NonZeroU32;
use std::sync::Arc;

use governor::{Quota, RateLimiter};
use serde::Deserialize;

use crate::error::AppError;

type GovernorLimiter = RateLimiter<
    governor::state::NotKeyed,
    governor::state::InMemoryState,
    governor::clock::DefaultClock,
>;

#[derive(Clone)]
pub struct OpenCloudClient {
    http: reqwest::Client,
    rate_limiter: Arc<GovernorLimiter>,
}

#[derive(Debug, Deserialize)]
pub struct UniverseInfo {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(rename = "displayName", default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
}

impl OpenCloudClient {
    pub fn new(rate_per_minute: u32) -> Self {
        let per_minute = NonZeroU32::new(rate_per_minute.max(1)).unwrap();
        let quota = Quota::per_minute(per_minute);
        let rate_limiter = Arc::new(RateLimiter::direct(quota));
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("Failed to build HTTP client");
        Self { http, rate_limiter }
    }

    async fn wait(&self) {
        self.rate_limiter.until_ready().await;
    }

    /// Verify API key ownership of a universe. Returns universe metadata if the key works.
    pub async fn get_universe(
        &self,
        universe_id: &str,
        api_key: &str,
    ) -> Result<UniverseInfo, AppError> {
        self.wait().await;
        let url = format!("https://apis.roblox.com/cloud/v2/universes/{universe_id}");
        let resp = self
            .http
            .get(&url)
            .header("x-api-key", api_key)
            .send()
            .await
            .map_err(|e| AppError::OpenCloud(format!("get_universe request: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::OpenCloud(format!(
                "get_universe returned {status}: {body}"
            )));
        }
        resp.json::<UniverseInfo>()
            .await
            .map_err(|e| AppError::OpenCloud(format!("get_universe parse: {e}")))
    }

    /// Read a single Standard DataStore entry. Returns the raw JSON value, or None if 404.
    /// Uses Open Cloud DataStore v2:
    ///   GET /cloud/v2/universes/{u}/data-stores/{ds}/scopes/global/entries/{entryId}
    pub async fn read_datastore_entry(
        &self,
        universe_id: &str,
        datastore_name: &str,
        entry_id: &str,
        api_key: &str,
    ) -> Result<Option<serde_json::Value>, AppError> {
        self.wait().await;
        let url = format!(
            "https://apis.roblox.com/cloud/v2/universes/{universe_id}/data-stores/{}/scopes/global/entries/{}",
            urlencoding::encode(datastore_name),
            urlencoding::encode(entry_id),
        );
        let resp = self
            .http
            .get(&url)
            .header("x-api-key", api_key)
            .send()
            .await
            .map_err(|e| AppError::OpenCloud(format!("datastore read request: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::OpenCloud(format!(
                "datastore read returned {status}: {body}"
            )));
        }

        // The v2 entry response wraps the value in {"value": ...}. We return the inner value.
        let env: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::OpenCloud(format!("datastore parse: {e}")))?;
        let value = env
            .get("value")
            .cloned()
            .or_else(|| Some(env.clone()));
        Ok(value)
    }
}

/// Apply a `stat_field_map` (admin-defined `{"path.in.json": "ourField"}`) to a
/// raw DataStore JSON value. Returns the mapped flat object.
pub fn apply_field_map(
    raw: &serde_json::Value,
    field_map: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    for (path, target) in field_map {
        let target_key = match target.as_str() {
            Some(s) => s,
            None => continue,
        };
        if let Some(v) = read_path(raw, path) {
            out.insert(target_key.to_string(), v.clone());
        }
    }
    out
}

fn read_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cur = value;
    for part in path.split('.') {
        if part.is_empty() {
            continue;
        }
        cur = match cur {
            serde_json::Value::Object(map) => map.get(part)?,
            serde_json::Value::Array(arr) => {
                let idx: usize = part.parse().ok()?;
                arr.get(idx)?
            }
            _ => return None,
        };
    }
    Some(cur)
}
