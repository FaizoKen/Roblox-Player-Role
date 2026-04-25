-- Per-Roblox-game (universe) integration registry. Created by game devs via /games admin UI.
CREATE TABLE IF NOT EXISTS game_universes (
    universe_id                     TEXT PRIMARY KEY,
    display_name                    TEXT NOT NULL,
    owner_discord_id                TEXT NOT NULL,
    -- Push (HttpService → /ingest/{universe_id}/stats)
    ingest_secret                   TEXT NOT NULL,
    push_enabled                    BOOLEAN NOT NULL DEFAULT TRUE,
    last_push_at                    TIMESTAMPTZ,
    -- Pull (Open Cloud DataStore polling)
    open_cloud_api_key_encrypted    TEXT,
    datastore_name                  TEXT,
    stat_field_map                  JSONB NOT NULL DEFAULT '{}',
    poll_interval_seconds           INTEGER NOT NULL DEFAULT 600,
    pull_enabled                    BOOLEAN NOT NULL DEFAULT FALSE,
    last_pull_at                    TIMESTAMPTZ,
    -- Recent activity log (last 50 ingest batches as JSONB array, FIFO)
    activity_log                    JSONB NOT NULL DEFAULT '[]',
    created_at                      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_game_universes_owner ON game_universes (owner_discord_id);
CREATE INDEX IF NOT EXISTS idx_game_universes_pull_due
    ON game_universes (last_pull_at)
    WHERE pull_enabled = TRUE;
