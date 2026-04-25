//! Public Roblox API client. No auth required for any endpoint here — these are
//! the documented unauth-friendly endpoints used by every Roblox-stats integration.
//! See the plan file's §7 table for the canonical list of endpoints.

use std::num::NonZeroU32;
use std::sync::Arc;

use governor::{Quota, RateLimiter};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

type GovernorLimiter = RateLimiter<
    governor::state::NotKeyed,
    governor::state::InMemoryState,
    governor::clock::DefaultClock,
>;

#[derive(Clone)]
pub struct RobloxApiClient {
    http: reqwest::Client,
    rate_limiter: Arc<GovernorLimiter>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RobloxUserProfile {
    pub id: i64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default, rename = "hasVerifiedBadge")]
    pub has_verified_badge: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GroupRole {
    pub group_id: i64,
    pub role_rank: i32,
    pub role_name: String,
}

#[derive(Debug, Deserialize)]
struct GroupsRolesEnvelope {
    data: Vec<GroupsRolesItem>,
}

#[derive(Debug, Deserialize)]
struct GroupsRolesItem {
    group: GroupsRolesGroup,
    role: GroupsRolesRole,
}

#[derive(Debug, Deserialize)]
struct GroupsRolesGroup {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct GroupsRolesRole {
    name: String,
    rank: i32,
}

#[derive(Debug, Deserialize)]
struct CountEnvelope {
    count: i64,
}

#[derive(Debug, Deserialize)]
struct PaginatedEnvelope<T> {
    data: Vec<T>,
    #[serde(default)]
    next_page_cursor: Option<String>,
    #[serde(default, rename = "nextPageCursor")]
    _next_page_cursor_camel: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BadgeItem {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct GamePassesEnvelope {
    #[serde(rename = "GamePasses", alias = "gamePasses", default)]
    game_passes: Vec<GamePassItem>,
}

#[derive(Debug, Deserialize)]
struct GamePassItem {
    #[serde(rename = "PassID", alias = "id", alias = "gamePassId")]
    id: i64,
}

impl RobloxApiClient {
    pub fn new(rate_per_minute: u32) -> Self {
        let per_minute = NonZeroU32::new(rate_per_minute.max(1)).unwrap();
        let quota = Quota::per_minute(per_minute);
        let rate_limiter = Arc::new(RateLimiter::direct(quota));

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .pool_max_idle_per_host(8)
            .user_agent("RoleLogic-RobloxPlayerRole/0.1 (+https://rolelogic.faizo.net)")
            .build()
            .expect("Failed to build reqwest client");

        Self { http, rate_limiter }
    }

    async fn wait(&self) {
        self.rate_limiter.until_ready().await;
    }

    pub async fn get_user_profile(&self, roblox_user_id: &str) -> Result<RobloxUserProfile, AppError> {
        self.wait().await;
        let url = format!("https://users.roblox.com/v1/users/{roblox_user_id}");
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::RobloxApi(format!("get_user_profile request: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::RobloxApi(format!(
                "get_user_profile {} returned {}",
                roblox_user_id,
                resp.status()
            )));
        }
        resp.json::<RobloxUserProfile>()
            .await
            .map_err(|e| AppError::RobloxApi(format!("get_user_profile parse: {e}")))
    }

    /// Returns (friends, followers, following) in a single call site (3 sub-requests).
    pub async fn get_friend_counts(&self, roblox_user_id: &str) -> Result<(i64, i64, i64), AppError> {
        let f = self.fetch_count(roblox_user_id, "friends/count");
        let fo = self.fetch_count(roblox_user_id, "followers/count");
        let fi = self.fetch_count(roblox_user_id, "followings/count");
        let (a, b, c) = tokio::join!(f, fo, fi);
        Ok((a.unwrap_or(0), b.unwrap_or(0), c.unwrap_or(0)))
    }

