-- Per-universe DataStore entry-key template for pull mode.
--
-- Most games key their PlayerData DataStore by something like
-- "Player_<userId>" rather than the bare userId. The poll worker substitutes
-- the literal substring "{user_id}" in this template with the linked Roblox
-- user_id when reading each entry.
--
-- Default "{user_id}" preserves the previous behavior for any existing
-- registrations that happened to use bare-userId keys.

ALTER TABLE game_universes
    ADD COLUMN IF NOT EXISTS entry_key_template TEXT NOT NULL DEFAULT '{user_id}';
