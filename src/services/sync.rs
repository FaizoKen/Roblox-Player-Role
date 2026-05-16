use std::collections::{HashMap, HashSet};

use futures_util::stream::{self, StreamExt};

use crate::error::AppError;
use crate::models::condition::{Condition, ConditionField, ConditionOperator};
use crate::services::auth_gateway;
use crate::services::condition_eval::{evaluate_conditions, PlayerGameStatsRow, UserCacheRow};
use crate::AppState;

#[derive(Debug, Clone)]
pub enum PlayerSyncEvent {
    PlayerUpdated { discord_id: String },
    AccountLinked { discord_id: String },
    AccountUnlinked { discord_id: String },
    /// Per-game stats refreshed for a Roblox user. Fans out to every linked Discord
    /// account.
    GameStatsUpdated { roblox_user_id: String },
}

#[derive(Debug, Clone)]
pub struct ConfigSyncEvent {
    pub guild_id: String,
    pub role_id: String,
}

/// Sync roles for a single Discord user across all guilds.
pub async fn sync_for_player(discord_id: &str, state: &AppState) -> Result<(), AppError> {
    let pool = &state.pool;
    let rl_client = &state.rl_client;

    let cache_row = sqlx::query_as::<_, (
        String,
        Option<chrono::DateTime<chrono::Utc>>,
        bool,
        i32,
        i32,
        i32,
        i32,
        serde_json::Value,
        serde_json::Value,
        serde_json::Value,
    )>(
        "SELECT uc.roblox_user_id, uc.account_created, uc.has_verified_badge, \
         uc.friends_count, uc.followers_count, uc.following_count, \
         uc.badges_count, uc.groups, uc.badges, uc.gamepasses \
         FROM user_cache uc \
         JOIN linked_accounts la ON la.roblox_user_id = uc.roblox_user_id \
         WHERE la.discord_id = $1",
    )
    .bind(discord_id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = cache_row else {
        return Ok(());
    };

    let user_cache = UserCacheRow {
        roblox_user_id: row.0.clone(),
        account_created: row.1,
        has_verified_badge: row.2,
        friends_count: row.3,
        followers_count: row.4,
        following_count: row.5,
        badges_count: row.6,
        groups: row.7,
        badges: row.8,
        gamepasses: row.9,
    };

    let guild_ids = auth_gateway::fetch_user_guild_ids(
        &state.http,
        &state.config.auth_gateway_url,
        &state.config.internal_api_key,
        discord_id,
    )
    .await?;

    if guild_ids.is_empty() {
        return Ok(());
    }

    let role_links = sqlx::query_as::<_, (String, String, String, sqlx::types::Json<Vec<Condition>>)>(
        "SELECT rl.guild_id, rl.role_id, rl.api_token, rl.conditions \
         FROM role_links rl \
         WHERE rl.guild_id = ANY($1)",
    )
    .bind(&guild_ids[..])
    .fetch_all(pool)
    .await?;

    let needed_universes: HashSet<String> = role_links
        .iter()
        .flat_map(|(_, _, _, conditions)| {
            conditions.iter().filter_map(|c| c.universe_id.clone())
        })
        .collect();

    // Defense-in-depth against stale role_links: only honor (guild_id, universe_id)
    // pairs that are still registered in game_universes. post_config validates at
    // save time, but a universe can be deleted afterward.
    let needed_guild_ids: HashSet<String> =
        role_links.iter().map(|(g, _, _, _)| g.clone()).collect();
    let authorized_pairs: HashSet<(String, String)> = if needed_universes.is_empty() {
        HashSet::new()
    } else {
        sqlx::query_as::<_, (String, String)>(
            "SELECT guild_id, universe_id FROM game_universes \
             WHERE guild_id = ANY($1) AND universe_id = ANY($2)",
        )
        .bind(needed_guild_ids.iter().cloned().collect::<Vec<_>>())
        .bind(needed_universes.iter().cloned().collect::<Vec<_>>())
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect()
    };

    let mut game_stats: HashMap<String, PlayerGameStatsRow> = HashMap::new();
    for universe_id in &needed_universes {
        if let Ok(Some(g)) = sqlx::query_as::<_, (serde_json::Value,)>(
            "SELECT custom FROM player_game_stats \
             WHERE roblox_user_id = $1 AND universe_id = $2",
        )
        .bind(&user_cache.roblox_user_id)
        .bind(universe_id)
        .fetch_optional(pool)
        .await
        {
            game_stats.insert(
                universe_id.clone(),
                PlayerGameStatsRow { custom: g.0 },
            );
        }
    }

    let existing: HashSet<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT guild_id, role_id FROM role_assignments WHERE discord_id = $1",
    )
    .bind(discord_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();

    enum Action {
        Add { guild_id: String, role_id: String, api_token: String },
        Remove { guild_id: String, role_id: String, api_token: String },
    }

    let mut actions: Vec<Action> = Vec::new();
    for (guild_id, role_id, api_token, conditions) in &role_links {
        // If any condition references a universe not registered for this guild,
        // the role does not qualify (treated as condition failure).
        let unauthorized = conditions.iter().any(|c| {
            c.universe_id
                .as_ref()
                .is_some_and(|u| !authorized_pairs.contains(&(guild_id.clone(), u.clone())))
        });
        let qualifies = !unauthorized && evaluate_conditions(conditions, &user_cache, &game_stats);
        let assigned = existing.contains(&(guild_id.clone(), role_id.clone()));
        match (qualifies, assigned) {
            (true, false) => actions.push(Action::Add {
                guild_id: guild_id.clone(),
                role_id: role_id.clone(),
                api_token: api_token.clone(),
            }),
            (false, true) => actions.push(Action::Remove {
                guild_id: guild_id.clone(),
                role_id: role_id.clone(),
                api_token: api_token.clone(),
            }),
            _ => {}
        }
    }

    if actions.is_empty() {
        return Ok(());
    }

    let discord_id_owned = discord_id.to_string();
    stream::iter(actions)
        .for_each_concurrent(10, |action| {
            let pool = pool.clone();
            let rl_client = rl_client.clone();
            let discord_id = discord_id_owned.clone();
            async move {
                match action {
                    Action::Add { guild_id, role_id, api_token } => {
                        match rl_client.add_user(&guild_id, &role_id, &discord_id, &api_token).await {
                            Err(AppError::RoleLinkNotFound) => {
                                delete_orphan_role_link(&guild_id, &role_id, &pool).await;
                                return;
                            }
                            Err(AppError::UserLimitReached { limit }) => {
                                tracing::warn!(guild_id, role_id, discord_id, limit, "User limit reached");
                                return;
                            }
                            Err(e) => {
                                tracing::error!(guild_id, role_id, discord_id, "add_user failed: {e}");
                                return;
                            }
                            Ok(_) => {}
                        }
                        if let Err(e) = sqlx::query(
                            "INSERT INTO role_assignments (guild_id, role_id, discord_id) \
                             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                        )
                        .bind(&guild_id).bind(&role_id).bind(&discord_id)
                        .execute(&pool).await {
                            tracing::error!(guild_id, role_id, discord_id, "Insert assignment: {e}");
                        }
                    }
                    Action::Remove { guild_id, role_id, api_token } => {
                        match rl_client.remove_user(&guild_id, &role_id, &discord_id, &api_token).await {
                            Err(AppError::RoleLinkNotFound) => {
                                delete_orphan_role_link(&guild_id, &role_id, &pool).await;
                                return;
                            }
                            Err(e) => {
                                tracing::error!(guild_id, role_id, discord_id, "remove_user failed: {e}");
                                return;
                            }
                            Ok(_) => {}
                        }
                        if let Err(e) = sqlx::query(
                            "DELETE FROM role_assignments WHERE guild_id = $1 AND role_id = $2 AND discord_id = $3",
                        )
                        .bind(&guild_id).bind(&role_id).bind(&discord_id)
                        .execute(&pool).await {
                            tracing::error!(guild_id, role_id, discord_id, "Delete assignment: {e}");
                        }
                    }
                }
            }
        })
        .await;

    Ok(())
}

/// Bind value types for dynamic condition queries.
enum ConditionBind {
    Int(i64),
    Bool(bool),
}

/// Build a SQL WHERE fragment that pushes condition filtering to Postgres for the
/// fast bulk path. Returns ("TRUE"/"FALSE"/clauses, binds, needs_eval) where
/// needs_eval=true means at least one condition could not be SQL-resolved
/// (custom JSON / per-game extras) and the candidates must be re-checked in Rust.
fn build_condition_where(conditions: &[Condition]) -> (String, Vec<ConditionBind>, bool) {
    if conditions.is_empty() {
        return ("FALSE".to_string(), vec![], false);
    }

    let mut clauses: Vec<String> = Vec::new();
    let mut binds: Vec<ConditionBind> = Vec::new();
    let mut needs_eval = false;

    for condition in conditions {
        match &condition.field {
            // Account-level numeric on user_cache
            ConditionField::FriendsCount
            | ConditionField::FollowersCount
            | ConditionField::FollowingCount
            | ConditionField::BadgesCount => {
                let col = condition.field.user_cache_column().unwrap();
                push_numeric_clause(&mut clauses, &mut binds, col, condition);
            }
            ConditionField::AccountAgeDays => {
                let val = condition.value.as_i64().unwrap_or(0);
                if matches!(condition.operator, ConditionOperator::Between) {
                    let end = condition.value_end.as_ref().and_then(|v| v.as_i64()).unwrap_or(val);
                    let i1 = binds.len() + 1;
                    let i2 = binds.len() + 2;
                    clauses.push(format!(
                        "EXTRACT(EPOCH FROM (now() - uc.account_created)) / 86400 BETWEEN ${i1} AND ${i2}"
                    ));
                    binds.push(ConditionBind::Int(val));
                    binds.push(ConditionBind::Int(end));
                } else {
                    let op = condition.operator.sql_operator();
                    let i = binds.len() + 1;
                    clauses.push(format!(
                        "EXTRACT(EPOCH FROM (now() - uc.account_created)) / 86400 {op} ${i}"
                    ));
                    binds.push(ConditionBind::Int(val));
                }
            }
            ConditionField::HasVerifiedBadge => {
                let col = condition.field.user_cache_column().unwrap();
                let val = condition.value.as_bool().unwrap_or(true);
                let i = binds.len() + 1;
                clauses.push(format!("{col} = ${i}"));
                binds.push(ConditionBind::Bool(val));
            }
            // JSONB-backed sets
            ConditionField::OwnsBadge => {
                if let Some(id) = &condition.badge_id {
                    let expected = condition.value.as_bool().unwrap_or(true);
                    let cl = format!(
                        "uc.badges @> to_jsonb(ARRAY['{}']::text[])",
                        sql_escape_text(id)
                    );
                    clauses.push(if expected { cl } else { format!("NOT ({cl})") });
                } else {
                    return ("FALSE".into(), vec![], false);
                }
            }
            ConditionField::OwnsGamepass | ConditionField::OwnsAsset => {
                let id = condition.gamepass_id.as_ref().or(condition.asset_id.as_ref());
                if let Some(id) = id {
                    let expected = condition.value.as_bool().unwrap_or(true);
                    let cl = format!(
                        "uc.gamepasses @> to_jsonb(ARRAY['{}']::text[])",
                        sql_escape_text(id)
                    );
                    clauses.push(if expected { cl } else { format!("NOT ({cl})") });
                } else {
                    return ("FALSE".into(), vec![], false);
                }
            }
            ConditionField::InGroup => {
                if let Some(gid) = &condition.group_id {
                    let expected = condition.value.as_bool().unwrap_or(true);
                    let escaped = sql_escape_text(gid);
                    let cl = format!(
                        "EXISTS (SELECT 1 FROM jsonb_array_elements(uc.groups) g WHERE \
                         (g->>'group_id') = '{escaped}')"
                    );
                    clauses.push(if expected { cl } else { format!("NOT ({cl})") });
                } else {
                    return ("FALSE".into(), vec![], false);
                }
            }
            ConditionField::GroupRoleRank => {
                if let Some(gid) = &condition.group_id {
                    let val = condition.value.as_i64().unwrap_or(0);
                    let escaped = sql_escape_text(gid);
                    if matches!(condition.operator, ConditionOperator::Between) {
                        let end = condition.value_end.as_ref().and_then(|v| v.as_i64()).unwrap_or(val);
                        let i1 = binds.len() + 1;
                        let i2 = binds.len() + 2;
                        clauses.push(format!(
                            "EXISTS (SELECT 1 FROM jsonb_array_elements(uc.groups) g WHERE \
                             (g->>'group_id') = '{escaped}' AND (g->>'role_rank')::int BETWEEN ${i1} AND ${i2})"
                        ));
                        binds.push(ConditionBind::Int(val));
                        binds.push(ConditionBind::Int(end));
                    } else {
                        let op = condition.operator.sql_operator();
                        let i = binds.len() + 1;
                        clauses.push(format!(
                            "EXISTS (SELECT 1 FROM jsonb_array_elements(uc.groups) g WHERE \
                             (g->>'group_id') = '{escaped}' AND (g->>'role_rank')::int {op} ${i})"
                        ));
                        binds.push(ConditionBind::Int(val));
                    }
                } else {
                    return ("FALSE".into(), vec![], false);
                }
            }
            // Per-game custom stats live in JSONB; resolve in Rust on the
            // post-filter pass instead of pushing into SQL.
            ConditionField::CustomNumeric
            | ConditionField::CustomBoolean
            | ConditionField::CustomString => {
                needs_eval = true;
            }
        }
    }

    let where_str = if clauses.is_empty() {
        "TRUE".to_string()
    } else {
        clauses.join(" AND ")
    };
    (where_str, binds, needs_eval)
}

fn push_numeric_clause(
    clauses: &mut Vec<String>,
    binds: &mut Vec<ConditionBind>,
    col: &str,
    condition: &Condition,
) {
    let val = condition.value.as_i64().unwrap_or(0);
    if matches!(condition.operator, ConditionOperator::Between) {
        let end = condition.value_end.as_ref().and_then(|v| v.as_i64()).unwrap_or(val);
        let i1 = binds.len() + 1;
        let i2 = binds.len() + 2;
        clauses.push(format!("{col} BETWEEN ${i1} AND ${i2}"));
        binds.push(ConditionBind::Int(val));
        binds.push(ConditionBind::Int(end));
    } else {
        let op = condition.operator.sql_operator();
        let i = binds.len() + 1;
        clauses.push(format!("{col} {op} ${i}"));
        binds.push(ConditionBind::Int(val));
    }
}

fn sql_escape_text(s: &str) -> String {
    s.replace('\'', "''")
}

/// Re-evaluate every member of a guild for one role link (after config change).
/// Uses the chunked-upload path on RoleLogic so it scales to 30M.
pub async fn sync_for_role_link(
    guild_id: &str,
    role_id: &str,
    state: &AppState,
) -> Result<(), AppError> {
    let pool = &state.pool;
    let rl_client = &state.rl_client;

    let link = sqlx::query_as::<_, (String, sqlx::types::Json<Vec<Condition>>)>(
        "SELECT api_token, conditions FROM role_links WHERE guild_id = $1 AND role_id = $2",
    )
    .bind(guild_id)
    .bind(role_id)
    .fetch_optional(pool)
    .await?;

    let Some((api_token, conditions_wrap)) = link else {
        return Ok(());
    };
    let conditions: Vec<Condition> = conditions_wrap.0;

    // Convention 42: empty conditions → grant to nobody.
    if conditions.is_empty() {
        match rl_client.replace_users_scalable(guild_id, role_id, &[], &api_token).await {
            Ok(_) => {}
            Err(AppError::RoleLinkNotFound) => {
                delete_orphan_role_link(guild_id, role_id, pool).await;
                return Ok(());
            }
            Err(e) => return Err(e),
        }
        sqlx::query("DELETE FROM role_assignments WHERE guild_id = $1 AND role_id = $2")
            .bind(guild_id)
            .bind(role_id)
            .execute(pool)
            .await?;
        tracing::info!(guild_id, role_id, "Cleared role (no conditions configured)");
        return Ok(());
    }

    // Defense-in-depth: if any condition references a universe not currently
    // registered for this guild (e.g. owner deleted it after config was saved),
    // treat the role as ungrantable and clear it. post_config blocks this at
    // save time, but registrations can change afterward.
    let referenced_universes: Vec<String> = conditions
        .iter()
        .filter_map(|c| c.universe_id.clone())
        .collect();
    if !referenced_universes.is_empty() {
        let registered: Vec<(String,)> = sqlx::query_as(
            "SELECT universe_id FROM game_universes \
             WHERE guild_id = $1 AND universe_id = ANY($2)",
        )
        .bind(guild_id)
        .bind(&referenced_universes)
        .fetch_all(pool)
        .await?;
        let registered_set: HashSet<String> = registered.into_iter().map(|(u,)| u).collect();
        if referenced_universes.iter().any(|u| !registered_set.contains(u)) {
            tracing::warn!(
                guild_id,
                role_id,
                "Role references unregistered universe(s); clearing assignments"
            );
            match rl_client.replace_users_scalable(guild_id, role_id, &[], &api_token).await {
                Ok(_) => {}
                Err(AppError::RoleLinkNotFound) => {
                    delete_orphan_role_link(guild_id, role_id, pool).await;
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
            sqlx::query("DELETE FROM role_assignments WHERE guild_id = $1 AND role_id = $2")
                .bind(guild_id)
                .bind(role_id)
                .execute(pool)
                .await?;
            return Ok(());
        }
    }

    let (member_ids, _guild_name) = auth_gateway::fetch_guild_member_ids(
        &state.http,
        &state.config.auth_gateway_url,
        &state.config.internal_api_key,
        guild_id,
    )
    .await?;

    if member_ids.is_empty() {
        match rl_client.replace_users_scalable(guild_id, role_id, &[], &api_token).await {
            Ok(_) => {}
            Err(AppError::RoleLinkNotFound) => {
                delete_orphan_role_link(guild_id, role_id, pool).await;
                return Ok(());
            }
            Err(e) => return Err(e),
        }
        sqlx::query("DELETE FROM role_assignments WHERE guild_id = $1 AND role_id = $2")
            .bind(guild_id)
            .bind(role_id)
            .execute(pool)
            .await?;
        return Ok(());
    }

    // Build per-universe LEFT JOIN aliases for game-specific conditions
    let mut universe_alias_map: HashMap<String, String> = HashMap::new();
    let mut universe_joins = String::new();
    let mut next_alias_idx = 0usize;
    for c in &conditions {
        if let Some(uid) = &c.universe_id {
            if !universe_alias_map.contains_key(uid) {
                let alias = format!("pgs{next_alias_idx}");
                next_alias_idx += 1;
                universe_joins.push_str(&format!(
                    " LEFT JOIN player_game_stats {alias} ON {alias}.roblox_user_id = la.roblox_user_id AND {alias}.universe_id = '{}'",
                    sql_escape_text(uid)
                ));
                universe_alias_map.insert(uid.clone(), alias);
            }
        }
    }

    let (where_clause, binds, needs_eval) = build_condition_where(&conditions);

    let qualifying_ids: Vec<String> = if needs_eval {
        // Fall back to the fetch-and-evaluate path for custom JSON / achievement conditions
        evaluate_role_link_in_memory(&conditions, &member_ids, &universe_alias_map, state).await?
    } else {
        // Pure SQL path
        let members_idx = binds.len() + 1;
        let sql = format!(
            "SELECT la.discord_id \
             FROM linked_accounts la \
             JOIN user_cache uc ON uc.roblox_user_id = la.roblox_user_id{universe_joins} \
             WHERE la.discord_id = ANY(${members_idx}::text[]) AND ({where_clause}) \
             ORDER BY la.linked_at ASC"
        );
        let mut q = sqlx::query_scalar::<_, String>(&sql);
        for b in &binds {
            q = match b {
                ConditionBind::Int(v) => q.bind(*v),
                ConditionBind::Bool(v) => q.bind(*v),
            };
        }
        q = q.bind(&member_ids);
        q.fetch_all(pool).await?
    };

    tracing::info!(
        guild_id,
        role_id,
        candidates = qualifying_ids.len(),
        members = member_ids.len(),
        "Bulk sync: pushing to RoleLogic"
    );

    match rl_client
        .replace_users_scalable(guild_id, role_id, &qualifying_ids, &api_token)
        .await
    {
        Ok(_) => {}
        Err(AppError::RoleLinkNotFound) => {
            delete_orphan_role_link(guild_id, role_id, pool).await;
            return Ok(());
        }
        Err(e) => return Err(e),
    }

    // Single-tx rebuild of local role_assignments
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM role_assignments WHERE guild_id = $1 AND role_id = $2")
        .bind(guild_id)
        .bind(role_id)
        .execute(&mut *tx)
        .await?;

    // Insert in 50k chunks via UNNEST
    for chunk in qualifying_ids.chunks(50_000) {
        sqlx::query(
            "INSERT INTO role_assignments (guild_id, role_id, discord_id) \
             SELECT $1, $2, UNNEST($3::text[]) ON CONFLICT DO NOTHING",
        )
        .bind(guild_id)
        .bind(role_id)
        .bind(chunk)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    Ok(())
}

/// Slow path: fetch all candidates that pass the SQL pre-filter, then evaluate
/// custom-JSON / achievement conditions in Rust. Used only when the condition
/// set includes fields that can't be SQL-resolved cleanly.
async fn evaluate_role_link_in_memory(
    conditions: &[Condition],
    member_ids: &[String],
    universe_alias_map: &HashMap<String, String>,
    state: &AppState,
) -> Result<Vec<String>, AppError> {
    let pool = &state.pool;

    let mut universe_joins = String::new();
    for (uid, alias) in universe_alias_map {
        universe_joins.push_str(&format!(
            " LEFT JOIN player_game_stats {alias} ON {alias}.roblox_user_id = la.roblox_user_id AND {alias}.universe_id = '{}'",
            sql_escape_text(uid)
        ));
    }

    let select_extras: Vec<String> = universe_alias_map
        .values()
        .map(|a| format!("{a}.custom AS {a}_custom"))
        .collect();

    let extras_sql = if select_extras.is_empty() {
        String::new()
    } else {
        format!(", {}", select_extras.join(", "))
    };

    let sql = format!(
        "SELECT la.discord_id, uc.roblox_user_id, uc.account_created, \
         uc.has_verified_badge, uc.friends_count, uc.followers_count, \
         uc.following_count, uc.badges_count, uc.groups, uc.badges, uc.gamepasses{extras_sql} \
         FROM linked_accounts la \
         JOIN user_cache uc ON uc.roblox_user_id = la.roblox_user_id{universe_joins} \
         WHERE la.discord_id = ANY($1::text[]) \
         ORDER BY la.linked_at ASC"
    );

    use sqlx::Row;
    let rows = sqlx::query(&sql).bind(member_ids).fetch_all(pool).await?;

    let mut qualifying: Vec<String> = Vec::new();
    for r in rows {
        let uc = UserCacheRow {
            roblox_user_id: r.get::<String, _>("roblox_user_id"),
            account_created: r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("account_created"),
            has_verified_badge: r.get::<bool, _>("has_verified_badge"),
            friends_count: r.get::<i32, _>("friends_count"),
            followers_count: r.get::<i32, _>("followers_count"),
            following_count: r.get::<i32, _>("following_count"),
            badges_count: r.get::<i32, _>("badges_count"),
            groups: r.get::<serde_json::Value, _>("groups"),
            badges: r.get::<serde_json::Value, _>("badges"),
            gamepasses: r.get::<serde_json::Value, _>("gamepasses"),
        };

        let mut game_stats: HashMap<String, PlayerGameStatsRow> = HashMap::new();
        for (uid, alias) in universe_alias_map {
            // Each LEFT JOIN may produce NULLs; treat absent rows as no stats.
            let custom: Option<serde_json::Value> =
                r.try_get(format!("{alias}_custom").as_str()).ok();
            if let Some(c) = custom {
                game_stats.insert(uid.clone(), PlayerGameStatsRow { custom: c });
            }
        }

        if evaluate_conditions(conditions, &uc, &game_stats) {
            qualifying.push(r.get::<String, _>("discord_id"));
        }
    }
    Ok(qualifying)
}

/// Remove a user from all roles after they unlink.
pub async fn remove_all_assignments(discord_id: &str, state: &AppState) -> Result<(), AppError> {
    let pool = &state.pool;
    let rl_client = &state.rl_client;

    let assignments = sqlx::query_as::<_, (String, String, String)>(
        "SELECT ra.guild_id, ra.role_id, rl.api_token \
         FROM role_assignments ra \
         JOIN role_links rl ON rl.guild_id = ra.guild_id AND rl.role_id = ra.role_id \
         WHERE ra.discord_id = $1",
    )
    .bind(discord_id)
    .fetch_all(pool)
    .await?;

    for (guild_id, role_id, api_token) in &assignments {
        match rl_client.remove_user(guild_id, role_id, discord_id, api_token).await {
            Ok(_) => {}
            Err(AppError::RoleLinkNotFound) => {
                delete_orphan_role_link(guild_id, role_id, pool).await;
            }
            Err(e) => {
                tracing::error!(guild_id, role_id, discord_id, "Failed to remove during unlink: {e}");
            }
        }
    }

    sqlx::query("DELETE FROM role_assignments WHERE discord_id = $1")
        .bind(discord_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Delete a role_link the RoleLogic API reports as gone (403 Invalid or
/// revoked token). CASCADE clears role_assignments. Best-effort: logs DB
/// failures, never propagates them — sync workers must not stop syncing
/// other links over a cleanup hiccup.
async fn delete_orphan_role_link(guild_id: &str, role_id: &str, pool: &sqlx::PgPool) {
    tracing::warn!(
        guild_id,
        role_id,
        "Role link not found on RoleLogic; removing orphaned local row"
    );
    if let Err(e) = sqlx::query("DELETE FROM role_links WHERE guild_id = $1 AND role_id = $2")
        .bind(guild_id)
        .bind(role_id)
        .execute(pool)
        .await
    {
        tracing::error!(guild_id, role_id, "Failed to delete orphan role_link: {e}");
    }
}

/// Fan a `GameStatsUpdated` event out to every linked Discord account.
pub async fn fan_out_game_stats_update(
    roblox_user_id: &str,
    state: &AppState,
) -> Result<(), AppError> {
    let pool = &state.pool;
    let discord_ids: Vec<String> = sqlx::query_scalar::<_, String>(
        "SELECT discord_id FROM linked_accounts WHERE roblox_user_id = $1",
    )
    .bind(roblox_user_id)
    .fetch_all(pool)
    .await?;

    for did in discord_ids {
        let _ = state
            .player_sync_tx
            .try_send(PlayerSyncEvent::PlayerUpdated { discord_id: did });
    }
    Ok(())
}
