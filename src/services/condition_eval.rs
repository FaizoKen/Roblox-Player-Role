use std::collections::HashMap;

use crate::models::condition::{Condition, ConditionField, ConditionOperator};

/// Subset of `user_cache` row used during evaluation.
pub struct UserCacheRow {
    pub roblox_user_id: String,
    pub account_created: Option<chrono::DateTime<chrono::Utc>>,
    pub has_verified_badge: bool,
    pub friends_count: i32,
    pub followers_count: i32,
    pub following_count: i32,
    pub badges_count: i32,
    pub groups: serde_json::Value,
    pub badges: serde_json::Value,
    pub gamepasses: serde_json::Value,
}

/// One row from `player_game_stats` for a specific universe.
pub struct PlayerGameStatsRow {
    pub custom: serde_json::Value,
}

/// Evaluate all conditions against cached data. All must pass (AND logic).
/// An empty condition list grants no role (Convention 42).
pub fn evaluate_conditions(
    conditions: &[Condition],
    user_cache: &UserCacheRow,
    game_stats: &HashMap<String, PlayerGameStatsRow>,
) -> bool {
    if conditions.is_empty() {
        return false;
    }
    conditions
        .iter()
        .all(|c| evaluate_single(c, user_cache, game_stats))
}

fn evaluate_single(
    condition: &Condition,
    uc: &UserCacheRow,
    game_stats: &HashMap<String, PlayerGameStatsRow>,
) -> bool {
    match &condition.field {
        ConditionField::AccountAgeDays => {
            let actual = uc
                .account_created
                .map(|c| (chrono::Utc::now() - c).num_days())
                .unwrap_or(0);
            let expected = condition.value.as_i64().unwrap_or(0);
            compare_int(actual, expected, &condition.operator, &condition.value_end)
        }
        ConditionField::HasVerifiedBadge => bool_eq(uc.has_verified_badge, &condition.value),
        ConditionField::FriendsCount => num_compare(uc.friends_count as i64, condition),
        ConditionField::FollowersCount => num_compare(uc.followers_count as i64, condition),
        ConditionField::FollowingCount => num_compare(uc.following_count as i64, condition),
        ConditionField::BadgesCount => num_compare(uc.badges_count as i64, condition),
        ConditionField::OwnsBadge => {
            let id = match condition.badge_id.as_deref() {
                Some(s) => s,
                None => return false,
            };
            let expected = condition.value.as_bool().unwrap_or(true);
            let owns = uc
                .badges
                .as_array()
                .is_some_and(|arr| arr.iter().any(|v| value_str_eq(v, id)));
            owns == expected
        }
        ConditionField::OwnsGamepass => {
            let id = match condition.gamepass_id.as_deref() {
                Some(s) => s,
                None => return false,
            };
            let expected = condition.value.as_bool().unwrap_or(true);
            let owns = uc
                .gamepasses
                .as_array()
                .is_some_and(|arr| arr.iter().any(|v| value_str_eq(v, id)));
            owns == expected
        }
        ConditionField::OwnsAsset => {
            // Asset ownership is not cached on user_cache (Roblox doesn't expose
            // the full asset inventory). Plugins that need this field rely on
            // the per-user `owns_item` check during refresh, which sets a flag
            // in `groups` JSONB key with prefix "asset:". For now, fall through
            // to false unless explicitly seeded (extension point).
            let id = match condition.asset_id.as_deref() {
                Some(s) => s,
                None => return false,
            };
            let expected = condition.value.as_bool().unwrap_or(true);
            let owns = uc.gamepasses.as_array().is_some_and(|arr| {
                arr.iter().any(|v| value_str_eq(v, id))
            });
            owns == expected
        }
        ConditionField::InGroup => {
            let gid = match condition.group_id.as_deref() {
                Some(s) => s,
                None => return false,
            };
            let expected = condition.value.as_bool().unwrap_or(true);
            let is_member = uc.groups.as_array().is_some_and(|arr| {
                arr.iter().any(|v| {
                    v.get("group_id")
                        .map(|gv| value_str_eq(gv, gid))
                        .unwrap_or(false)
                })
            });
            is_member == expected
        }
        ConditionField::GroupRoleRank => {
            let gid = match condition.group_id.as_deref() {
                Some(s) => s,
                None => return false,
            };
            let actual = uc
                .groups
                .as_array()
                .and_then(|arr| {
                    arr.iter().find_map(|v| {
                        if v.get("group_id").map(|gv| value_str_eq(gv, gid)).unwrap_or(false) {
                            v.get("role_rank").and_then(|r| r.as_i64())
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or(0);
            let expected = condition.value.as_i64().unwrap_or(0);
            compare_int(actual, expected, &condition.operator, &condition.value_end)
        }
        // Game-specific fields — all custom-keyed.
        ConditionField::CustomNumeric
        | ConditionField::CustomBoolean
        | ConditionField::CustomString => {
            let universe_id = match condition.universe_id.as_deref() {
                Some(s) => s,
                None => return false,
            };
            let row = match game_stats.get(universe_id) {
                Some(r) => r,
                None => return false,
            };
            evaluate_game(condition, row)
        }
    }
}

fn evaluate_game(condition: &Condition, row: &PlayerGameStatsRow) -> bool {
    match &condition.field {
        ConditionField::CustomNumeric => {
            let key = match condition.stat_key.as_deref() {
                Some(s) => s,
                None => return false,
            };
            let actual = row
                .custom
                .get(key)
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                .unwrap_or(0);
            num_compare(actual, condition)
        }
        ConditionField::CustomBoolean => {
            let key = match condition.stat_key.as_deref() {
                Some(s) => s,
                None => return false,
            };
            let actual = row.custom.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
            bool_eq(actual, &condition.value)
        }
        ConditionField::CustomString => {
            let key = match condition.stat_key.as_deref() {
                Some(s) => s,
                None => return false,
            };
            let expected = condition.value.as_str().unwrap_or("");
            let actual = row.custom.get(key).and_then(|v| v.as_str()).unwrap_or("");
            actual == expected
        }
        _ => false,
    }
}

fn num_compare(actual: i64, condition: &Condition) -> bool {
    let expected = condition.value.as_i64().unwrap_or(0);
    compare_int(actual, expected, &condition.operator, &condition.value_end)
}

fn compare_int(
    actual: i64,
    expected: i64,
    operator: &ConditionOperator,
    value_end: &Option<serde_json::Value>,
) -> bool {
    match operator {
        ConditionOperator::Eq => actual == expected,
        ConditionOperator::Gt => actual > expected,
        ConditionOperator::Gte => actual >= expected,
        ConditionOperator::Lt => actual < expected,
        ConditionOperator::Lte => actual <= expected,
        ConditionOperator::Between => {
            let end = value_end.as_ref().and_then(|v| v.as_i64()).unwrap_or(expected);
            actual >= expected && actual <= end
        }
    }
}

fn bool_eq(actual: bool, value: &serde_json::Value) -> bool {
    let expected = value.as_bool().unwrap_or(true);
    actual == expected
}

fn value_str_eq(v: &serde_json::Value, target: &str) -> bool {
    match v {
        serde_json::Value::String(s) => s == target,
        serde_json::Value::Number(n) => n.to_string() == target,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_uc() -> UserCacheRow {
        UserCacheRow {
            roblox_user_id: "12345".into(),
            account_created: Some(chrono::Utc::now() - chrono::Duration::days(1000)),
            has_verified_badge: false,
            friends_count: 50,
            followers_count: 100,
            following_count: 25,
            badges_count: 15,
            groups: json!([
                {"group_id": 7, "role_rank": 100, "role_name": "Member"},
                {"group_id": 42, "role_rank": 250, "role_name": "Admin"}
            ]),
            badges: json!(["b1", "b2"]),
            gamepasses: json!(["gp1"]),
        }
    }

    #[test]
    fn empty_grants_to_no_one() {
        assert!(!evaluate_conditions(&[], &sample_uc(), &HashMap::new()));
    }

    #[test]
    fn account_age_gte() {
        let cs = vec![Condition {
            field: ConditionField::AccountAgeDays,
            operator: ConditionOperator::Gte,
            value: json!(365),
            value_end: None,
            group_id: None,
            universe_id: None,
            badge_id: None,
            gamepass_id: None,
            asset_id: None,
            stat_key: None,
        }];
        assert!(evaluate_conditions(&cs, &sample_uc(), &HashMap::new()));
    }

    #[test]
    fn in_group_true() {
        let cs = vec![Condition {
            field: ConditionField::InGroup,
            operator: ConditionOperator::Eq,
            value: json!(true),
            value_end: None,
            group_id: Some("42".into()),
            universe_id: None,
            badge_id: None,
            gamepass_id: None,
            asset_id: None,
            stat_key: None,
        }];
        assert!(evaluate_conditions(&cs, &sample_uc(), &HashMap::new()));
    }

    #[test]
    fn group_rank_gte() {
        let cs = vec![Condition {
            field: ConditionField::GroupRoleRank,
            operator: ConditionOperator::Gte,
            value: json!(200),
            value_end: None,
            group_id: Some("42".into()),
            universe_id: None,
            badge_id: None,
            gamepass_id: None,
            asset_id: None,
            stat_key: None,
        }];
        assert!(evaluate_conditions(&cs, &sample_uc(), &HashMap::new()));
    }

    #[test]
    fn owns_badge_true() {
        let cs = vec![Condition {
            field: ConditionField::OwnsBadge,
            operator: ConditionOperator::Eq,
            value: json!(true),
            value_end: None,
            group_id: None,
            universe_id: None,
            badge_id: Some("b1".into()),
            gamepass_id: None,
            asset_id: None,
            stat_key: None,
        }];
        assert!(evaluate_conditions(&cs, &sample_uc(), &HashMap::new()));
    }

    #[test]
    fn custom_numeric_stat_gte_with_universe_data() {
        let mut gs = HashMap::new();
        gs.insert(
            "u1".to_string(),
            PlayerGameStatsRow {
                custom: json!({"level": 12, "vip": true, "score": 9000}),
            },
        );
        let cs = vec![Condition {
            field: ConditionField::CustomNumeric,
            operator: ConditionOperator::Gte,
            value: json!(10),
            value_end: None,
            group_id: None,
            universe_id: Some("u1".into()),
            badge_id: None,
            gamepass_id: None,
            asset_id: None,
            stat_key: Some("level".into()),
        }];
        assert!(evaluate_conditions(&cs, &sample_uc(), &gs));
    }

    #[test]
    fn custom_numeric_stat_gt() {
        let mut gs = HashMap::new();
        gs.insert(
            "u1".to_string(),
            PlayerGameStatsRow {
                custom: json!({"score": 9000}),
            },
        );
        let cs = vec![Condition {
            field: ConditionField::CustomNumeric,
            operator: ConditionOperator::Gt,
            value: json!(5000),
            value_end: None,
            group_id: None,
            universe_id: Some("u1".into()),
            badge_id: None,
            gamepass_id: None,
            asset_id: None,
            stat_key: Some("score".into()),
        }];
        assert!(evaluate_conditions(&cs, &sample_uc(), &gs));
    }

    #[test]
    fn missing_universe_data_fails() {
        let cs = vec![Condition {
            field: ConditionField::CustomNumeric,
            operator: ConditionOperator::Gte,
            value: json!(1),
            value_end: None,
            group_id: None,
            universe_id: Some("nonexistent".into()),
            badge_id: None,
            gamepass_id: None,
            asset_id: None,
            stat_key: Some("level".into()),
        }];
        assert!(!evaluate_conditions(&cs, &sample_uc(), &HashMap::new()));
    }
}
