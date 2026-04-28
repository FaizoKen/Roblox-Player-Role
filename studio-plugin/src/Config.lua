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
      
    After finishing setup, dont forget to allow HTTP & publish:
   		File → Experience Settings → Security → Allow HTTP Requests = ON.
		File → Publish to Roblox.
--]]

local Config = {}

Config.WebhookUrl = ""    -- e.g. "https://plugin-rolelogic.faizo.net/roblox-player-role/ingest/123456/stats"
Config.IngestSecret = ""  -- copy from the /games admin page; rotate via the same page
Config.BatchIntervalSeconds = 60

Config.StatPaths = {
    { key = "level",            lookup = "leaderstats:Level" },
    { key = "wins",             lookup = "leaderstats:Wins" },
    { key = "losses",           lookup = "leaderstats:Losses" },
    { key = "currency",         lookup = "leaderstats:Coins" },
	-- you can add more stats here — every key is a free-form name that shows
	-- up in the role-condition stat dropdown verbatim. Examples:
	-- { key = "timePlayed",       lookup = "leaderstats:TimePlayed" },
	-- { key = "kittens",          lookup = "leaderstats:Kittens" },
	-- { key = "isVIP",            lookup = function(player) return player:GetAttribute("VIP") end },
	-- { key = "currentZone",      lookup = "attribute:CurrentZone" },
	-- { key = "equippedPetId",    lookup = function(player) return player:GetAttribute("EquippedPetId") end },
}

return Config
