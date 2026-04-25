-- Roblox user data cache (profile + global stats snapshot)
CREATE TABLE IF NOT EXISTS user_cache (
    roblox_user_id      TEXT PRIMARY KEY,
    username            TEXT,
    display_name        TEXT,
    description         TEXT,
    -- Account-level denormalized columns (for SQL-side WHERE filtering)
    account_created     TIMESTAMPTZ,
    has_verified_badge  BOOLEAN NOT NULL DEFAULT FALSE,
    friends_count       INTEGER NOT NULL DEFAULT 0,
    followers_count     INTEGER NOT NULL DEFAULT 0,
    following_count     INTEGER NOT NULL DEFAULT 0,
    badges_count        INTEGER NOT NULL DEFAULT 0,
    inventory_public    BOOLEAN NOT NULL DEFAULT TRUE,
    -- JSONB for set-membership conditions (groups, badges, gamepasses, assets)
    groups              JSONB NOT NULL DEFAULT '[]',     -- [{group_id, role_rank, role_name}]
    badges              JSONB NOT NULL DEFAULT '[]',     -- [badge_id, ...]
    gamepasses          JSONB NOT NULL DEFAULT '[]',     -- [gamepass_id, ...]
    -- Raw API responses for admin/debug
    profile_data        JSONB NOT NULL DEFAULT '{}',
    -- Refresh tracking
    fetched_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    next_fetch_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    fetch_failures      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_user_cache_next_fetch ON user_cache (next_fetch_at ASC);
CREATE INDEX IF NOT EXISTS idx_user_cache_friends ON user_cache (friends_count);
CREATE INDEX IF NOT EXISTS idx_user_cache_badges_count ON user_cache (badges_count);
CREATE INDEX IF NOT EXISTS idx_user_cache_groups_gin ON user_cache USING GIN (groups);
CREATE INDEX IF NOT EXISTS idx_user_cache_badges_gin ON user_cache USING GIN (badges);
CREATE INDEX IF NOT EXISTS idx_user_cache_gamepasses_gin ON user_cache USING GIN (gamepasses);
