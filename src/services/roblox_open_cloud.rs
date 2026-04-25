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

    /// Strict ownership probe.
    ///
    ///   `GET /universes/{u}/data-stores?maxPageSize=1` — needs
    ///   `universe-datastores.control:list` on the key, scoped to this exact
    ///   universe. Only HTTP 200 counts as success.
    ///
    /// 404 is rejected because Roblox returns 404 for both "universe doesn't
    /// exist" and "scope doesn't include this universe" — accepting it would
    /// let any valid key register any universe ID (squatting / corruption of
    /// another game's stats).
    ///
    /// We don't probe `/places` because Roblox no longer exposes
    /// `universe.place:read` as a checkable scope in the dashboard. We don't
    /// probe a single DataStore entry (`objects:read`) because that endpoint
    /// returns 404 for missing entries, indistinguishable from "wrong universe".
    pub async fn verify_universe_ownership(
        &self,
        universe_id: &str,
        api_key: &str,
    ) -> Result<(), AppError> {
        self.wait().await;
        let url = format!(
            "https://apis.roblox.com/cloud/v2/universes/{universe_id}/data-stores?maxPageSize=1"
        );
        let resp = self
            .http
            .get(&url)
            .header("x-api-key", api_key)
            .send()
            .await
            .map_err(|e| AppError::OpenCloud(format!("verify_universe request: {e}")))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        Err(AppError::OpenCloud(format!(
            "verify_universe failed: {status}: {body}"
        )))
    }

    /// List the DataStores in a universe. Used to populate the pull-mode
    /// configuration dropdown. Requires `universe-datastores.objects:list`
    /// scope on the API key.
    pub async fn list_datastores(
        &self,
        universe_id: &str,
        api_key: &str,
    ) -> Result<Vec<String>, AppError> {
        self.wait().await;
        let url = format!(
            "https://apis.roblox.com/cloud/v2/universes/{universe_id}/data-stores?maxPageSize=100"
        );
        let resp = self
            .http
            .get(&url)
            .header("x-api-key", api_key)
            .send()
            .await
            .map_err(|e| AppError::OpenCloud(format!("list_datastores request: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::OpenCloud(format!(
                "list_datastores returned {status}: {body}"
            )));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::OpenCloud(format!("list_datastores parse: {e}")))?;
        let names = body
            .get("dataStores")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|d| {
                        d.get("id")
                            .and_then(|v| v.as_str().map(String::from))
                            .or_else(|| {
                                // Fall back to parsing the "path" suffix
                                d.get("path").and_then(|v| v.as_str()).and_then(|p| {
                                    p.rsplit('/').next().map(String::from)
                                })
                            })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(names)
    }

    /// List entry IDs for a DataStore. Used to grab a sample entry for
    /// field-map preview. Requires `universe-datastores.objects:list` scope.
    pub async fn list_entry_ids(
        &self,
        universe_id: &str,
        datastore_name: &str,
        api_key: &str,
        max: u32,
    ) -> Result<Vec<String>, AppError> {
        self.wait().await;
        let url = format!(
            "https://apis.roblox.com/cloud/v2/universes/{universe_id}/data-stores/{}/scopes/global/entries?maxPageSize={max}",
            urlencoding::encode(datastore_name),
        );
        let resp = self
            .http
            .get(&url)
            .header("x-api-key", api_key)
            .send()
            .await
            .map_err(|e| AppError::OpenCloud(format!("list_entries request: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::OpenCloud(format!(
                "list_entries returned {status}: {body}"
            )));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::OpenCloud(format!("list_entries parse: {e}")))?;
        let ids = body
            .get("dataStoreEntries")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|d| {
                        d.get("id")
                            .and_then(|v| v.as_str().map(String::from))
                            .or_else(|| {
                                d.get("path").and_then(|v| v.as_str()).and_then(|p| {
                                    p.rsplit('/').next().map(String::from)
                                })
                            })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(ids)
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
