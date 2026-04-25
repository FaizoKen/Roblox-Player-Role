use serde_json::{json, Value};
use std::collections::HashMap;

use crate::error::AppError;
use crate::models::condition::{Condition, ConditionCategory, ConditionField, ConditionOperator};

const NUMERIC_ACCOUNT_FIELDS: &[&str] = &[
    "accountAgeDays",
    "friendsCount",
    "followersCount",
    "followingCount",
    "badgesCount",
];

/// Encoded game-stat option value: `<universe_id>::<key>::<type>` where type
/// is one of `numeric`, `boolean`, `string`. Used for the dynamic stat
/// dropdown so a single field carries (universe, key, type) without needing
/// runtime option filtering by another field's value.
pub fn encode_game_stat(universe_id: &str, key: &str, ty: &str) -> String {
    format!("{universe_id}::{key}::{ty}")
}

/// Decode the inverse of `encode_game_stat`. Returns (universe_id, key, type).
pub fn decode_game_stat(value: &str) -> Option<(String, String, String)> {
    let mut parts = value.splitn(3, "::");
    let u = parts.next()?.to_string();
    let k = parts.next()?.to_string();
    let t = parts.next()?.to_string();
    if u.is_empty() || k.is_empty() || t.is_empty() {
        return None;
    }
    Some((u, k, t))
}

