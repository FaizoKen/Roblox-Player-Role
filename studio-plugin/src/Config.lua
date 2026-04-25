--[[
    Config.lua  —  ModuleScript

    Returns the runtime configuration for the Roblox-Player-Role uploader.

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
    -- Custom stats land in the "custom" object on the server side.
    -- Your role conditions then reference them by `stat_key`.
    -- { key = "custom.score",  lookup = "attribute:Score" },
}

return Config
