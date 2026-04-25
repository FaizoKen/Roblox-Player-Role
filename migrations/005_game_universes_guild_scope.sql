-- Scope each universe registration to a single Discord guild so condition
-- authors cannot reference another guild's registered game data.
--
-- Existing rows are left with guild_id NULL — they no longer satisfy the
-- partial unique index, so the same universe_id can be re-registered under a
-- guild. Validation in post_config rejects conditions whose universe is not
-- registered for the saving guild, so legacy NULL rows are effectively orphaned
-- until the owner re-registers via the new guild-scoped /games UI.

ALTER TABLE game_universes ADD COLUMN IF NOT EXISTS guild_id TEXT;

-- Drop the old PK on universe_id alone; replace with a partial unique index on
-- the (universe_id, guild_id) pair for non-NULL guilds. NULL legacy rows are
-- ignored by the index but still exist physically (admin can DELETE via UI).
ALTER TABLE game_universes DROP CONSTRAINT IF EXISTS game_universes_pkey;
CREATE UNIQUE INDEX IF NOT EXISTS game_universes_uid_gid_uniq
    ON game_universes (universe_id, guild_id)
    WHERE guild_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_game_universes_guild ON game_universes (guild_id);

-- player_game_stats.universe_id had a FK to game_universes(universe_id), which
-- relied on universe_id being unique. Drop the FK; integrity is now enforced
-- at the application layer (ingest writes only when (universe_id, secret) match
-- a registered row). Cascade-on-delete is replaced by an explicit DELETE in
-- the delete_universe handler.
ALTER TABLE player_game_stats DROP CONSTRAINT IF EXISTS player_game_stats_universe_id_fkey;
