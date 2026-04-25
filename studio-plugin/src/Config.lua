--[[
    Config.lua  —  ModuleScript

    Returns the runtime configuration for the Roblox-Player-Role uploader.

    Full setup guide (Steps 1–6, including playtime tracking) lives on the
    admin page where you registered your universe:
        https://rolelogic.faizo.net/roblox-player-role/games/<your-universe-id>

    Set BOTH WebhookUrl and IngestSecret on the Configuration instance that
    this plugin creates under ServerScriptService at first run, OR override
    them at the top of this file before publishing your game.

    StatPaths is a list of {key, lookup} pairs telling the uploader where to
    read each stat from on a Player. Edit this to match your game's leaderstats
    layout. Anything you don't configure is omitted from the upload (and the
    plugin server-side leaves the previous value untouched).

    "lookup" can be:
      - "leaderstats:Name"      → reads player.leaderstats.<Name>.Value
      - "attribute:Name"        → reads player:GetAttribute("Name")
      - function(player) ... end → arbitrary getter you write
--]]

local Config = {}

Config.WebhookUrl = ""    -- e.g. "https://example.com/roblox-player-role/ingest/123456/stats"
Config.IngestSecret = ""  -- copy from the /games admin page; rotate via the same page
Config.BatchIntervalSeconds = 60

Config.StatPaths = {
    { key = "level",            lookup = "leaderstats:Level" },
    { key = "wins",             lookup = "leaderstats:Wins" },
    { key = "losses",           lookup = "leaderstats:Losses" },
    { key = "currency",         lookup = "leaderstats:Coins" },
    -- Playtime is not tracked by Roblox out of the box. To enable the
    -- "Total in-game playtime (minutes)" role condition, wire a tracker
    -- (see studio-plugin/README.md) and uncomment one of:
    -- { key = "playtime_minutes", lookup = "leaderstats:Playtime" },
    -- { key = "playtime_minutes", lookup = "attribute:PlaytimeMinutes" },
    -- Achievements: a list of string keys the player has earned. Used by the
    -- "Has a specific in-game achievement" role condition (the dashboard
    -- "Value" field is matched against entries in this list, e.g. "first_blood").
    -- Roblox has no built-in achievement system — return whatever flags your
    -- game tracks (attributes, badges, DataStore-loaded set, etc.) as an array.
    -- Example using attributes:
    -- { key = "achievements", lookup = function(player)
    --     local earned = {}
    --     for _, name in ipairs({ "first_blood", "speedrun", "boss_killed" }) do
    --         if player:GetAttribute("ach_" .. name) then
    --             table.insert(earned, name)
    --         end
    --     end
    --     return earned
    -- end },
    -- Custom stats land in the "custom" object on the server side.
    -- Your role conditions then reference them by `stat_key`.
    -- { key = "custom.score",  lookup = "attribute:Score" },
}

return Config
