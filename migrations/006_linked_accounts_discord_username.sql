-- Cache the Discord display name on the linked_accounts row so the /players
-- admin page can show "which Discord user this Roblox account belongs to".
-- Captured from the rl_session cookie at verify-callback time. Nullable for
-- legacy rows linked before this column existed; refreshed on each re-link.

ALTER TABLE linked_accounts ADD COLUMN IF NOT EXISTS discord_username TEXT;