pub fn build_config_schema(
    conditions: &[Condition],
    verify_url: &str,
    players_url: &str,
    games_url: &str,
    view_permission: &str,
    universes: &[(String, String)],
    observed_stats: &[(String, String, Option<String>)],
) -> Value {
    let c = conditions.first();
    let mut values = HashMap::new();

    let category = c.map(|c| c.field.category()).unwrap_or(ConditionCategory::Account);
    values.insert("condition_category".to_string(), json!(category.key()));

    // Per-category field slots so switching category doesn't fight stale values
    values.insert(
        "condition_field_account".to_string(),
        json!(if category == ConditionCategory::Account {
            c.map(|c| c.field.json_key()).unwrap_or("")
        } else {
            ""
        }),
    );
    values.insert(
        "condition_field_group".to_string(),
        json!(if category == ConditionCategory::Group {
            c.map(|c| c.field.json_key()).unwrap_or("")
        } else {
            ""
        }),
    );
    // For Game category we encode the current selection as
    // `<universe_id>::<key>::<type>` so the dynamic stat dropdown re-selects it.
    // Legacy (non-custom) Game fields can't be re-encoded — admin must re-pick.
    let game_seed = if category == ConditionCategory::Game {
        c.and_then(|c| {
            let uid = c.universe_id.as_deref()?;
            let key = c.stat_key.as_deref();
            match c.field {
                ConditionField::CustomNumeric => key.map(|k| encode_game_stat(uid, k, "numeric")),
                ConditionField::CustomBoolean => key.map(|k| encode_game_stat(uid, k, "boolean")),
                ConditionField::CustomString => key.map(|k| encode_game_stat(uid, k, "string")),
                _ => None,
            }
        })
        .unwrap_or_default()
    } else {
        String::new()
    };
    values.insert("condition_field_game".to_string(), json!(game_seed));

    let operator_key = c.map(|c| c.operator.key()).unwrap_or("gte");
    values.insert("operator_account".to_string(), json!(operator_key));
    values.insert("operator_group".to_string(), json!(operator_key));
    values.insert("operator_game".to_string(), json!(operator_key));
    values.insert("view_permission".to_string(), json!(view_permission));

    if let Some(c) = c {
        if let Some(g) = &c.group_id {
            values.insert("group_id".to_string(), json!(g));
        }
        if let Some(u) = &c.universe_id {
            values.insert("universe_id".to_string(), json!(u));
        }
        if let Some(b) = &c.badge_id {
            values.insert("badge_id".to_string(), json!(b));
        }
        if let Some(g) = &c.gamepass_id {
            values.insert("gamepass_id".to_string(), json!(g));
        }
        if let Some(a) = &c.asset_id {
            values.insert("asset_id".to_string(), json!(a));
        }
        if let Some(s) = &c.stat_key {
            values.insert("stat_key".to_string(), json!(s));
        }

        let cat_suffix = category.key();
        if c.field.is_boolean() {
            let bool_val = c.value.as_bool().unwrap_or(true);
            values.insert(
                format!("value_bool_{cat_suffix}"),
                json!(if bool_val { "true" } else { "false" }),
            );
        } else if c.field.is_string_exact() {
            let s = c.value.as_str().unwrap_or("").to_string();
            values.insert(format!("value_string_{cat_suffix}"), json!(s));
        } else if let Some(n) = c.value.as_i64() {
            values.insert(format!("value_num_{cat_suffix}"), json!(n));
        }

        if c.operator == ConditionOperator::Between {
            if let Some(end) = c.value_end.as_ref().and_then(|v| v.as_i64()) {
                values.insert(format!("value_end_{cat_suffix}"), json!(end));
            }
        }
    }

    // ─── Dynamic Game-stat dropdown ─────────────────────────────────────────
    //
    // Each option encodes (universe, custom_key, json_type) so the schema
    // engine — which can't filter options by another field's value — still
    // gives the admin a single picker that drives both universe selection and
    // type-aware value inputs. equals_any sets below let the value fields show
    // only when a matching-type stat is picked.
    let mut universe_label: HashMap<String, String> = HashMap::new();
    for (uid, name) in universes {
        universe_label.insert(uid.clone(), name.clone());
    }
    let mut game_stat_options: Vec<Value> = Vec::new();
    let mut numeric_game_opts: Vec<String> = Vec::new();
    let mut boolean_game_opts: Vec<String> = Vec::new();
    let mut string_game_opts: Vec<String> = Vec::new();
    for (uid, key, jtype_opt) in observed_stats {
        let jtype = jtype_opt.as_deref().unwrap_or("string");
        let bucket = match jtype {
            "number" => "numeric",
            "boolean" => "boolean",
            _ => "string",
        };
        let value = encode_game_stat(uid, key, bucket);
        let label = format!(
            "{} · {} ({})",
            universe_label.get(uid).cloned().unwrap_or_else(|| format!("Universe {uid}")),
            key,
            bucket,
        );
        game_stat_options.push(json!({"label": label, "value": value}));
        match bucket {
            "numeric" => numeric_game_opts.push(value),
            "boolean" => boolean_game_opts.push(value),
            _ => string_game_opts.push(value),
        }
    }
    // Always seed the placeholder so the dropdown isn't empty on first load.
    if game_stat_options.is_empty() {
        let hint = if universes.is_empty() {
            "(no Roblox games registered yet — visit the Games page)".to_string()
        } else {
            "(no in-game stats reported yet — push or pull at least one verified player's data first)".to_string()
        };
        game_stat_options.push(json!({"label": hint, "value": ""}));
    }
    let registered_universes_blurb: String = if universes.is_empty() {
        "Register a Roblox game on the Games page first.".into()
    } else {
        let names: Vec<String> = universes
            .iter()
            .map(|(uid, name)| format!("• {name} (universe {uid})"))
            .collect();
        format!("Registered games for this server:\n{}", names.join("\n"))
    };

    json!({
        "version": 1,
        "name": "Roblox Player Roles",
        "description": "Grant a Discord role based on a member's Roblox account or in-game progression — account age, premium, group rank, badges, gamepasses, friends, or per-game stats from any Roblox game you've integrated.",
        "sections": [
            {
                "title": "How it works",
                "fields": [{
                    "type": "display",
                    "key": "info",
                    "label": "Three steps",
                    "value": format!(
                        "1. Members link their Roblox account at:\n   {verify_url}\n\n\
                         2. Pick one condition below — for example: account age ≥ 365 days, owns a specific badge, member of a Roblox group with rank ≥ 100, or in-game level ≥ 10 in your Roblox game.\n\n\
                         3. Verified members who match get this role automatically. Roblox data refreshes on a schedule.\n\n\
                         Verified members for this server:\n   {players_url}\n\n\
                         Game creators — connect your Roblox game to grant roles based on in-game stats:\n   {games_url}"
                    )
                }]
            },
            {
                "title": "Condition",
                "description": "Pick a category — the fields below adjust to match.",
                "fields": [
                    {
                        "type": "radio",
                        "key": "condition_category",
                        "label": "Category",
                        "description": "Account = global Roblox profile signals (age, premium, friends, badges).  Group = membership/rank in a Roblox group.  Game = stats from a specific Roblox game (requires the game owner to set up integration on the Games tab).",
                        "validation": { "required": true },
                        "options": [
                            {"label": "Account-level (no extra setup)", "value": "account"},
                            {"label": "Roblox group membership / rank", "value": "group"},
                            {"label": "In-game stats (game integration required)", "value": "game"}
                        ]
                    },

                    // ─── Account branch ─────────────────────────────
                    {
                        "type": "select",
                        "key": "condition_field_account",
                        "label": "What to check",
                        "description": "Pick which account-level data the plugin should evaluate.",
                        "validation": { "required": true },
                        "condition": { "field": "condition_category", "equals": "account" },
                        "options": [
                            {"label": "Account age (days since the Roblox account was created)", "value": "accountAgeDays"},
                            {"label": "Has the Roblox 'Verified' badge", "value": "hasVerifiedBadge"},
                            {"label": "Friend count", "value": "friendsCount"},
                            {"label": "Follower count", "value": "followersCount"},
                            {"label": "Following count", "value": "followingCount"},
                            {"label": "Total badge count (across all games)", "value": "badgesCount"},
                            {"label": "Owns a specific badge", "value": "ownsBadge"},
                            {"label": "Owns a specific gamepass", "value": "ownsGamepass"},
                            {"label": "Owns a specific asset (item / hat / ugc)", "value": "ownsAsset"}
                        ]
                    },

                    // ─── Group branch ───────────────────────────────
                    {
                        "type": "select",
                        "key": "condition_field_group",
                        "label": "What to check",
                        "description": "Group membership or minimum rank within a Roblox group.",
                        "validation": { "required": true },
                        "condition": { "field": "condition_category", "equals": "group" },
                        "options": [
                            {"label": "Is a member of the group", "value": "inGroup"},
                            {"label": "Has at least a given role rank in the group", "value": "groupRoleRank"}
                        ]
                    },
                    {
                        "type": "text",
                        "key": "group_id",
                        "label": "Roblox Group ID",
                        "description": "The numeric Roblox Group ID. Find it in the URL of your group page (https://www.roblox.com/groups/12345/...).",
                        "validation": { "pattern": "^[0-9]+$", "pattern_message": "Group ID must be numeric", "required": true },
                        "condition": { "field": "condition_category", "equals": "group" }
                    },

                    // ─── Game branch ────────────────────────────────
                    {
                        "type": "display",
                        "key": "registered_games_info",
                        "label": "Game integration",
                        "value": registered_universes_blurb,
                        "condition": { "field": "condition_category", "equals": "game" }
                    },
                    {
                        "type": "select",
                        "key": "condition_field_game",
                        "label": "What to check",
                        "description": "Pick the registered Roblox game and the in-game stat to evaluate. Stats appear here once your game has reported at least one player's data (push via Studio plugin / webhook, or pull via Open Cloud DataStore).",
                        "validation": { "required": true },
                        "condition": { "field": "condition_category", "equals": "game" },
                        "options": game_stat_options
                    },

                    // ─── Comparison operator (numeric only) ─────────
                    {
                        "type": "select",
                        "key": "operator_account",
                        "label": "Comparison",
                        "default_value": "gte",
                        "conditions": [
                            { "field": "condition_category", "equals": "account" },
                            { "field": "condition_field_account", "equals_any": NUMERIC_ACCOUNT_FIELDS }
                        ],
                        "options": [
                            {"label": "= equals", "value": "eq"},
                            {"label": "> greater than", "value": "gt"},
                            {"label": ">= at least", "value": "gte"},
                            {"label": "< less than", "value": "lt"},
                            {"label": "<= at most", "value": "lte"},
                            {"label": "↔ between (range)", "value": "between"}
                        ]
                    },
                    {
                        "type": "select",
                        "key": "operator_group",
                        "label": "Comparison",
                        "default_value": "gte",
                        "conditions": [
                            { "field": "condition_category", "equals": "group" },
                            { "field": "condition_field_group", "equals": "groupRoleRank" }
                        ],
                        "options": [
                            {"label": "= equals", "value": "eq"},
                            {"label": "> greater than", "value": "gt"},
                            {"label": ">= at least", "value": "gte"},
                            {"label": "< less than", "value": "lt"},
                            {"label": "<= at most", "value": "lte"},
                            {"label": "↔ between (range)", "value": "between"}
                        ]
                    },
                    {
                        "type": "select",
                        "key": "operator_game",
                        "label": "Comparison",
                        "default_value": "gte",
                        "conditions": [
                            { "field": "condition_category", "equals": "game" },
                            { "field": "condition_field_game", "equals_any": numeric_game_opts.clone() }
                        ],
                        "options": [
                            {"label": "= equals", "value": "eq"},
                            {"label": "> greater than", "value": "gt"},
                            {"label": ">= at least", "value": "gte"},
                            {"label": "< less than", "value": "lt"},
                            {"label": "<= at most", "value": "lte"},
                            {"label": "↔ between (range)", "value": "between"}
                        ]
                    },

                    // ─── ID inputs for ownership conditions ─────────
                    {
                        "type": "text",
                        "key": "badge_id",
                        "label": "Badge ID",
                        "description": "Numeric Roblox badge ID — find it in the URL of the badge page.",
                        "validation": { "pattern": "^[0-9]+$", "pattern_message": "Badge ID must be numeric", "required": true },
                        "conditions": [
                            { "field": "condition_category", "equals": "account" },
                            { "field": "condition_field_account", "equals": "ownsBadge" }
                        ]
                    },
                    {
                        "type": "text",
                        "key": "gamepass_id",
                        "label": "Gamepass ID",
                        "description": "Numeric Roblox gamepass ID.",
                        "validation": { "pattern": "^[0-9]+$", "pattern_message": "Gamepass ID must be numeric", "required": true },
                        "conditions": [
                            { "field": "condition_category", "equals": "account" },
                            { "field": "condition_field_account", "equals": "ownsGamepass" }
                        ]
                    },
                    {
                        "type": "text",
                        "key": "asset_id",
                        "label": "Asset ID",
                        "description": "Numeric Roblox asset ID (e.g. for a UGC item).",
                        "validation": { "pattern": "^[0-9]+$", "pattern_message": "Asset ID must be numeric", "required": true },
                        "conditions": [
                            { "field": "condition_category", "equals": "account" },
                            { "field": "condition_field_account", "equals": "ownsAsset" }
                        ]
                    },

                    // ─── Account value inputs ───────────────────────
                    {
                        "type": "radio",
                        "key": "value_bool_account",
                        "label": "Value",
                        "default_value": "true",
                        "conditions": [
                            { "field": "condition_category", "equals": "account" },
                            { "field": "condition_field_account", "equals_any": ["hasVerifiedBadge","ownsBadge","ownsGamepass","ownsAsset"] }
                        ],
                        "options": [
                            {"label": "Yes — must match", "value": "true"},
                            {"label": "No — must NOT match", "value": "false"}
                        ]
                    },
                    {
                        "type": "number",
                        "key": "value_num_account",
                        "label": "Value",
                        "description": "The number to compare against (e.g. 365 for 'one year old account').",
                        "validation": { "min": 0, "required": true },
                        "conditions": [
                            { "field": "condition_category", "equals": "account" },
                            { "field": "condition_field_account", "equals_any": NUMERIC_ACCOUNT_FIELDS }
                        ]
                    },
                    {
                        "type": "number",
                        "key": "value_end_account",
                        "label": "End value",
                        "description": "Upper bound of the range (inclusive).",
                        "validation": { "min": 0, "required": true },
                        "pair_with": "value_num_account",
                        "conditions": [
                            { "field": "condition_category", "equals": "account" },
                            { "field": "condition_field_account", "equals_any": NUMERIC_ACCOUNT_FIELDS },
                            { "field": "operator_account", "equals": "between" }
                        ]
                    },

                    // ─── Group value inputs ─────────────────────────
                    {
                        "type": "radio",
                        "key": "value_bool_group",
                        "label": "Value",
                        "default_value": "true",
                        "conditions": [
                            { "field": "condition_category", "equals": "group" },
                            { "field": "condition_field_group", "equals": "inGroup" }
                        ],
                        "options": [
                            {"label": "Yes — must be a member", "value": "true"},
                            {"label": "No — must NOT be a member", "value": "false"}
                        ]
                    },
                    {
                        "type": "number",
                        "key": "value_num_group",
                        "label": "Minimum role rank",
                        "description": "Roblox group role rank (0–255). Roles in your group have a rank number — find them in your group's configure → Roles page.",
                        "validation": { "min": 0, "required": true },
                        "conditions": [
                            { "field": "condition_category", "equals": "group" },
                            { "field": "condition_field_group", "equals": "groupRoleRank" }
                        ]
                    },
                    {
                        "type": "number",
                        "key": "value_end_group",
                        "label": "End value",
                        "description": "Upper bound of the range (inclusive).",
                        "validation": { "min": 0, "required": true },
                        "pair_with": "value_num_group",
                        "conditions": [
                            { "field": "condition_category", "equals": "group" },
                            { "field": "condition_field_group", "equals": "groupRoleRank" },
                            { "field": "operator_group", "equals": "between" }
                        ]
                    },

                    // ─── Game value inputs ──────────────────────────
                    {
                        "type": "radio",
                        "key": "value_bool_game",
                        "label": "Value",
                        "default_value": "true",
                        "conditions": [
                            { "field": "condition_category", "equals": "game" },
                            { "field": "condition_field_game", "equals_any": boolean_game_opts.clone() }
                        ],
                        "options": [
                            {"label": "Yes — must match", "value": "true"},
                            {"label": "No — must NOT match", "value": "false"}
                        ]
                    },
                    {
                        "type": "number",
                        "key": "value_num_game",
                        "label": "Value",
                        "description": "The number to compare against (e.g. 600 for 600 minutes of playtime).",
                        "validation": { "min": 0, "required": true },
                        "conditions": [
                            { "field": "condition_category", "equals": "game" },
                            { "field": "condition_field_game", "equals_any": numeric_game_opts.clone() }
                        ]
                    },
                    {
                        "type": "number",
                        "key": "value_end_game",
                        "label": "End value",
                        "description": "Upper bound of the range (inclusive).",
                        "validation": { "min": 0, "required": true },
                        "pair_with": "value_num_game",
                        "conditions": [
                            { "field": "condition_category", "equals": "game" },
                            { "field": "condition_field_game", "equals_any": numeric_game_opts.clone() },
                            { "field": "operator_game", "equals": "between" }
                        ]
                    },
                    {
                        "type": "text",
                        "key": "value_string_game",
                        "label": "Value",
                        "description": "The exact value to match (case-sensitive).",
                        "validation": { "required": true },
                        "conditions": [
                            { "field": "condition_category", "equals": "game" },
                            { "field": "condition_field_game", "equals_any": string_game_opts.clone() }
                        ]
                    }
                ]
            },
            {
                "title": "Player list access",
                "description": "Choose who can view the verified-player list. Shared across every role link in the server.",
                "fields": [{
                    "type": "radio",
                    "key": "view_permission",
                    "label": "Who can view the player list",
                    "default_value": "members",
                    "options": [
                        {"label": "Anyone in the server", "value": "members"},
                        {"label": "Server managers only (Manage Server permission)", "value": "managers"}
                    ]
                }]
            },
            {
                "title": "Examples",
                "collapsible": true,
                "default_collapsed": true,
                "fields": [{
                    "type": "display",
                    "key": "examples",
                    "label": "Common setups",
                    "value": "1+ year old account → Account → Account age, >= 365\n\
                              50+ friends → Account → Friends count, >= 50\n\
                              Owns gamepass 'VIP' → Account → Owns specific gamepass, ID = your_pass_id\n\
                              Group officer → Group → Has at least rank in group, Group ID = X, >= 100\n\
                              Game stat (e.g. Level >= 10) → Game → pick the game/stat, comparison >=, value 10\n\
                              VIP flag (custom boolean) → Game → pick the game/isVip stat, value Yes"
                }]
            }
        ],
        "values": values
    })
}

