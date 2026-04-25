use std::sync::Arc;

use tokio::sync::mpsc;

use crate::services::sync::{self, PlayerSyncEvent};
use crate::AppState;

pub async fn run(mut rx: mpsc::Receiver<PlayerSyncEvent>, state: Arc<AppState>) {
    tracing::info!("Player sync worker started");
    while let Some(event) = rx.recv().await {
        let result = match &event {
            PlayerSyncEvent::PlayerUpdated { discord_id }
            | PlayerSyncEvent::AccountLinked { discord_id } => {
                sync::sync_for_player(discord_id, &state).await
            }
            PlayerSyncEvent::AccountUnlinked { discord_id } => {
                sync::remove_all_assignments(discord_id, &state).await
            }
            PlayerSyncEvent::GameStatsUpdated { roblox_user_id } => {
                sync::fan_out_game_stats_update(roblox_user_id, &state).await
            }
        };
        if let Err(e) = result {
            tracing::error!(event = ?event, "Player sync failed: {e}");
        }
    }
    tracing::warn!("Player sync worker channel closed");
}
