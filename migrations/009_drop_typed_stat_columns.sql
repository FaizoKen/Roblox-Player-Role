-- Collapse player_game_stats to a single `custom` JSONB blob.
-- All stats — including playtime, level, wins, losses, currency, achievements —
-- are now keyed inside `custom` and looked up by name. Role conditions only
-- ever reference `(custom->>'<key>')`; there is no privileged stat namespace.
DROP INDEX IF EXISTS idx_pgs_universe_level;
DROP INDEX IF EXISTS idx_pgs_universe_wins;
DROP INDEX IF EXISTS idx_pgs_universe_playtime;
DROP INDEX IF EXISTS idx_pgs_achievements_gin;

ALTER TABLE player_game_stats DROP COLUMN IF EXISTS playtime_minutes;
ALTER TABLE player_game_stats DROP COLUMN IF EXISTS level;
ALTER TABLE player_game_stats DROP COLUMN IF EXISTS wins;
ALTER TABLE player_game_stats DROP COLUMN IF EXISTS losses;
ALTER TABLE player_game_stats DROP COLUMN IF EXISTS currency;
ALTER TABLE player_game_stats DROP COLUMN IF EXISTS achievements;