pub fn parse_config(config: &HashMap<String, Value>) -> Result<Vec<Condition>, AppError> {
    let category = config
        .get("condition_category")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // For Game category, condition_field_game now carries an encoded triplet
    // `<universe_id>::<key>::<type>`; decode it into the legacy field-key the
    // rest of this function expects, plus the derived universe_id + stat_key.
    let mut decoded_universe: Option<String> = None;
    let mut decoded_stat_key: Option<String> = None;

    let field_key: String = match category {
        "account" => config
            .get("condition_field_account")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "group" => config
            .get("condition_field_group")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "game" => {
            let raw = config
                .get("condition_field_game")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let (uid, key, ty) = decode_game_stat(raw).ok_or_else(|| {
                AppError::BadRequest(
                    "Pick a registered game/stat. If the dropdown is empty, register a game on the Games page and let it report at least one player's data first.".into(),
                )
            })?;
            decoded_universe = Some(uid);
            decoded_stat_key = Some(key);
            match ty.as_str() {
                "numeric" => "customNumeric".to_string(),
                "boolean" => "customBoolean".to_string(),
                "string" => "customString".to_string(),
                other => {
                    return Err(AppError::BadRequest(format!(
                        "Unknown stat type '{other}'."
                    )))
                }
            }
        }
        _ => return Err(AppError::BadRequest("Pick a category (Account / Group / Game)".into())),
    };

    if field_key.is_empty() {
        return Err(AppError::BadRequest("Pick what to check".into()));
    }

    let field = ConditionField::from_key(&field_key)
        .ok_or_else(|| AppError::BadRequest(format!("Invalid condition field '{field_key}'")))?;

    // Cross-check category against field
    let field_cat = field.category();
    if field_cat.key() != category {
        return Err(AppError::BadRequest(format!(
            "Field '{field_key}' belongs to the {} category — switch the category.",
            field_cat.key()
        )));
    }

    let operator = if field.is_boolean() || field.is_string_exact() {
        ConditionOperator::Eq
    } else {
        let op_key = match category {
            "account" => config.get("operator_account").and_then(|v| v.as_str()).unwrap_or(""),
            "group" => config.get("operator_group").and_then(|v| v.as_str()).unwrap_or(""),
            "game" => config.get("operator_game").and_then(|v| v.as_str()).unwrap_or(""),
            _ => "",
        };
        if op_key.is_empty() {
            return Err(AppError::BadRequest("Pick a comparison (>=, =, between, …)".into()));
        }
        ConditionOperator::from_key(op_key)
            .ok_or_else(|| AppError::BadRequest(format!("Invalid operator '{op_key}'")))?
    };

    let bool_key = format!("value_bool_{category}");
    let num_key = format!("value_num_{category}");
    let end_key = format!("value_end_{category}");
    let string_key = format!("value_string_{category}");

    let value: Value = if field.is_boolean() {
        let s = config
            .get(&bool_key)
            .and_then(|v| v.as_str())
            .unwrap_or("true");
        Value::Bool(s == "true")
    } else if field.is_string_exact() {
        let s = config
            .get(&string_key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if s.is_empty() {
            return Err(AppError::BadRequest("Value is required".into()));
        }
        Value::String(s)
    } else {
        let n = config
            .get(&num_key)
            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .ok_or_else(|| AppError::BadRequest("Numeric value is required".into()))?;
        Value::Number(n.into())
    };

    let value_end = if operator == ConditionOperator::Between {
        let n = config
            .get(&end_key)
            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .ok_or_else(|| AppError::BadRequest("End value is required for the between operator".into()))?;
        if let Some(start) = value.as_i64() {
            if start > n {
                return Err(AppError::BadRequest("Start value must be ≤ end value".into()));
            }
        }
        Some(Value::Number(n.into()))
    } else {
        None
    };

    let group_id = trimmed_string(config, "group_id");
    // For Game category these come from the encoded condition_field_game
    // selection; for other categories they may still arrive via legacy keys.
    let universe_id = decoded_universe.or_else(|| trimmed_string(config, "universe_id"));
    let stat_key = decoded_stat_key.or_else(|| trimmed_string(config, "stat_key"));
    let badge_id = trimmed_string(config, "badge_id");
    let gamepass_id = trimmed_string(config, "gamepass_id");
    let asset_id = trimmed_string(config, "asset_id");

    if field.requires_group() && group_id.is_none() {
        return Err(AppError::BadRequest("Roblox Group ID is required".into()));
    }
    if field.requires_universe() && universe_id.is_none() {
        return Err(AppError::BadRequest("Roblox Universe ID is required".into()));
    }
    if field.requires_badge() && badge_id.is_none() {
        return Err(AppError::BadRequest("Badge ID is required".into()));
    }
    if field.requires_gamepass() && gamepass_id.is_none() {
        return Err(AppError::BadRequest("Gamepass ID is required".into()));
    }
    if field.requires_asset() && asset_id.is_none() {
        return Err(AppError::BadRequest("Asset ID is required".into()));
    }
    if field.requires_stat_key() && stat_key.is_none() {
        return Err(AppError::BadRequest("Stat key is required for custom stats".into()));
    }

    Ok(vec![Condition {
        field,
        operator,
        value,
        value_end,
        group_id,
        universe_id,
        badge_id,
        gamepass_id,
        asset_id,
        stat_key,
    }])
}

fn trimmed_string(config: &HashMap<String, Value>, key: &str) -> Option<String> {
    config
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
