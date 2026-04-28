-- Per-(roblox_user_id, universe_id) snapshot of in-game stats.
-- Populated by /ingest webhook (push) and opencloud_poll_worker (pull).
-- All stats live as keys in the `custom` JSONB blob. Role conditions reference
-- those keys directly via (custom->>'<key>') — there are no privileged stat
-- columns.
CREATE TABLE IF NOT EXISTS player_game_stats (
    roblox_user_id      TEXT NOT NULL,
    universe_id         TEXT NOT NULL REFERENCES game_universes(universe_id) ON DELETE CASCADE,
    custom              JSONB NOT NULL DEFAULT '{}',
    fetched_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (roblox_user_id, universe_id)
);
CREATE INDEX IF NOT EXISTS idx_pgs_universe ON player_game_stats (universe_id);
