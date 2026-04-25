# Roblox Player Role

A [RoleLogic](https://rolelogic.faizo.net) plugin that grants Discord roles based on a member's Roblox account or in-game progression.

## What it does

Members link their Roblox account once. The plugin then assigns Discord roles automatically based on a single configurable condition per role link. Conditions span three categories:

- **Account** — account age, Premium subscription, verified badge, friend / follower / following / badge counts, ownership of a specific badge / gamepass / asset.
- **Group** — membership in a Roblox group, or a minimum role rank within it.
- **Game** — per-game stats (playtime, level, wins, losses, currency, achievements, custom numeric/boolean/string fields) for any Roblox game whose owner has registered an integration on the `/games` page.

Per-game stats can reach the plugin in two ways:

1. **Push** — the game's server scripts call `POST /ingest/{universe_id}/stats` with an `X-Ingest-Secret` header. The shipped Studio plugin under [studio-plugin/](./studio-plugin/) does this in 60s batches.
2. **Pull** — the plugin polls Roblox Open Cloud DataStore v2 for every linked user. The game owner pastes an Open Cloud API key + DataStore name + JSON-path → field map on the `/games` admin page; no in-game code change required.

Bulk syncs use RoleLogic's chunked-upload API and scale to the documented 30M-per-role ceiling.

## Endpoints

All endpoints are nested under `/roblox-player-role`.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `POST` | `/register` | `Token {api_token}` | RoleLogic registers a guild/role pair |
| `GET` | `/config` | `Token {api_token}` | Returns the form schema (with current values) |
| `POST` | `/config` | `Token {api_token}` | Saves a new condition; triggers a debounced bulk re-sync |
| `DELETE` | `/config` | `Token {api_token}` | Deletes the role link |
| `GET` | `/verify` | none | Linking page (Discord → Roblox OAuth) |
| `GET` | `/verify/login` | none | Redirects to the Auth Gateway for Discord sign-in |
| `GET` | `/verify/roblox` | `rl_session` cookie | Begins the Roblox OAuth (PKCE) flow |
| `GET` | `/verify/callback` | `rl_session` cookie | Roblox OAuth callback |
| `POST` | `/verify/unlink` | `rl_session` cookie | Drops the Roblox link, clears all roles |
| `GET` | `/players/{guild_id}` | `rl_session` cookie | Verified-player list for a guild |
| `GET` | `/games` | `rl_session` cookie | Game-creator admin dashboard |
| `POST` | `/games` | `rl_session` cookie | Register a Roblox universe (returns `ingest_secret` once) |
| `POST` | `/games/{universe_id}/regenerate-secret` | `rl_session` cookie | Rotate the ingest secret |
| `POST` | `/games/{universe_id}/open-cloud` | `rl_session` cookie | Save / disable Open Cloud DataStore polling |
| `POST` | `/games/{universe_id}/delete` | `rl_session` cookie | Delete the universe + cached stats |
| `POST` | `/ingest/{universe_id}/stats` | `X-Ingest-Secret` header | Push per-player stats (max 100 players, 60 req/min/universe) |
| `GET` | `/health` | none | Liveness + dependency check |

## Setup

1. Register a Roblox OAuth app at [create.roblox.com → Credentials → API Keys](https://create.roblox.com/dashboard/credentials). Set the redirect URI to `{BASE_URL}/verify/callback`. Request scopes `openid profile`. (`premium`, `verification` are optional but populate the matching condition fields.)
2. Copy `.env.example` → `.env`, fill it in. Generate `TOKEN_ENCRYPTION_KEY` with `openssl rand -hex 32`.
3. Install [Postgres 12+](https://www.postgresql.org/) (or use the bundled `compose.yml`).
4. `docker compose up -d --build` (or `cargo run --release`).

The Auth Gateway service must be reachable at `AUTH_GATEWAY_URL` and share the same `SESSION_SECRET` and `INTERNAL_API_KEY` as configured here.

## Game integration (for Roblox creators)

Once a server admin has set up at least one role link with a `game` condition pointing at your universe, register the universe at `{BASE_URL}/games`. From there you can:

1. **Install the shipped Studio plugin** (`studio-plugin/dist/Roblox-Player-Role.rbxm` after `rojo build`). It batches every 60s and POSTs stats from `leaderstats` to the webhook. Paste your webhook URL + secret into the Configuration instance the plugin creates.
2. **Or** copy the Lua snippet shown on the admin page into a `ServerScriptService` script.
3. **Or** paste an Open Cloud API key + DataStore name + JSON field mapping; we'll poll it.

Stat payload shape (push):

```json
{
  "players": [
    {
      "user_id": "12345",
      "stats": {
        "playtime_minutes": 120,
        "level": 7,
        "wins": 3,
        "losses": 1,
        "currency": 5000,
        "achievements": ["first_blood", "speedrun"],
        "custom": { "guild_score": 42, "is_vip": true }
      }
    }
  ]
}
```

Each field is optional; partial updates only overwrite what you send. `custom` is shallow-merged.

## Scale

- Per-user incremental updates (link / unlink / refresh / single-game ingest) go through the per-user `add_user` / `remove_user` path with `for_each_concurrent(10)`.
- Bulk re-syncs (after a config change) build a single SQL `SELECT discord_id WHERE …` against denormalized columns + JSONB GIN indices, then push the qualifying list via `RoleLogicClient::replace_users_scalable`. That client transparently switches to the chunked `start_upload` → `append_chunk` (100k per chunk) → `commit_upload` flow above the single-PUT limit, atomic on commit. Pre-flights against the documented 30M-per-role ceiling.
- Channels are bounded (`PlayerSyncEvent` 4096, `ConfigSyncEvent` 256). Producers use `try_send` to apply backpressure rather than block.
- Config events are debounced 5s before triggering a rebuild, so rapid edits coalesce.

## Tests

`cargo test` runs unit tests for `condition_eval` (every supported field) and `schema::parse_config`.

## Layout

```
src/
├── main.rs                 — Router, AppState, channel + worker wiring
├── config.rs               — Env loading
├── db.rs                   — PgPool, migrations
├── error.rs                — AppError + IntoResponse
├── schema.rs               — /config form schema + parse_config
├── models/                 — Condition, ConditionField, PlayerStats
├── routes/                 — RoleLogic contract, verification, players, games, ingest, health
├── services/               — roblox_api, roblox_oauth, roblox_open_cloud, rolelogic, condition_eval, sync, auth_gateway, session, crypto
└── tasks/                  — player_sync_worker, config_sync_worker, refresh_worker, opencloud_poll_worker
migrations/                  — 001 base / 002 user_cache / 003 game_universes / 004 player_game_stats
studio-plugin/               — Rojo source for the shipped .rbxm
```

See [.claude/BLUEPRINT.md](../.claude/BLUEPRINT.md) for cross-plugin conventions.
