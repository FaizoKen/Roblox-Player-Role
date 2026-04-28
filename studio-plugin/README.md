# Roblox-Player-Role Studio plugin

Source for the `.rbxm` we ship for one-click in-game integration. Built with [Rojo](https://rojo.space).

## Build

```bash
cd studio-plugin
rojo build --output dist/Roblox-Player-Role.rbxm
```

The resulting file can be:

- Sideloaded into Roblox Studio: **Plugins** tab → **Plugins Folder** → drop the `.rbxm` in → restart Studio.
- Inserted into an open place by right-clicking `ServerScriptService` in the Explorer → **Insert** → **Import Roblox Model** (older Studio: **Insert from File…**) and picking the `.rbxm`. Alternatively drag the `.rbxm` from your file explorer into the Studio viewport, then drag the resulting Script into `ServerScriptService`. **File → Open** does not accept `.rbxm` — that menu opens place files only.
- Published to the Creator Store at https://create.roblox.com so devs can install with one click.

## Configuration

**Recommended (simplest):** in Studio, expand the inserted `Roblox-Player-Role` Script → double-click child `Config` ModuleScript → set `WebhookUrl` and `IngestSecret` at the top → Ctrl+S.

- `WebhookUrl` — URL shown on `https://<your-host>/roblox-player-role/games/<universe-id>` (under "Webhook URL").
- `IngestSecret` — secret shown immediately after registering the universe (rotate via the same page).

**Advanced:** press F5, switch Explorer to **Server** view via the **Test** tab — the script auto-creates a `RoleLogicConfig` Configuration under `ServerScriptService` with `WebhookUrl`/`IngestSecret` `StringValue` children. Useful if you'd rather keep secrets out of the script source, but you must re-create the Configuration in Edit mode for values to persist outside Play.

## Layout

- [src/Config.lua](src/Config.lua) — `StatPaths` map (which `leaderstats` / attributes to upload), batch interval.
- [src/init.server.lua](src/init.server.lua) — `Players.PlayerAdded` snapshot + 60s batch loop posting to the webhook with `X-Ingest-Secret`.

## Stat keys

Every entry's `key` is a free-form name. The plugin sends `stats[key] = value` for every player and the server stores it under that exact name. The role-condition stat dropdown (**Game → Custom numeric / boolean / string**) lists the keys this universe has reported. Pick one, set the comparison, save.

Example:
```lua
{ key = "guild_score", lookup = "attribute:GuildScore" },
```
Then in the dashboard: **Game → Custom numeric → stat_key=`guild_score`, >= 5000**.

## Playtime tracking

The shipped plugin does **not** track playtime out of the box. Add a stat with whatever key you want — `timePlayed`, `playtime_minutes`, anything — and that key shows up in the dropdown.

**If your game already has a `Playtime` `leaderstats` entry (in minutes)** — append to `StatPaths`:
```lua
{ key = "timePlayed", lookup = "leaderstats:Playtime" },
```

**If you have no playtime tracking** — right-click `ServerScriptService` → **Insert Object** → `Script` (rename to `PlaytimeTracker`) and paste:
```lua
local Players = game:GetService("Players")
Players.PlayerAdded:Connect(function(p)
    p:SetAttribute("PlaytimeMinutes", 0)
    task.spawn(function()
        while p.Parent do
            task.wait(60)
            p:SetAttribute("PlaytimeMinutes", (p:GetAttribute("PlaytimeMinutes") or 0) + 1)
        end
    end)
end)
```
Then append to `StatPaths`:
```lua
{ key = "timePlayed", lookup = "attribute:PlaytimeMinutes" },
```

Note: this attribute resets on rejoin (no persistence). For permanent cumulative playtime, persist via `DataStoreService` (read on `PlayerAdded`, save on `PlayerRemoving` and `BindToClose`).
