-- Per-(roblox_user_id, universe_id) snapshot of in-game stats.
-- Populated by /ingest webhook (push) and opencloud_poll_worker (pull).
CREATE TABLE IF NOT EXISTS player_game_stats (
    roblox_user_id      TEXT NOT NULL,
    universe_id         TEXT NOT NULL REFERENCES game_universes(universe_id) ON DELETE CASCADE,
    -- Common stats (denormalized for fast SQL filtering)
    playtime_minutes    INTEGER NOT NULL DEFAULT 0,
    level               INTEGER NOT NULL DEFAULT 0,
    wins                INTEGER NOT NULL DEFAULT 0,
    losses              INTEGER NOT NULL DEFAULT 0,
    currency            BIGINT NOT NULL DEFAULT 0,
    -- JSONB collections for ownership / set-membership conditions
    achievements        JSONB NOT NULL DEFAULT '[]',     -- ["first_blood", "speedrun", ...]
    custom              JSONB NOT NULL DEFAULT '{}',     -- arbitrary {key: number|bool|string}
    fetched_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (roblox_user_id, universe_id)
);
CREATE INDEX IF NOT EXISTS idx_pgs_universe ON player_game_stats (universe_id);
CREATE INDEX IF NOT EXISTS idx_pgs_universe_level ON player_game_stats (universe_id, level);
CREATE INDEX IF NOT EXISTS idx_pgs_universe_wins ON player_game_stats (universe_id, wins);
CREATE INDEX IF NOT EXISTS idx_pgs_universe_playtime ON player_game_stats (universe_id, playtime_minutes);
CREATE INDEX IF NOT EXISTS idx_pgs_achievements_gin ON player_game_stats USING GIN (achievements);
