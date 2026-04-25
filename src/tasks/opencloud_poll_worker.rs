//! Polls Open Cloud DataStore for every universe with `pull_enabled = TRUE`.
//! Reads each linked Roblox user's entry, applies the admin's stat_field_map,
//! and upserts into player_game_stats.

use std::sync::Arc;

use crate::services::roblox_open_cloud::{apply_field_map, OpenCloudClient};
use crate::AppState;

pub async fn run(state: Arc<AppState>) {
    tracing::info!("Open Cloud poll worker started");
    let client = OpenCloudClient::new(state.config.open_cloud_rate_limit);

    loop {
        // Pick the universe whose last_pull_at is oldest and is due
        let due = sqlx::query_as::<_, (
            String,
            String,
            String,
            i32,
            sqlx::types::Json<serde_json::Value>,
        )>(
            "SELECT universe_id, open_cloud_api_key_encrypted, datastore_name, poll_interval_seconds, stat_field_map \
             FROM game_universes \
             WHERE pull_enabled = TRUE \
               AND open_cloud_api_key_encrypted IS NOT NULL \
               AND datastore_name IS NOT NULL \
               AND (last_pull_at IS NULL OR last_pull_at + (poll_interval_seconds || ' seconds')::interval <= now()) \
             ORDER BY last_pull_at NULLS FIRST \
             LIMIT 1",
        )
        .fetch_optional(&state.pool)
        .await;

        let (universe_id, key_encrypted, datastore_name, _interval, stat_map) = match due {
            Ok(Some(r)) => r,
            Ok(None) => {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                continue;
            }
            Err(e) => {
                tracing::error!("opencloud_poll DB error: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                continue;
            }
        };

        let api_key = match crate::services::crypto::decrypt(&state.encryption_key, &key_encrypted) {
            Ok(k) => k,
            Err(e) => {
                tracing::error!(universe_id, "Failed to decrypt Open Cloud key: {e}");
                let _ = sqlx::query("UPDATE game_universes SET last_pull_at = now() WHERE universe_id = $1")
                    .bind(&universe_id)
                    .execute(&state.pool)
                    .await;
                continue;
            }
        };

        let stat_map_obj: serde_json::Map<String, serde_json::Value> = stat_map
            .0
            .as_object()
            .cloned()
            .unwrap_or_default();

        // Pull stats for every linked Roblox user. We deliberately keep this
        // simple — at very high user counts, switch to Ordered DataStores for
        // server-side sorting + paging.
        let linked: Vec<String> = sqlx::query_scalar::<_, String>(
            "SELECT roblox_user_id FROM linked_accounts ORDER BY linked_at ASC LIMIT 5000",
        )
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

        let mut updated = 0usize;
        for rid in &linked {
            match client
                .read_datastore_entry(&universe_id, &datastore_name, rid, &api_key)
                .await
            {
                Ok(Some(raw)) => {
                    let mapped = apply_field_map(&raw, &stat_map_obj);
                    if mapped.is_empty() {
                        continue;
                    }
                    if let Err(e) = upsert_mapped(&state, rid, &universe_id, &mapped).await {
                        tracing::error!(rid, universe_id, "upsert mapped stats failed: {e}");
                        continue;
                    }
                    let _ = crate::services::sync::fan_out_game_stats_update(rid, &state).await;
                    updated += 1;
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(rid, universe_id, "Open Cloud read failed: {e}");
                }
            }
        }

        let _ = sqlx::query("UPDATE game_universes SET last_pull_at = now() WHERE universe_id = $1")
            .bind(&universe_id)
            .execute(&state.pool)
            .await;

        tracing::info!(universe_id, updated, "Open Cloud pull complete");
    }
}

async fn upsert_mapped(
    state: &AppState,
    roblox_user_id: &str,
    universe_id: &str,
    mapped: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), crate::error::AppError> {
    let playtime = mapped.get("playtime_minutes").and_then(|v| v.as_i64());
    let level = mapped.get("level").and_then(|v| v.as_i64());
    let wins = mapped.get("wins").and_then(|v| v.as_i64());
    let losses = mapped.get("losses").and_then(|v| v.as_i64());
    let currency = mapped.get("currency").and_then(|v| v.as_i64());
    let achievements = mapped.get("achievements").cloned();

    // Anything not in the fixed columns goes into `custom`
    let mut custom = serde_json::Map::new();
    for (k, v) in mapped {
        if !matches!(
            k.as_str(),
            "playtime_minutes" | "level" | "wins" | "losses" | "currency" | "achievements"
        ) {
            custom.insert(k.clone(), v.clone());
        }
    }

    sqlx::query(
        "INSERT INTO player_game_stats (roblox_user_id, universe_id, \
            playtime_minutes, level, wins, losses, currency, achievements, custom, fetched_at) \
         VALUES ($1, $2, COALESCE($3::int, 0), COALESCE($4::int, 0), COALESCE($5::int, 0), \
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
    .bind(roblox_user_id)
    .bind(universe_id)
    .bind(playtime.map(|v| v as i32))
    .bind(level.map(|v| v as i32))
    .bind(wins.map(|v| v as i32))
    .bind(losses.map(|v| v as i32))
    .bind(currency)
    .bind(achievements.map(sqlx::types::Json))
    .bind(if custom.is_empty() {
        None
    } else {
        Some(sqlx::types::Json(serde_json::Value::Object(custom)))
    })
    .execute(&state.pool)
    .await?;

    Ok(())
}
