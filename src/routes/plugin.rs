use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

use crate::error::AppError;
use crate::schema;
use crate::services::sync::ConfigSyncEvent;
use crate::AppState;

fn extract_token(headers: &HeaderMap) -> Result<String, AppError> {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Unauthorized)?;
    let token = auth.strip_prefix("Token ").ok_or(AppError::Unauthorized)?;
    Ok(token.to_string())
}

#[derive(Deserialize)]
pub struct RegisterBody {
    pub guild_id: String,
    pub role_id: String,
}

pub async fn register(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<RegisterBody>,
) -> Result<Json<Value>, AppError> {
    let token = extract_token(&headers)?;

    sqlx::query(
        "INSERT INTO role_links (guild_id, role_id, api_token) VALUES ($1, $2, $3) \
         ON CONFLICT (guild_id, role_id) DO UPDATE SET api_token = $3, updated_at = now()",
    )
    .bind(&body.guild_id)
    .bind(&body.role_id)
    .bind(&token)
    .execute(&state.pool)
    .await?;

    sqlx::query("INSERT INTO guild_settings (guild_id) VALUES ($1) ON CONFLICT (guild_id) DO NOTHING")
        .bind(&body.guild_id)
        .execute(&state.pool)
        .await?;

    tracing::info!(guild_id = body.guild_id, role_id = body.role_id, "Role link registered");
    Ok(Json(serde_json::json!({"success": true})))
}

pub async fn get_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let token = extract_token(&headers)?;

    let link = sqlx::query_as::<_, (String, sqlx::types::Json<Vec<crate::models::condition::Condition>>)>(
        "SELECT guild_id, conditions FROM role_links WHERE api_token = $1",
    )
    .bind(&token)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::Unauthorized)?;

    let view_permission: String = sqlx::query_scalar(
        "SELECT view_permission FROM guild_settings WHERE guild_id = $1",
    )
    .bind(&link.0)
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(|| "members".to_string());

    // Registered universes for this guild — surfaced as the universe dropdown.
    let universes: Vec<(String, String)> = sqlx::query_as(
        "SELECT universe_id, display_name FROM game_universes \
         WHERE guild_id = $1 ORDER BY display_name ASC",
    )
    .bind(&link.0)
    .fetch_all(&state.pool)
    .await?;

    // Observed (universe, custom_key, jsonb_type) tuples — surfaced as the
    // stat-key dropdown. We pick the most-recent observation per (universe,
    // key) to infer type. Universes with zero player rows yet won't appear
    // here — the schema falls back to a placeholder option.
    let stats: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT DISTINCT ON (g.universe_id, kv.key) \
            g.universe_id, kv.key, jsonb_typeof(kv.value) \
         FROM game_universes g \
         JOIN player_game_stats p ON p.universe_id = g.universe_id \
         JOIN LATERAL jsonb_each(p.custom) AS kv(key, value) ON TRUE \
         WHERE g.guild_id = $1 \
         ORDER BY g.universe_id, kv.key, p.fetched_at DESC",
    )
    .bind(&link.0)
    .fetch_all(&state.pool)
    .await?;

    let verify_url = format!("{}/verify", state.config.base_url);
    let players_url = format!("{}/players/{}", state.config.base_url, link.0);
    let games_url = format!("{}/games/{}", state.config.base_url, link.0);

    let s = schema::build_config_schema(
        &link.1,
        &verify_url,
        &players_url,
        &games_url,
        &view_permission,
        &universes,
        &stats,
    );
    Ok(Json(s))
}

#[derive(Deserialize)]
pub struct ConfigBody {
    pub guild_id: String,
    pub role_id: String,
    pub config: HashMap<String, Value>,
}

pub async fn post_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ConfigBody>,
) -> Result<Json<Value>, AppError> {
    let token = extract_token(&headers)?;

    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM role_links WHERE guild_id = $1 AND role_id = $2 AND api_token = $3)",
    )
    .bind(&body.guild_id)
    .bind(&body.role_id)
    .bind(&token)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(false);

    if !exists {
        return Err(AppError::Unauthorized);
    }

    let conditions = schema::parse_config(&body.config)?;

    // Each Game-category condition references a universe_id. Universes are
    // registered per-(guild_id, owner) — reject any universe not registered
    // for THIS guild so admins can't piggyback on another server's stats.
    use std::collections::HashSet;
    let referenced: HashSet<String> = conditions
        .iter()
        .filter_map(|c| c.universe_id.clone())
        .collect();
    if !referenced.is_empty() {
        let registered: Vec<(String,)> = sqlx::query_as(
            "SELECT universe_id FROM game_universes \
             WHERE guild_id = $1 AND universe_id = ANY($2)",
        )
        .bind(&body.guild_id)
        .bind(referenced.iter().cloned().collect::<Vec<_>>())
        .fetch_all(&state.pool)
        .await?;
        let registered_set: HashSet<String> = registered.into_iter().map(|(u,)| u).collect();
        if let Some(missing) = referenced.iter().find(|u| !registered_set.contains(*u)) {
            return Err(AppError::BadRequest(format!(
                "Universe {missing} is not registered for this server. Visit the Games page and register it first."
            )));
        }
    }

    let view_permission = body
        .config
        .get("view_permission")
        .and_then(|v| v.as_str())
        .unwrap_or("members")
        .to_string();
    if view_permission != "members" && view_permission != "managers" {
        return Err(AppError::BadRequest(
            "view_permission must be 'members' or 'managers'".into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE role_links SET conditions = $1, updated_at = now() \
         WHERE guild_id = $2 AND role_id = $3",
    )
    .bind(sqlx::types::Json(&conditions))
    .bind(&body.guild_id)
    .bind(&body.role_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO guild_settings (guild_id, view_permission, updated_at) VALUES ($1, $2, now()) \
         ON CONFLICT (guild_id) DO UPDATE SET view_permission = EXCLUDED.view_permission, updated_at = now()",
    )
    .bind(&body.guild_id)
    .bind(&view_permission)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    tracing::info!(
        guild_id = body.guild_id,
        role_id = body.role_id,
        count = conditions.len(),
        "Config updated"
    );

    let _ = state.config_sync_tx.try_send(ConfigSyncEvent {
        guild_id: body.guild_id,
        role_id: body.role_id,
    });

    Ok(Json(serde_json::json!({"success": true})))
}

#[derive(Deserialize)]
pub struct DeleteConfigBody {
    pub guild_id: String,
    pub role_id: String,
}

pub async fn delete_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<DeleteConfigBody>,
) -> Result<Json<Value>, AppError> {
    let token = extract_token(&headers)?;

    let result = sqlx::query(
        "DELETE FROM role_links WHERE guild_id = $1 AND role_id = $2 AND api_token = $3",
    )
    .bind(&body.guild_id)
    .bind(&body.role_id)
    .bind(&token)
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::Unauthorized);
    }

    tracing::info!(guild_id = body.guild_id, role_id = body.role_id, "Role link deleted");
    Ok(Json(serde_json::json!({"success": true})))
}
