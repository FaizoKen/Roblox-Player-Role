use serde::{Deserialize, Serialize};

/// Per-player stat blob accepted by `POST /ingest/{universe_id}/stats`
/// and produced by the Open Cloud DataStore poll worker.
///
/// All fields are optional — a partial update only overwrites the fields
/// that were sent. `custom` is merged shallowly (existing keys not in the
/// new payload are preserved).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PlayerStats {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playtime_minutes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wins: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub losses: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub achievements: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub custom: serde_json::Map<String, serde_json::Value>,
}

impl PlayerStats {
    pub fn is_empty(&self) -> bool {
        self.playtime_minutes.is_none()
            && self.level.is_none()
            && self.wins.is_none()
            && self.losses.is_none()
            && self.currency.is_none()
            && self.achievements.is_none()
            && self.custom.is_empty()
    }
}
