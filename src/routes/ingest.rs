//! POST /ingest/{universe_id}/stats — webhook accepting per-player stats from a
//! Roblox game's HttpService:PostAsync. Auth via `X-Ingest-Secret` header.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use governor::{Quota, RateLimiter};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::error::AppError;
use crate::models::game_stat::PlayerStats;
use crate::AppState;

/// Per-universe rate limiter keyed by `universe_id`. 60 ingest requests per
/// minute per universe is plenty for the documented 60s batch interval in the
/// shipped Studio plugin, and well below RoleLogic's overall ingress capacity.
type UniverseLimiter = RateLimiter<
    governor::state::NotKeyed,
    governor::state::InMemoryState,
    governor::clock::DefaultClock,
>;

#[derive(Default)]
pub struct IngestLimiterTable {
    inner: Mutex<HashMap<String, Arc<UniverseLimiter>>>,
}

impl IngestLimiterTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get_or_insert(&self, universe_id: &str) -> Arc<UniverseLimiter> {
        let mut guard = self.inner.lock().await;
        if let Some(l) = guard.get(universe_id) {
            return l.clone();
        }
        let quota = Quota::per_minute(NonZeroU32::new(60).unwrap());
        let lim = Arc::new(RateLimiter::direct(quota));
        guard.insert(universe_id.to_string(), lim.clone());
        lim
    }
}

#[derive(Deserialize)]
pub struct IngestPayload {
    pub players: Vec<IngestPlayer>,
}

#[derive(Deserialize)]
pub struct IngestPlayer {
    pub user_id: String,
    #[serde(default)]
    pub stats: PlayerStats,
}

const MAX_PLAYERS_PER_REQUEST: usize = 100;

pub async fn ingest_stats(
    State(state): State<Arc<AppState>>,
    Path(universe_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<IngestPayload>,
) -> Result<Json<Value>, AppError> {
    if body.players.is_empty() {
        return Err(AppError::BadRequest("players array is empty".into()));
    }
    if body.players.len() > MAX_PLAYERS_PER_REQUEST {
        return Err(AppError::BadRequest(format!(
            "too many players in one request (max {MAX_PLAYERS_PER_REQUEST})"
        )));
    }

    // Look up universe + verify secret (constant-time)
    let row = sqlx::query_as::<_, (String, bool)>(
        "SELECT ingest_secret, push_enabled FROM game_universes WHERE universe_id = $1",
    )
    .bind(&universe_id)
    .fetch_optional(&state.pool)
    .await?;

    let Some((expected_secret, push_enabled)) = row else {
        return Err(AppError::NotFound(format!("Universe {universe_id} not registered")));
    };
    if !push_enabled {
        return Err(AppError::Forbidden("Push ingestion disabled for this universe".into()));
    }

    let provided = headers
        .get("X-Ingest-Secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !constant_time_eq(provided.as_bytes(), expected_secret.as_bytes()) {
        return Err(AppError::Unauthorized);
    }

    // Per-universe rate limit
    let limiter = state.ingest_limiters.get_or_insert(&universe_id).await;
    if limiter.check().is_err() {
        return Err(AppError::Forbidden(
            "Ingest rate limit reached for this universe (60 req/min)".into(),
        ));
    }

    let mut accepted = 0usize;
    let mut roblox_ids_to_sync: Vec<String> = Vec::new();

    let mut tx = state.pool.begin().await?;
    for p in &body.players {
        if p.user_id.is_empty() || p.stats.is_empty() {
            continue;
        }

        // Build COALESCE-style upsert that only overwrites supplied fields, and
        // shallowly merges custom into the existing JSONB.
        sqlx::query(
            "INSERT INTO player_game_stats (roblox_user_id, universe_id, \
                playtime_minutes, level, wins, losses, currency, achievements, custom, fetched_at) \
             VALUES ($1, $2, \
                COALESCE($3::int, 0), COALESCE($4::int, 0), COALESCE($5::int, 0), \
                COALESCE($6::int, 0), COALESCE($7::bigint, 0), \
                COALESCE($8::jsonb, '[]'::jsonb), COALESCE($9::jsonb, '{}'::jsonb), now()) \
             ON CONFLICT (roblox_user_id, universe_id) DO UPDATE SET \
                playtime_minutes = COALESCE($3::int, player_game_stats.playtime_minutes), \
                level            = COALESCE($4::int, player_game_stats.level), \
                wins             = COALESCE($5::int, player_game_stats.wins), \
                losses           = COALESCE($6::int, player_game_stats.losses), \
                currency         = COALESCE($7::bigint, player_game_stats.currency), \
                achievements     = COALESCE($8::jsonb, player_game_stats.achievements), \
                custom           = player_game_stats.custom || COALESCE($9::jsonb, '{}'::jsonb), \
                fetched_at       = now()",
        )
        .bind(&p.user_id)
        .bind(&universe_id)
        .bind(p.stats.playtime_minutes.map(|v| v as i32))
        .bind(p.stats.level.map(|v| v as i32))
        .bind(p.stats.wins.map(|v| v as i32))
        .bind(p.stats.losses.map(|v| v as i32))
        .bind(p.stats.currency)
        .bind(p.stats.achievements.as_ref().map(|a| sqlx::types::Json(a.clone())))
        .bind(if p.stats.custom.is_empty() {
            None
        } else {
            Some(sqlx::types::Json(p.stats.custom.clone()))
        })
        .execute(&mut *tx)
        .await?;

        accepted += 1;
        roblox_ids_to_sync.push(p.user_id.clone());
    }

    sqlx::query("UPDATE game_universes SET last_push_at = now() WHERE universe_id = $1")
        .bind(&universe_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    // Fan out PlayerSyncEvents for any Roblox users that are linked to a Discord account.
    for rid in &roblox_ids_to_sync {
        let _ = crate::services::sync::fan_out_game_stats_update(rid, &state).await;
    }

    Ok(Json(json!({
        "success": true,
        "accepted": accepted,
        "received": body.players.len(),
    })))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
