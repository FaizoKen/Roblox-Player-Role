-- Mutual-exclusion between push (Studio plugin / HttpService) and pull
-- (Open Cloud DataStore polling) modes per universe registration. Same
-- (universe_id, guild_id) row picks ONE — eliminates last-write-wins races
-- on player_game_stats when both sources targeted the same fields.

ALTER TABLE game_universes ADD COLUMN IF NOT EXISTS mode TEXT;

UPDATE game_universes
SET mode = CASE WHEN pull_enabled THEN 'pull' ELSE 'push' END
WHERE mode IS NULL;

ALTER TABLE game_universes ALTER COLUMN mode SET NOT NULL;
ALTER TABLE game_universes ALTER COLUMN mode SET DEFAULT 'push';

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'game_universes_mode_check'
    ) THEN
        ALTER TABLE game_universes
            ADD CONSTRAINT game_universes_mode_check
            CHECK (mode IN ('push', 'pull'));
    END IF;
END $$;

-- Enforce: push mode never has pull_enabled, pull mode never has push_enabled.
UPDATE game_universes SET pull_enabled = FALSE WHERE mode = 'push';
UPDATE game_universes SET push_enabled = FALSE WHERE mode = 'pull';
