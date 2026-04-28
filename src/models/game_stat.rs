use serde::{Deserialize, Serialize};

/// Per-player stat blob accepted by `POST /ingest/{universe_id}/stats`
/// and produced by the Open Cloud DataStore poll worker.
///
/// Every key the Studio plugin (or pull worker) reports lands in `custom`
/// under that exact name. Role conditions reference custom keys directly.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct PlayerStats(pub serde_json::Map<String, serde_json::Value>);

impl PlayerStats {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
