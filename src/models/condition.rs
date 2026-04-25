use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ConditionField {
    // Account-level (no extra ID required)
    AccountAgeDays,
    HasVerifiedBadge,
    FriendsCount,
    FollowersCount,
    FollowingCount,
    BadgesCount,
    // Account ownership (require an asset/badge/gamepass id in the dedicated field)
    OwnsBadge,
    OwnsGamepass,
    OwnsAsset,
    // Group (requires group_id; GroupRoleRank also uses operator + value)
    InGroup,
    GroupRoleRank,
    // Per-game (require universe_id + the matching field)
    GamePlaytimeMinutes,
    GameLevel,
    GameWins,
    GameLosses,
    GameCurrency,
    HasGameAchievement,
    CustomNumeric,
    CustomBoolean,
    CustomString,
}

impl ConditionField {
    pub fn is_boolean(&self) -> bool {
        matches!(
            self,
            Self::HasVerifiedBadge
                | Self::OwnsBadge
                | Self::OwnsGamepass
                | Self::OwnsAsset
                | Self::InGroup
                | Self::CustomBoolean
        )
    }

    pub fn is_string_exact(&self) -> bool {
        matches!(self, Self::HasGameAchievement | Self::CustomString)
    }

    pub fn requires_universe(&self) -> bool {
        matches!(
            self,
            Self::GamePlaytimeMinutes
                | Self::GameLevel
                | Self::GameWins
                | Self::GameLosses
                | Self::GameCurrency
                | Self::HasGameAchievement
                | Self::CustomNumeric
                | Self::CustomBoolean
                | Self::CustomString
        )
    }

    pub fn requires_group(&self) -> bool {
        matches!(self, Self::InGroup | Self::GroupRoleRank)
    }

    pub fn requires_badge(&self) -> bool {
        matches!(self, Self::OwnsBadge)
    }

    pub fn requires_gamepass(&self) -> bool {
        matches!(self, Self::OwnsGamepass)
    }

    pub fn requires_asset(&self) -> bool {
        matches!(self, Self::OwnsAsset)
    }

    pub fn requires_stat_key(&self) -> bool {
        matches!(self, Self::CustomNumeric | Self::CustomBoolean | Self::CustomString)
    }

    pub fn category(&self) -> ConditionCategory {
        if self.requires_universe() {
            ConditionCategory::Game
        } else if self.requires_group() {
            ConditionCategory::Group
        } else {
            ConditionCategory::Account
        }
    }

    pub fn json_key(&self) -> &'static str {
        match self {
            Self::AccountAgeDays => "accountAgeDays",
            Self::HasVerifiedBadge => "hasVerifiedBadge",
            Self::FriendsCount => "friendsCount",
            Self::FollowersCount => "followersCount",
            Self::FollowingCount => "followingCount",
            Self::BadgesCount => "badgesCount",
            Self::OwnsBadge => "ownsBadge",
            Self::OwnsGamepass => "ownsGamepass",
            Self::OwnsAsset => "ownsAsset",
            Self::InGroup => "inGroup",
            Self::GroupRoleRank => "groupRoleRank",
            Self::GamePlaytimeMinutes => "gamePlaytimeMinutes",
            Self::GameLevel => "gameLevel",
            Self::GameWins => "gameWins",
            Self::GameLosses => "gameLosses",
            Self::GameCurrency => "gameCurrency",
            Self::HasGameAchievement => "hasGameAchievement",
            Self::CustomNumeric => "customNumeric",
            Self::CustomBoolean => "customBoolean",
            Self::CustomString => "customString",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "accountAgeDays" => Some(Self::AccountAgeDays),
            "hasVerifiedBadge" => Some(Self::HasVerifiedBadge),
            "friendsCount" => Some(Self::FriendsCount),
            "followersCount" => Some(Self::FollowersCount),
            "followingCount" => Some(Self::FollowingCount),
            "badgesCount" => Some(Self::BadgesCount),
            "ownsBadge" => Some(Self::OwnsBadge),
            "ownsGamepass" => Some(Self::OwnsGamepass),
            "ownsAsset" => Some(Self::OwnsAsset),
            "inGroup" => Some(Self::InGroup),
            "groupRoleRank" => Some(Self::GroupRoleRank),
            "gamePlaytimeMinutes" => Some(Self::GamePlaytimeMinutes),
            "gameLevel" => Some(Self::GameLevel),
            "gameWins" => Some(Self::GameWins),
            "gameLosses" => Some(Self::GameLosses),
            "gameCurrency" => Some(Self::GameCurrency),
            "hasGameAchievement" => Some(Self::HasGameAchievement),
            "customNumeric" => Some(Self::CustomNumeric),
            "customBoolean" => Some(Self::CustomBoolean),
            "customString" => Some(Self::CustomString),
            _ => None,
        }
    }

    /// PostgreSQL column on the `user_cache uc` alias for fields that are denormalized.
    /// Returns None for fields that need JSONB lookups or game-specific tables.
    pub fn user_cache_column(&self) -> Option<&'static str> {
        match self {
            Self::HasVerifiedBadge => Some("uc.has_verified_badge"),
            Self::FriendsCount => Some("uc.friends_count"),
            Self::FollowersCount => Some("uc.followers_count"),
            Self::FollowingCount => Some("uc.following_count"),
            Self::BadgesCount => Some("uc.badges_count"),
            _ => None,
        }
    }

    /// PostgreSQL column on the `pgs_<universe> pgs` alias for per-game numeric stats.
    pub fn player_game_stats_column(&self) -> Option<&'static str> {
        match self {
            Self::GamePlaytimeMinutes => Some("pgs.playtime_minutes"),
            Self::GameLevel => Some("pgs.level"),
            Self::GameWins => Some("pgs.wins"),
            Self::GameLosses => Some("pgs.losses"),
            Self::GameCurrency => Some("pgs.currency"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionCategory {
    Account,
    Group,
    Game,
}

impl ConditionCategory {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Group => "group",
            Self::Game => "game",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ConditionOperator {
    Eq,
    Gt,
    Gte,
    Lt,
    Lte,
    Between,
}

impl ConditionOperator {
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "eq" => Some(Self::Eq),
            "gt" => Some(Self::Gt),
            "gte" => Some(Self::Gte),
            "lt" => Some(Self::Lt),
            "lte" => Some(Self::Lte),
            "between" => Some(Self::Between),
            _ => None,
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Self::Eq => "eq",
            Self::Gt => "gt",
            Self::Gte => "gte",
            Self::Lt => "lt",
            Self::Lte => "lte",
            Self::Between => "between",
        }
    }

    pub fn sql_operator(&self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::Gt => ">",
            Self::Gte => ">=",
            Self::Lt => "<",
            Self::Lte => "<=",
            Self::Between => "BETWEEN",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    pub field: ConditionField,
    pub operator: ConditionOperator,
    pub value: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_end: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub universe_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub badge_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gamepass_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stat_key: Option<String>,
}