    async fn fetch_count(&self, roblox_user_id: &str, suffix: &str) -> Result<i64, AppError> {
        self.wait().await;
        let url = format!("https://friends.roblox.com/v1/users/{roblox_user_id}/{suffix}");
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::RobloxApi(format!("count request: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::RobloxApi(format!(
                "count {suffix} returned {}",
                resp.status()
            )));
        }
        let env: CountEnvelope = resp
            .json()
            .await
            .map_err(|e| AppError::RobloxApi(format!("count parse: {e}")))?;
        Ok(env.count)
    }

    pub async fn get_groups(&self, roblox_user_id: &str) -> Result<Vec<GroupRole>, AppError> {
        self.wait().await;
        let url = format!(
            "https://groups.roblox.com/v2/users/{roblox_user_id}/groups/roles?includeLocked=false"
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::RobloxApi(format!("get_groups request: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::RobloxApi(format!(
                "get_groups returned {}",
                resp.status()
            )));
        }
        let env: GroupsRolesEnvelope = resp
            .json()
            .await
            .map_err(|e| AppError::RobloxApi(format!("get_groups parse: {e}")))?;
        Ok(env
            .data
            .into_iter()
            .map(|i| GroupRole {
                group_id: i.group.id,
                role_rank: i.role.rank,
                role_name: i.role.name,
            })
            .collect())
    }

    /// Paginates badges.roblox.com — returns (badge_ids, total_count).
    /// Caps at 1000 badges to bound work; high-badge users are rare.
    pub async fn get_badges(&self, roblox_user_id: &str) -> Result<(Vec<i64>, usize), AppError> {
        let mut ids: Vec<i64> = Vec::new();
        let mut cursor: Option<String> = None;
        let max_pages = 10;
        let mut pages = 0;

        loop {
            self.wait().await;
            let mut url = format!(
                "https://badges.roblox.com/v1/users/{roblox_user_id}/badges?limit=100&sortOrder=Desc"
            );
            if let Some(c) = &cursor {
                url.push_str(&format!("&cursor={}", urlencoding::encode(c)));
            }
            let resp = self
                .http
                .get(&url)
                .send()
                .await
                .map_err(|e| AppError::RobloxApi(format!("get_badges request: {e}")))?;
            if !resp.status().is_success() {
                return Err(AppError::RobloxApi(format!(
                    "get_badges returned {}",
                    resp.status()
                )));
            }
            let env: PaginatedEnvelope<BadgeItem> = resp
                .json()
                .await
                .map_err(|e| AppError::RobloxApi(format!("get_badges parse: {e}")))?;
            for b in env.data {
                ids.push(b.id);
            }
            pages += 1;
            cursor = env.next_page_cursor;
            if cursor.is_none() || pages >= max_pages {
                break;
            }
        }
        let total = ids.len();
        Ok((ids, total))
    }

    pub async fn get_gamepasses(&self, roblox_user_id: &str) -> Result<Vec<i64>, AppError> {
        self.wait().await;
        let url = format!(
            "https://www.roblox.com/users/inventory/list-json?assetTypeId=34&cursor=&itemsPerPage=100&pageNumber=1&userId={roblox_user_id}"
        );
        // Try the newer Open Cloud-style endpoint first, fall back to legacy
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::RobloxApi(format!("get_gamepasses request: {e}")))?;
        if !resp.status().is_success() {
            // Inventory may be private (403). Treat as empty rather than fail.
            if resp.status() == reqwest::StatusCode::FORBIDDEN {
                return Ok(vec![]);
            }
            return Err(AppError::RobloxApi(format!(
                "get_gamepasses returned {}",
                resp.status()
            )));
        }
        // Legacy endpoint returns { Data: { Items: [{ Item: { AssetId } }, ...] } }
        let raw: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::RobloxApi(format!("get_gamepasses parse: {e}")))?;

        // Try newer top-level shape first
        if let Ok(env) = serde_json::from_value::<GamePassesEnvelope>(raw.clone()) {
            return Ok(env.game_passes.into_iter().map(|g| g.id).collect());
        }
        // Fallback to legacy shape
        let mut out = Vec::new();
        if let Some(items) = raw["Data"]["Items"].as_array() {
            for it in items {
                if let Some(id) = it["Item"]["AssetId"].as_i64() {
                    out.push(id);
                }
            }
        }
        Ok(out)
    }

    /// Per-item ownership check via inventory.roblox.com. Returns true if owned.
    /// item_type: 1=Asset, 21=Badge, 34=GamePass.
    pub async fn owns_item(
        &self,
        roblox_user_id: &str,
        item_type: u32,
        item_id: &str,
    ) -> Result<bool, AppError> {
        self.wait().await;
        let url = format!(
            "https://inventory.roblox.com/v1/users/{roblox_user_id}/items/{item_type}/{item_id}"
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::RobloxApi(format!("owns_item request: {e}")))?;
        if resp.status() == reqwest::StatusCode::FORBIDDEN {
            // Inventory private — cannot determine, treat as not-owned.
            return Ok(false);
        }
        if !resp.status().is_success() {
            return Err(AppError::RobloxApi(format!(
                "owns_item returned {}",
                resp.status()
            )));
        }
        let env: PaginatedEnvelope<serde_json::Value> = resp
            .json()
            .await
            .map_err(|e| AppError::RobloxApi(format!("owns_item parse: {e}")))?;
        Ok(!env.data.is_empty())
    }
}
