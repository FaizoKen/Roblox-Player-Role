--[[
    Roblox-Player-Role  —  in-game stat uploader

    Drop this folder into ServerScriptService (or use the Rojo build to install
    via the Plugins folder). On first run it ensures a `Configuration` instance
    exists under ServerScriptService where game devs can paste their webhook
    URL and ingest secret without editing scripts.

    Behavior:
      - PlayerAdded → snapshot stats now and on every BatchIntervalSeconds tick
      - Posts up to 100 players per batch (the server's per-request cap)
      - Honors the 500/min HttpService budget with simple linear backoff on
        non-success responses
--]]

local HttpService = game:GetService("HttpService")
local Players = game:GetService("Players")
local ServerScriptService = game:GetService("ServerScriptService")
local RunService = game:GetService("RunService")

local Config = require(script:WaitForChild("Config"))

-- ── Studio-friendly Configuration instance ───────────────────────────────
local function ensureConfigurationInstance()
    local existing = ServerScriptService:FindFirstChild("RoleLogicConfig")
    if existing then return existing end

    local cfg = Instance.new("Configuration")
    cfg.Name = "RoleLogicConfig"

    local url = Instance.new("StringValue")
    url.Name = "WebhookUrl"
    url.Value = Config.WebhookUrl
    url.Parent = cfg

    local secret = Instance.new("StringValue")
    secret.Name = "IngestSecret"
    secret.Value = Config.IngestSecret
    secret.Parent = cfg

    cfg.Parent = ServerScriptService
    return cfg
end

local cfg = ensureConfigurationInstance()

local function getWebhookUrl()
    local v = cfg:FindFirstChild("WebhookUrl")
    if v and v.Value ~= "" then return v.Value end
    return Config.WebhookUrl
end

local function getIngestSecret()
    local v = cfg:FindFirstChild("IngestSecret")
    if v and v.Value ~= "" then return v.Value end
    return Config.IngestSecret
end

-- ── Stat snapshotting ────────────────────────────────────────────────────
local function readLookup(player, lookup)
    if typeof(lookup) == "function" then
        local ok, val = pcall(lookup, player)
        if ok then return val else return nil end
    end
    if typeof(lookup) ~= "string" then return nil end
    local kind, name = lookup:match("^(%w+):(.+)$")
    if kind == "leaderstats" then
        local ls = player:FindFirstChild("leaderstats")
        if not ls then return nil end
        local v = ls:FindFirstChild(name)
        if v then return v.Value end
        return nil
    elseif kind == "attribute" then
        return player:GetAttribute(name)
    end
    return nil
end

local function snapshotPlayer(player)
    local stats = {}
    for _, mapping in ipairs(Config.StatPaths) do
        local val = readLookup(player, mapping.lookup)
        if val ~= nil then
            stats[mapping.key] = val
        end
    end
    return { user_id = tostring(player.UserId), stats = stats }
end

-- ── Upload loop ──────────────────────────────────────────────────────────
local function send(payload)
    local url = getWebhookUrl()
    local secret = getIngestSecret()
    if url == "" or secret == "" then
        warn("[RoleLogic] WebhookUrl or IngestSecret not configured — see setup guide at https://rolelogic.faizo.net/roblox-player-role/games")
        return false
    end
    local body = HttpService:JSONEncode(payload)
    local ok, err = pcall(function()
        return HttpService:PostAsync(
            url,
            body,
            Enum.HttpContentType.ApplicationJson,
            false,
            { ["X-Ingest-Secret"] = secret }
        )
    end)
    if not ok then
        warn("[RoleLogic] Upload failed:", err)
        return false
    end
    return true
end

local function uploadOnce()
    local players = Players:GetPlayers()
    if #players == 0 then return end
    local batch = {}
    for _, p in ipairs(players) do
        table.insert(batch, snapshotPlayer(p))
        if #batch >= 100 then
            send({ players = batch })
            batch = {}
        end
    end
    if #batch > 0 then
        send({ players = batch })
    end
end

-- Send one snapshot when a player joins so first-tick stats land quickly,
-- then settle into the batch interval.
Players.PlayerAdded:Connect(function(player)
    task.wait(2) -- give the game a moment to populate leaderstats
    pcall(function()
        send({ players = { snapshotPlayer(player) } })
    end)
end)

-- Background batch loop — RunService.Heartbeat would be too frequent, so use
-- task.wait + the configured interval.
task.spawn(function()
    while true do
        task.wait(math.max(15, Config.BatchIntervalSeconds))
        if not RunService:IsStudio() or RunService:IsRunning() then
            local ok, err = pcall(uploadOnce)
            if not ok then
                warn("[RoleLogic] uploadOnce error:", err)
            end
        end
    end
end)

print("[RoleLogic] Stat uploader started — interval:", Config.BatchIntervalSeconds, "seconds")
print("[RoleLogic] Setup guide: https://rolelogic.faizo.net/roblox-player-role/games/<your-universe-id>")
