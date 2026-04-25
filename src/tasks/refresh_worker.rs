//! Periodically refreshes one user_cache row at a time. Schedule density is
//! tuned to the configured Roblox API rate limit and the live linked-user count.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::Mutex;

use crate::services::sync::PlayerSyncEvent;
use crate::AppState;

const MIN_REFRESH_SECS: i64 = 1800;
const MAX_REFRESH_SECS: i64 = 86400;
const INTERVAL_CACHE_SECS: u64 = 300;
const INACTIVE_MULTIPLIER: i64 = 6;

struct CachedInterval {
    value: AtomicI64,
    max_req_per_hour: i64,
    last_computed: Mutex<Instant>,
}

impl CachedInterval {
    fn new(max_req_per_hour: i64) -> Self {
        Self {
            value: AtomicI64::new(MIN_REFRESH_SECS),
            max_req_per_hour,
            last_computed: Mutex::new(Instant::now() - std::time::Duration::from_secs(INTERVAL_CACHE_SECS + 1)),
        }
    }

    async fn get(&self, pool: &sqlx::PgPool) -> i64 {
        let mut last = self.last_computed.lock().await;
        if last.elapsed() >= std::time::Duration::from_secs(INTERVAL_CACHE_SECS) {
            let player_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM linked_accounts")
                .fetch_one(pool)
                .await
                .unwrap_or(0);
            let interval = if player_count == 0 {
                MIN_REFRESH_SECS
            } else {
                ((player_count * 3600) / self.max_req_per_hour)
                    .clamp(MIN_REFRESH_SECS, MAX_REFRESH_SECS)
            };
            self.value.store(interval, Ordering::Relaxed);
            *last = Instant::now();
        }
        self.value.load(Ordering::Relaxed)
    }
}

pub async fn run(state: Arc<AppState>) {
    // Convert per-minute Roblox limit into per-hour budget for scheduling math
    let max_req_hour = state.config.roblox_api_rate_limit as i64 * 60;
    tracing::info!(max_req_hour, "Refresh worker started");
    let cached = CachedInterval::new(max_req_hour);

    loop {
        let next = sqlx::query_as::<_, (String, String, bool)>(
            "SELECT uc.roblox_user_id, la.discord_id, \
             EXISTS(SELECT 1 FROM role_assignments ra WHERE ra.discord_id = la.discord_id) as is_active \
             FROM user_cache uc \
             JOIN linked_accounts la ON la.roblox_user_id = uc.roblox_user_id \
             WHERE uc.next_fetch_at <= now() \
             ORDER BY is_active DESC, uc.fetch_failures ASC, uc.next_fetch_at ASC \
             LIMIT 1",
        )
        .fetch_optional(&state.pool)
        .await;

        let (roblox_user_id, discord_id, is_active) = match next {
            Ok(Some(row)) => row,
            Ok(None) => {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                continue;
            }
            Err(e) => {
                tracing::error!("Refresh worker DB error: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
        };

        match refresh_user(&state, &roblox_user_id).await {
            Ok(()) => {
                let base = cached.get(&state.pool).await;
                let interval = base * if is_active { 1 } else { INACTIVE_MULTIPLIER };
                let next_fetch = chrono::Utc::now() + chrono::Duration::seconds(interval);
                if let Err(e) = sqlx::query(
                    "UPDATE user_cache SET next_fetch_at = $1, fetch_failures = 0 WHERE roblox_user_id = $2",
                )
                .bind(next_fetch)
                .bind(&roblox_user_id)
                .execute(&state.pool)
                .await
                {
                    tracing::error!(roblox_user_id, "Failed to update next_fetch_at: {e}");
                }
                let _ = state
                    .player_sync_tx
                    .try_send(PlayerSyncEvent::PlayerUpdated { discord_id });
                tracing::debug!(roblox_user_id, interval, is_active, "Roblox data refreshed");
            }
            Err(e) => {
                if let Err(db_err) = sqlx::query(
                    "UPDATE user_cache SET fetch_failures = fetch_failures + 1, \
                     next_fetch_at = now() + LEAST(INTERVAL '60 seconds' * POWER(2, fetch_failures), INTERVAL '1 hour') \
                     WHERE roblox_user_id = $1",
                )
                .bind(&roblox_user_id)
                .execute(&state.pool)
                .await
                {
                    tracing::error!(roblox_user_id, "Failed to update failure count: {db_err}");
                }
                tracing::warn!(roblox_user_id, "Roblox refresh failed: {e}");
            }
        }
    }
}

async fn refresh_user(state: &AppState, roblox_user_id: &str) -> Result<(), crate::error::AppError> {
    let api = &state.roblox_client;

    // Profile + counts + groups + badges + gamepasses (best effort — non-fatal failures
    // surface as zeros so we don't lose role grants on transient hiccups).
    let profile = api.get_user_profile(roblox_user_id).await?;
    let (friends, followers, following) = api.get_friend_counts(roblox_user_id).await.unwrap_or((0, 0, 0));
    let groups = api.get_groups(roblox_user_id).await.unwrap_or_default();
    let (badges, badges_total) = api.get_badges(roblox_user_id).await.unwrap_or_default();
    let gamepasses = api.get_gamepasses(roblox_user_id).await.unwrap_or_default();

    let groups_json = serde_json::to_value(&groups).unwrap_or_default();
    let badges_json = serde_json::Value::Array(
        badges.into_iter().map(|i| serde_json::Value::String(i.to_string())).collect(),
    );
    let gamepasses_json = serde_json::Value::Array(
        gamepasses.into_iter().map(|i| serde_json::Value::String(i.to_string())).collect(),
    );
    let profile_json = serde_json::to_value(&profile).unwrap_or_default();

    let account_created = profile
        .created
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));

    let has_verified_badge = profile.has_verified_badge.unwrap_or(false);

    sqlx::query(
        "UPDATE user_cache SET \
         username = $1, display_name = $2, description = $3, \
         account_created = COALESCE($4, account_created), \
         has_verified_badge = $5, friends_count = $6, followers_count = $7, following_count = $8, \
         badges_count = $9, groups = $10, badges = $11, gamepasses = $12, profile_data = $13, \
         fetched_at = now() \
         WHERE roblox_user_id = $14",
    )
    .bind(profile.name.as_deref())
    .bind(profile.display_name.as_deref())
    .bind(profile.description.as_deref())
    .bind(account_created)
    .bind(has_verified_badge)
    .bind(friends as i32)
    .bind(followers as i32)
    .bind(following as i32)
    .bind(badges_total as i32)
    .bind(&groups_json)
    .bind(&badges_json)
    .bind(&gamepasses_json)
    .bind(&profile_json)
    .bind(roblox_user_id)
    .execute(&state.pool)
    .await?;

    Ok(())
}
