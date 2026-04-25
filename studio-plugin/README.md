# Roblox-Player-Role Studio plugin

Source for the `.rbxm` we ship for one-click in-game integration. Built with [Rojo](https://rojo.space).

## Build

```bash
cd studio-plugin
rojo build --output dist/Roblox-Player-Role.rbxm
```

The resulting file can be:

- Sideloaded into Roblox Studio: **Plugins** tab → **Plugins Folder** → drop the `.rbxm` in → restart Studio.
- Inserted into an open place via **File → Open** then dragged into `ServerScriptService`.
- Published to the Creator Store at https://create.roblox.com so devs can install with one click.

## Configuration

After install, a `Configuration` instance named **RoleLogicConfig** appears under `ServerScriptService`. Set:

- `WebhookUrl` — the URL shown on `https://<your-host>/roblox-player-role/games/<universe-id>` (under "Webhook URL").
- `IngestSecret` — the secret shown immediately after registering the universe (rotate via the same page).

You can also hard-code these at the top of [src/Config.lua](src/Config.lua) and rebuild — the Configuration instance only takes effect when both values are non-empty.

## Layout

- [src/Config.lua](src/Config.lua) — `StatPaths` map (which `leaderstats` / attributes to upload), batch interval.
- [src/init.server.lua](src/init.server.lua) — `Players.PlayerAdded` snapshot + 60s batch loop posting to the webhook with `X-Ingest-Secret`.

## Custom stats

Anything in `StatPaths` whose `key` starts with `custom.` lands in the `custom` object on the server side. Your role conditions then reference it via the `stat_key` form field (Custom Numeric / Boolean / String).

Example:
```lua
{ key = "custom.guild_score", lookup = "attribute:GuildScore" },
```
Then in the dashboard: **Game → Custom numeric → stat_key=`guild_score`, >= 5000**.
