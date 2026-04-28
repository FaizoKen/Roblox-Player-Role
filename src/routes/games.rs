//! Game-creator admin UI under `/games`. Cookie-authed via Auth Gateway session.
//!
//! Each universe registration is scoped to a single Discord guild — only that
//! guild's role conditions can reference its in-game stats. Owners who want to
//! grant roles in multiple guilds register the same universe under each guild.
//!
//! Endpoints:
//!   GET  /games                                — landing page: pick a guild
//!   GET  /games/data                           — JSON list of caller's guilds (for picker)
//!   GET  /games/{guild_id}                     — HTML dashboard for that guild's universes
//!   GET  /games/{guild_id}/data                — JSON list of universes for the guild
//!   POST /games/{guild_id}                     — register a new universe under this guild
//!   POST /games/{guild_id}/{universe_id}/regenerate-secret  — rotate ingest_secret
//!   POST /games/{guild_id}/{universe_id}/open-cloud         — set/clear Open Cloud key
//!   POST /games/{guild_id}/{universe_id}/delete             — delete the universe

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use axum_extra::extract::CookieJar;
use rand::RngCore;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::services::roblox_open_cloud::OpenCloudClient;
use crate::services::session;
use crate::AppState;

const SESSION_COOKIE: &str = "rl_session";

fn get_session(jar: &CookieJar, secret: &str) -> Result<(String, String), AppError> {
    let cookie = jar.get(SESSION_COOKIE).ok_or(AppError::Unauthorized)?;
    session::verify_session(cookie.value(), secret).ok_or(AppError::Unauthorized)
}

fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Forward the caller's `rl_session` cookie to the Auth Gateway and parse the
/// JSON response. Returns Unauthorized on 401, propagates other errors.
async fn auth_gateway_get(
    state: &Arc<AppState>,
    path_and_query: &str,
    session_cookie_value: &str,
) -> Result<Value, AppError> {
    let url = format!("{}{path_and_query}", state.config.auth_gateway_url);
    let outgoing = axum_extra::extract::cookie::Cookie::build((
        "rl_session",
        session_cookie_value.to_string(),
    ))
    .build();
    let cookie_header = outgoing.encoded().to_string();

    let resp = state
        .http
        .get(&url)
        .header(axum::http::header::COOKIE, cookie_header)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Auth Gateway unreachable: {e}")))?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(AppError::Unauthorized);
    }
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!(
            "Auth Gateway returned {status}: {body_text}"
        )));
    }
    resp.json::<Value>()
        .await
        .map_err(|e| AppError::Internal(format!("Auth Gateway parse error: {e}")))
}

/// Fetch (is_member, is_manager) for the caller in the given guild.
async fn fetch_guild_permission(
    state: &Arc<AppState>,
    guild_id: &str,
    cookie: &str,
) -> Result<(bool, bool), AppError> {
    let path = format!("/auth/guild_permission?guild_id={}", urlencoding::encode(guild_id));
    let body = auth_gateway_get(state, &path, cookie).await?;
    Ok((
        body.get("is_member").and_then(|v| v.as_bool()).unwrap_or(false),
        body.get("is_manager").and_then(|v| v.as_bool()).unwrap_or(false),
    ))
}

/// Require the caller to be a manager of `guild_id`. Returns the caller's
/// `discord_id` on success.
async fn require_manager(
    state: &Arc<AppState>,
    jar: &CookieJar,
    guild_id: &str,
) -> Result<String, AppError> {
    let (discord_id, _) = get_session(jar, &state.config.session_secret)?;
    let cookie = jar.get(SESSION_COOKIE).map(|c| c.value().to_string()).unwrap_or_default();
    let (is_member, is_manager) = fetch_guild_permission(state, guild_id, &cookie).await?;
    if !is_member {
        return Err(AppError::Forbidden(
            "You must be a member of this Discord server.".into(),
        ));
    }
    if !is_manager {
        return Err(AppError::Forbidden(
            "You must have Manage Server permission to register games for this Discord server."
                .into(),
        ));
    }
    Ok(discord_id)
}

pub fn render_landing_page(base_url: &str) -> String {
    format!(
        r##"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Roblox Player Roles - Pick a Server</title>
    <link rel="icon" href="{base_url}/favicon.ico" type="image/x-icon">
    <meta name="theme-color" content="#232527">
    <style>
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{ font-family: system-ui, -apple-system, sans-serif; max-width: 720px; margin: 0 auto; padding: 32px 20px; background: #232527; color: #ebedf0; min-height: 100vh; }}
        h1 {{ color: #00a2ff; font-size: 24px; }}
        p {{ color: #b8bcc1; font-size: 14px; line-height: 1.6; margin: 8px 0; }}
        a {{ color: #00a2ff; }}
        .card {{ background: #2e3133; padding: 22px; border-radius: 10px; margin: 12px 0; border: 1px solid #3d4144; }}
        .guild {{ display: flex; align-items: center; justify-content: space-between; padding: 10px 14px; border-radius: 6px; border: 1px solid #3d4144; background: #232527; margin: 6px 0; }}
        .guild-name {{ color: #ebedf0; font-weight: 500; }}
        .guild-meta {{ color: #8a9099; font-size: 12px; margin-top: 2px; }}
        .btn {{ display: inline-block; padding: 8px 18px; background: #00a2ff; color: #fff; text-decoration: none; border-radius: 6px; font-size: 13px; font-weight: 500; border: none; cursor: pointer; font-family: inherit; }}
        .btn:hover {{ background: #0086d3; }}
        .btn-disabled {{ background: #3d4144; color: #8a9099; cursor: not-allowed; }}
        .login-btn {{ display: inline-block; padding: 10px 22px; border-radius: 6px; background: #5865f2; color: #fff; text-decoration: none; font-weight: 600; }}
        .hidden {{ display: none; }}
        .msg {{ padding: 10px 14px; border-radius: 6px; font-size: 13px; position: fixed; top: 16px; left: 50%; transform: translateX(-50%); z-index: 1000; max-width: calc(100% - 32px); box-shadow: 0 6px 24px rgba(0,0,0,0.45); }}
        .msg-error {{ background: #1c0a0a; color: #fca5a5; border: 1px solid #7f1d1d; }}
    </style>
</head>
<body>
    <h1>Game Integrations</h1>
    <p>Pick the Discord server you'd like to register a Roblox game for. Each registration is private to one server — only that server's role conditions can reference its in-game stats.</p>

    <div id="loading" class="card"><p>Loading your servers...</p></div>
    <div id="error" class="hidden msg msg-error"></div>

    <div id="login-prompt" class="card hidden" style="text-align:center;">
        <p>You're not signed in.</p>
        <p style="margin:14px 0;"><a id="login-link" class="login-btn" href="#">Login with Discord</a></p>
    </div>

    <div id="content" class="hidden">
        <div class="card">
            <div id="guilds"></div>
        </div>
        <p style="font-size:12px; color:#8a9099;">Only servers where you have <strong>Manage Server</strong> permission can register games.</p>
    </div>

    <script>
    const API = '{base_url}';
    (function () {{
        const returnTo = window.location.pathname;
        document.getElementById('login-link').href = '/auth/login?return_to=' + encodeURIComponent(returnTo);
    }})();
    function esc(s) {{ const d = document.createElement('div'); d.textContent = s; return d.innerHTML; }}
    async function load() {{
        try {{
            const res = await fetch(API + '/games/data', {{ credentials: 'include' }});
            if (res.status === 401) {{
                document.getElementById('loading').classList.add('hidden');
                document.getElementById('login-prompt').classList.remove('hidden');
                return;
            }}
            const data = await res.json();
            if (!res.ok) throw new Error(data.error || 'Failed to load servers');
            document.getElementById('loading').classList.add('hidden');
            document.getElementById('content').classList.remove('hidden');
            const c = document.getElementById('guilds');
            const manageable = (data.guilds || []).filter(g => g.manage_guild);
            if (manageable.length === 0) {{
                c.innerHTML = '<p>No Discord servers found where you have <strong>Manage Server</strong> permission. Only server managers can register games.</p>';
                return;
            }}
            c.innerHTML = manageable.map(g => {{
                const label = g.guild_name || ('Server ' + g.guild_id);
                return '<div class="guild"><div><div class="guild-name">' + esc(label) + '</div></div><a class="btn" href="' + API + '/games/' + encodeURIComponent(g.guild_id) + '">Manage games</a></div>';
            }}).join('');
        }} catch (e) {{
            document.getElementById('loading').classList.add('hidden');
            const el = document.getElementById('error');
            el.textContent = e.message;
            el.classList.remove('hidden');
        }}
    }}
    load();
    </script>
</body>
</html>"##
    )
}

pub fn render_games_page(base_url: &str) -> String {
    format!(
        r##"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Roblox Player Roles - Game Integrations</title>
    <link rel="icon" href="{base_url}/favicon.ico" type="image/x-icon">
    <meta name="theme-color" content="#232527">
    <style>
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{ font-family: system-ui, -apple-system, sans-serif; max-width: 780px; margin: 0 auto; padding: 32px 20px; background: #232527; color: #ebedf0; min-height: 100vh; }}
        h1 {{ color: #00a2ff; font-size: 24px; }}
        h2 {{ color: #fff; font-size: 17px; margin: 18px 0 8px; }}
        h3 {{ color: #ebedf0; font-size: 15px; margin: 10px 0 4px; }}
        p, li {{ color: #b8bcc1; font-size: 14px; line-height: 1.6; }}
        a {{ color: #00a2ff; }}
        code {{ font-family: 'Courier New', monospace; background: #232527; padding: 2px 6px; border-radius: 4px; font-size: 12px; color: #ebedf0; }}
        pre {{ background: #1a1c1e; padding: 14px; border-radius: 6px; overflow-x: auto; font-size: 12px; line-height: 1.5; color: #ebedf0; border: 1px solid #3d4144; }}
        .card {{ background: #2e3133; padding: 22px; border-radius: 10px; margin: 12px 0; border: 1px solid #3d4144; }}
        .btn {{ display: inline-block; padding: 8px 18px; background: #00a2ff; color: #fff; text-decoration: none; border-radius: 6px; font-size: 13px; font-weight: 500; border: none; cursor: pointer; font-family: inherit; }}
        .btn:hover {{ background: #0086d3; }}
        .btn-danger {{ background: transparent; color: #f87171; border: 1px solid #7f1d1d; }}
        .btn-danger:hover {{ background: #7f1d1d33; }}
        .row {{ display: flex; gap: 8px; align-items: center; margin: 6px 0; }}
        input[type=text], input[type=number] {{ flex: 1; padding: 8px 12px; border-radius: 6px; border: 1px solid #3d4144; background: #232527; color: #ebedf0; font-family: inherit; font-size: 13px; }}
        textarea {{ width: 100%; min-height: 80px; padding: 8px; border-radius: 6px; border: 1px solid #3d4144; background: #232527; color: #ebedf0; font-family: 'Courier New', monospace; font-size: 12px; }}
        .universe-card {{ border-left: 3px solid #00a2ff; }}
        .label {{ color: #8a9099; font-size: 11px; text-transform: uppercase; letter-spacing: 0.5px; margin-bottom: 4px; display: block; }}
        .secret-box {{ background: #1a1c1e; padding: 10px 14px; border-radius: 6px; border: 1px solid #3d4144; font-family: 'Courier New', monospace; font-size: 12px; word-break: break-all; }}
        .msg {{ padding: 10px 14px; border-radius: 6px; font-size: 13px; position: fixed; top: 16px; left: 50%; transform: translateX(-50%); z-index: 1000; max-width: calc(100% - 32px); box-shadow: 0 6px 24px rgba(0,0,0,0.45); }}
        .msg-error {{ background: #1c0a0a; color: #fca5a5; border: 1px solid #7f1d1d; }}
        .msg-success {{ background: #052e16; color: #86efac; border: 1px solid #14532d; }}
        .hidden {{ display: none; }}
        .badge-on {{ display: inline-block; padding: 2px 8px; background: #052e16; color: #4ade80; border: 1px solid #14532d; border-radius: 12px; font-size: 11px; }}
        .badge-off {{ display: inline-block; padding: 2px 8px; background: #1c0a0a; color: #fca5a5; border: 1px solid #7f1d1d; border-radius: 12px; font-size: 11px; }}
        .status-box {{ margin-top: 10px; padding: 10px 14px; border-radius: 6px; border: 1px solid #3d4144; background: #1a1c1e; font-size: 13px; display: flex; align-items: center; gap: 10px; }}
        .status-dot {{ width: 10px; height: 10px; border-radius: 50%; flex-shrink: 0; }}
        .status-ok {{ background: #4ade80; box-shadow: 0 0 8px #4ade8088; }}
        .status-stale {{ background: #fbbf24; box-shadow: 0 0 8px #fbbf2488; }}
        .status-none {{ background: #fca5a5; }}
        .status-text {{ color: #ebedf0; flex: 1; }}
        .status-meta {{ color: #8a9099; font-size: 11px; margin-top: 2px; }}
    </style>
</head>
<body>
    <h1>Game Integrations</h1>
    <p id="guild-context" style="margin-top:4px;"></p>
    <p>Connect your Roblox game so members of <strong id="guild-name-inline">this server</strong> get roles automatically based on their in-game progress. Registrations are private to this server — other Discord servers cannot reference this game's data.</p>
    <p style="margin-top:6px;"><a href="{base_url}/games">← Pick a different server</a> · <a href="{base_url}/verify">Player verification page</a></p>

    <div id="msg" class="hidden"></div>

    <div id="loading" class="card"><p>Loading...</p></div>

    <div id="login-prompt" class="card hidden">
        <p>You're not signed in. <a id="login-link" href="#">Login with Discord</a> to manage your games.</p>
    </div>

    <div id="forbidden" class="card hidden">
        <p>You don't have <strong>Manage Server</strong> permission for this Discord server, so you can't register games for it.</p>
    </div>

    <div id="content" class="hidden">
        <div class="card">
            <h2>Register a new Roblox game</h2>
            <p>Pick <strong>one</strong> integration mode per universe — push and pull writing the same fields would race, so they're mutually exclusive.</p>
            <div style="margin:10px 0; padding:10px; background:#1a1c1e; border-radius:6px; border:1px solid #3d4144;">
                <label style="display:block; margin:4px 0; cursor:pointer;">
                    <input type="radio" name="new-mode" value="pull" checked> <strong>Pull (Open Cloud DataStore)</strong>
                    <span style="display:block; font-size:12px; color:#8a9099; margin:2px 0 0 22px;">Recommended. Server polls a Roblox DataStore via the Open Cloud key. No code in your game. Updates every poll interval (default 600s).</span>
                </label>
                <label style="display:block; margin:8px 0 4px; cursor:pointer;">
                    <input type="radio" name="new-mode" value="push"> <strong>Push (Studio plugin / HttpService)</strong>
                    <span style="display:block; font-size:12px; color:#8a9099; margin:2px 0 0 22px;">Game posts to a webhook on a batch interval (default 60s). Use if your game has no DataStore for the stats you need.</span>
                </label>
            </div>
            <p>You'll need your Roblox <strong>Universe ID</strong> (find it in <a href="https://create.roblox.com/dashboard/creations" target="_blank">Creator Dashboard</a> → game settings) and an <strong>Open Cloud API key</strong> scoped to that universe. The key is used to prove you own this universe.</p>
            <details style="margin:6px 0 12px;">
                <summary style="cursor:pointer; color:#00a2ff;">How to create the Open Cloud API key</summary>
                <ol style="margin:8px 0 4px 20px; color:#b8bcc1;">
                    <li>Go to <a href="https://create.roblox.com/dashboard/credentials" target="_blank">Creator Dashboard → Credentials → Open Cloud API Keys</a> → <strong>Create API Key</strong>.</li>
                    <li>Give it any name. In <strong>Access Permissions</strong>, click <strong>Select API System</strong> and pick <strong>universe-datastores</strong>. Make sure <strong>Restrict by Experience</strong> is ON for that row and pick your specific game (don't use "All Experiences").</li>
                    <li>Open the <strong>Select Operations to Add</strong> dropdown for <strong>universe-datastores</strong> and tick these operations (scroll the dropdown — they're under different sub-groups):
                        <ul style="margin:4px 0 4px 20px;">
                            <li><strong>universe-datastores.controls → list</strong> — required for the ownership check (we use it to list DataStores in your universe; a 200 proves the key is scoped to this universe).</li>
                            <li><strong>universe-datastores.objects → list</strong> — pull mode only (list entries inside the chosen DataStore).</li>
                            <li><strong>universe-datastores.objects → read</strong> — pull mode only (read each entry).</li>
                        </ul>
                        Push mode only needs <code>controls.list</code>; pull mode needs all three.
                    </li>
                    <li>Save → copy the key (shown once) → paste below.</li>
                </ol>
                <p style="font-size:12px; color:#8a9099; margin-top:6px;">A key scoped to game A cannot register universe B's ID — Roblox enforces per-universe scope on every probe endpoint, so the registration call will fail.</p>
            </details>
            <div class="row">
                <input type="text" id="new-universe-id" placeholder="Universe ID (numeric)">
                <input type="text" id="new-display-name" placeholder="Game name (for display)">
            </div>
            <div class="row" style="margin-top:6px;">
                <input type="text" id="new-open-cloud-key" placeholder="Open Cloud API key (rbxop_... or rbxak_...) — for ownership check">
                <button class="btn" onclick="createUniverse()">Register</button>
            </div>
        </div>

        <h2>Registered games for this server</h2>
        <div id="universes"></div>
    </div>

    <script>
    const API = '{base_url}';
    const guildId = (function() {{
        const parts = window.location.pathname.split('/').filter(Boolean);
        return parts[parts.indexOf('games') + 1] || '';
    }})();
    (function () {{
        const returnTo = window.location.pathname;
        document.getElementById('login-link').href = '/auth/login?return_to=' + encodeURIComponent(returnTo);
    }})();
    function showMsg(text, type) {{
        const el = document.getElementById('msg');
        el.className = 'msg msg-' + type;
        el.textContent = text;
        el.classList.remove('hidden');
        setTimeout(() => el.classList.add('hidden'), 6000);
    }}
    async function api(method, path, body) {{
        const opts = {{ method, headers: {{}}, credentials: 'include' }};
        if (body) {{ opts.headers['Content-Type'] = 'application/json'; opts.body = JSON.stringify(body); }}
        const res = await fetch(API + path, opts);
        const data = await res.json().catch(() => ({{}}));
        if (!res.ok) {{ const err = new Error(data.error || 'Request failed'); err.status = res.status; throw err; }}
        return data;
    }}
    function esc(s) {{ const d = document.createElement('div'); d.textContent = s; return d.innerHTML; }}
    function flattenPaths(obj, prefix, out) {{
        out = out || [];
        prefix = prefix || '';
        if (obj && typeof obj === 'object' && !Array.isArray(obj)) {{
            Object.keys(obj).forEach(function(k) {{
                const p = prefix ? prefix + '.' + k : k;
                const v = obj[k];
                if (v && typeof v === 'object' && !Array.isArray(v)) {{
                    flattenPaths(v, p, out);
                }} else {{
                    out.push(p);
                }}
            }});
        }}
        return out;
    }}
    function defaultCustomKey(path) {{
        // Use the leaf segment as the suggested custom key (e.g. "Stats.Coins" → "Coins").
        const parts = path.split('.');
        return parts[parts.length - 1];
    }}
    function renderMapper(uid, sample, savedMap) {{
        const container = document.getElementById('oc-mapper-' + uid);
        if (!container) return;
        const saved = savedMap || {{}};
        const paths = sample ? flattenPaths(sample) : [];
        Object.keys(saved).forEach(function(k) {{ if (paths.indexOf(k) === -1) paths.push(k); }});
        if (paths.length === 0) {{
            container.innerHTML = '<p style="color:#8a9099; font-size:12px; margin:6px 0;">No fields detected yet — click <strong>Detect fields from a sample entry</strong> above to load your DataStore schema.</p>';
            return;
        }}
        const hasSaved = Object.keys(saved).length > 0;
        container.innerHTML = paths.map(function(p) {{
            const def = defaultCustomKey(p);
            const cur = saved[p];
            const enabled = hasSaved ? (cur !== undefined && cur !== null && cur !== '') : true;
            const keyVal = (cur && cur !== '') ? cur : def;
            return '<div class="mapper-row" style="margin:4px 0; padding:8px 10px; background:#1a1c1e; border-radius:6px; border:1px solid #3d4144; display:flex; gap:10px; align-items:center;">' +
                '<input type="checkbox" data-enabled ' + (enabled ? 'checked' : '') + ' style="cursor:pointer; width:16px; height:16px;">' +
                '<code style="flex:1; padding:6px 10px; background:#232527; border-radius:6px; border:1px solid #3d4144; overflow-wrap:anywhere;">' + esc(p) + '</code>' +
                '<span style="color:#8a9099;">→</span>' +
                '<input type="text" data-path="' + esc(p) + '" data-custom value="' + esc(keyVal) + '" placeholder="' + esc(def) + '" style="flex:0 0 220px; padding:6px 10px; border-radius:6px; border:1px solid #3d4144; background:#232527; color:#ebedf0; font-family:inherit; font-size:13px;">' +
            '</div>';
        }}).join('');
    }}
    function buildMapperJson(uid) {{
        const container = document.getElementById('oc-mapper-' + uid);
        if (!container) return {{}};
        const out = {{}};
        container.querySelectorAll('.mapper-row').forEach(function(row) {{
            const cb = row.querySelector('input[data-enabled]');
            if (!cb || !cb.checked) return;
            const inp = row.querySelector('input[data-custom]');
            if (!inp) return;
            const path = inp.getAttribute('data-path');
            const target = inp.value.trim() || defaultCustomKey(path);
            if (path && target) out[path] = target;
        }});
        return out;
    }}
    function timeAgo(iso) {{
        if (!iso) return 'never';
        const s = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
        if (s < 0)   return 'just now';
        if (s < 60)  return s + 's ago';
        if (s < 3600) return Math.floor(s/60) + 'm ago';
        if (s < 86400) return Math.floor(s/3600) + 'h ago';
        return Math.floor(s/86400) + 'd ago';
    }}
    function renderStatus(u) {{
        const lastPush = u.last_push_at ? new Date(u.last_push_at).getTime() : 0;
        const lastPull = u.last_pull_at ? new Date(u.last_pull_at).getTime() : 0;
        const lastFetched = u.last_stat_fetched_at ? new Date(u.last_stat_fetched_at).getTime() : 0;
        const lastAny = Math.max(lastPush, lastPull, lastFetched);
        const ageSec = lastAny > 0 ? Math.floor((Date.now() - lastAny) / 1000) : Infinity;
        const count = u.players_count || 0;
        let dot, title, meta;
        if (lastAny === 0 && count === 0) {{
            dot = 'status-none';
            title = '<strong>Setup not confirmed yet</strong> — no stats received from your game.';
            meta = 'Once a player joins your live (published) game and the Studio plugin posts a batch (default 60s), this turns green.';
        }} else if (ageSec <= 300) {{
            dot = 'status-ok';
            title = '<strong>Setup confirmed — stats flowing</strong>';
            meta = count + ' player ' + (count === 1 ? 'row' : 'rows') + ' tracked. Last push: ' + timeAgo(u.last_push_at) + '. Last pull: ' + timeAgo(u.last_pull_at) + '.';
        }} else {{
            dot = 'status-stale';
            title = '<strong>No recent activity</strong> — last update ' + timeAgo(new Date(lastAny).toISOString());
            meta = count + ' player ' + (count === 1 ? 'row' : 'rows') + ' on file. Stats arrive while players are in the live game; this is normal if no one is playing right now.';
        }}
        return '<div class="status-box"><div class="status-dot ' + dot + '"></div><div style="flex:1;"><div class="status-text">' + title + '</div><div class="status-meta">' + meta + '</div></div></div>';
    }}
    function renderPullSection(u) {{
        const isPull = u.mode === 'pull';
        if (!isPull) return '';
        return `
            <details style="margin-top:14px;" open>
                <summary style="cursor:pointer; color:#00a2ff;"><strong>Open Cloud DataStore configuration</strong></summary>
                <div style="margin-top:8px;">
                    <p>Your Open Cloud key is already saved (encrypted) from registration. To start polling, the saved key needs three operations under the <strong>universe-datastores</strong> API System (Restrict by Experience ON, scoped to this universe): <strong>controls.list</strong>, <strong>objects.list</strong>, and <strong>objects.read</strong>. Add any that are missing in the Roblox dashboard (open the operations dropdown and scroll — they're under different sub-groups).</p>
                    <div class="row"><input type="text" id="oc-key-${{esc(u.universe_id)}}" placeholder="Replace API key (leave blank to keep saved key)"></div>

                    <p style="margin-top:8px;"><span class="label">DataStore name</span></p>
                    <div class="row">
                        <select id="oc-ds-select-${{esc(u.universe_id)}}" style="flex:1; padding:8px 12px; border-radius:6px; border:1px solid #3d4144; background:#232527; color:#ebedf0; font-family:inherit; font-size:13px;" onchange="onDsSelect('${{esc(u.universe_id)}}', this.value)">
                            <option value="">${{u.datastore_name ? '— change —' : '— pick one —'}}</option>
                            ${{u.datastore_name ? '<option value="' + esc(u.datastore_name) + '" selected>' + esc(u.datastore_name) + '</option>' : ''}}
                        </select>
                        <button class="btn" onclick="fetchDatastores('${{esc(u.universe_id)}}')">Fetch DataStores</button>
                    </div>
                    <input type="text" id="oc-ds-${{esc(u.universe_id)}}" placeholder="…or type DataStore name manually (e.g. PlayerData)" value="${{esc(u.datastore_name || '')}}" style="width:100%; padding:8px 12px; border-radius:6px; border:1px solid #3d4144; background:#232527; color:#ebedf0; font-family:inherit; font-size:13px; margin-top:6px;">

                    <div class="row" style="margin-top:8px;"><input type="number" id="oc-poll-${{esc(u.universe_id)}}" placeholder="Poll interval (seconds)" value="${{u.poll_interval_seconds || 600}}"></div>

                    <p style="margin-top:8px;"><span class="label">Entry key template</span> — pattern your game uses to key each player's DataStore entry. Use the literal token <code>{{user_id}}</code> where the Roblox UserId goes (e.g. <code>Player_{{user_id}}</code>). Click <strong>Detect fields</strong> below to auto-fill from a sample entry.</p>
                    <div class="row" style="margin-top:4px;"><input type="text" id="oc-key-template-${{esc(u.universe_id)}}" placeholder="{{user_id}}" value="${{esc(u.entry_key_template || '{{user_id}}')}}"></div>

                    <p style="margin-top:8px;"><span class="label">Stat field map</span> — checkbox toggles each field on/off. Each key defaults to the field name; edit it directly to use a different stat key. Unchecked fields are skipped.</p>
                    <div class="row" style="margin-bottom:6px;"><button class="btn" onclick="previewEntry('${{esc(u.universe_id)}}')">Detect fields from a sample entry</button></div>
                    <pre id="oc-sample-${{esc(u.universe_id)}}" class="hidden" style="margin:6px 0; max-height:240px; overflow:auto;"></pre>
                    <div id="oc-mapper-${{esc(u.universe_id)}}"></div>

                    <div class="row" style="margin-top:8px;">
                        <button class="btn" onclick="saveOpenCloud('${{esc(u.universe_id)}}')">Save DataStore config</button>
                        <button class="btn btn-danger" onclick="clearOpenCloud('${{esc(u.universe_id)}}')">Pause polling</button>
                    </div>
                    <p style="font-size:12px; color:#8a9099; margin-top:6px;">Polling is ${{u.pull_enabled ? '<strong style="color:#4ade80;">active</strong>' : '<strong style="color:#fbbf24;">not active yet</strong> — fill in DataStore name and Save'}}.</p>
                </div>
            </details>`;
    }}
    function renderUniverse(u) {{
        const ingestUrl = API + '/ingest/' + encodeURIComponent(u.universe_id) + '/stats';
        const rbxmUrl = API + '/studio-plugin/Roblox-Player-Role.rbxm';
        const luaSnippet =
            'local HttpService = game:GetService("HttpService")\n' +
            'local Players = game:GetService("Players")\n' +
            'local WEBHOOK = "' + ingestUrl + '"\n' +
            'local SECRET  = "<paste-your-ingest-secret-here>"\n\n' +
            'local function snapshot(p)\n' +
            '    local ls = p:FindFirstChild("leaderstats")\n' +
            '    return {{\n' +
            '        user_id = tostring(p.UserId),\n' +
            '        stats   = {{ level = ls and ls:FindFirstChild("Level") and ls.Level.Value or 0 }}\n' +
            '    }}\n' +
            'end\n\n' +
            'task.spawn(function()\n' +
            '    while true do\n' +
            '        local players = {{}}\n' +
            '        for _, p in ipairs(Players:GetPlayers()) do table.insert(players, snapshot(p)) end\n' +
            '        if #players > 0 then\n' +
            '            pcall(function()\n' +
            '                HttpService:PostAsync(WEBHOOK, HttpService:JSONEncode({{players = players}}),\n' +
            '                    Enum.HttpContentType.ApplicationJson, false,\n' +
            '                    {{ ["X-Ingest-Secret"] = SECRET }})\n' +
            '            end)\n' +
            '        end\n' +
            '        task.wait(60)\n' +
            '    end\n' +
            'end)';
        const isPush = u.mode === 'push';
        const isPull = u.mode === 'pull';
        const modeBadge = isPush
            ? '<span class="badge-on" style="background:#1a2a3a; color:#7dd3fc; border-color:#1e3a52;">push mode</span>'
            : '<span class="badge-on" style="background:#2a1a3a; color:#c4b5fd; border-color:#3b2a52;">pull mode</span>';

        const pushSection = isPush ? `
            <p style="margin-top:12px;"><span class="label">Webhook URL</span></p>
            <div class="secret-box">${{esc(ingestUrl)}}</div>

            <details style="margin-top:14px;">
                <summary style="cursor:pointer; color:#00a2ff;">Show ingest secret &amp; rotate</summary>
                <div style="margin-top:8px;">
                    <button class="btn" onclick="rotateSecret('${{esc(u.universe_id)}}')">Generate new secret</button>
                    <p style="margin-top:6px; font-size:12px;">Secrets are shown only once. The Studio plugin and any custom server scripts must be updated after rotation.</p>
                    <div id="secret-display-${{esc(u.universe_id)}}" class="hidden" style="margin-top:10px;">
                        <span class="label">New ingest secret (copy now)</span>
                        <div class="secret-box" id="secret-value-${{esc(u.universe_id)}}"></div>
                        <button class="btn" style="margin-top:6px; padding: 6px 14px; font-size: 12px;" onclick="copySecret('${{esc(u.universe_id)}}')">Copy</button>
                    </div>
                </div>
            </details>

            <details style="margin-top:14px;" open>
                <summary style="cursor:pointer; color:#00a2ff;"><strong>Studio plugin install guide</strong></summary>
                <div style="margin-top:10px;">
                    <p><strong>Step 1 — Download</strong></p>
                    <p style="margin:6px 0;"><a class="btn" href="${{rbxmUrl}}" download>Download Roblox-Player-Role.rbxm</a></p>

                    <p style="margin-top:14px;"><strong>Step 2 — Install in Roblox Studio</strong></p>
                    <ol style="margin:6px 0 6px 20px; color:#b8bcc1;">
                        <li>Open your game's place file in <a href="https://create.roblox.com/dashboard/creations" target="_blank">Roblox Studio</a>.</li>
                        <li>Right-click <code>ServerScriptService</code> → <strong>Insert</strong> → <strong>Import Roblox Model</strong> and pick the downloaded <code>Roblox-Player-Role.rbxm</code>.</li>
                    </ol>

                    <p style="margin-top:14px;"><strong>Step 3 — Configure</strong></p>
                    <p style="margin:6px 0;">Open the child <code>Config</code> ModuleScript and set:</p>
                    <ul style="margin:4px 0 4px 20px; color:#b8bcc1;">
                        <li><code>WebhookUrl = </code> the URL above (in quotes).</li>
                        <li><code>IngestSecret = </code> the secret shown at registration (in quotes).</li>
                    </ul>

                    <p style="margin-top:14px;"><strong>Step 4 — Allow HTTP &amp; publish</strong></p>
                    <ol style="margin:6px 0 6px 20px; color:#b8bcc1;">
                        <li><strong>File → Experience Settings → Security → Allow HTTP Requests = ON</strong>.</li>
                        <li><strong>File → Publish to Roblox</strong>.</li>
                    </ol>
                </div>
            </details>

            <details style="margin-top:14px;">
                <summary style="cursor:pointer; color:#00a2ff;">Alternative: paste your own server script</summary>
                <p style="margin-top:8px; font-size:12px;">Drop this into a ServerScript instead of the plugin. Same wire format.</p>
                <pre>${{esc(luaSnippet)}}</pre>
            </details>` : '';

        return `
        <div class="card universe-card" id="u-${{esc(u.universe_id)}}">
            <h3>${{esc(u.display_name || 'Game ' + u.universe_id)}} ${{modeBadge}}</h3>
            <p><span class="label">Universe ID</span> <code>${{esc(u.universe_id)}}</code></p>
            <div id="status-${{esc(u.universe_id)}}">${{renderStatus(u)}}</div>
            ${{pushSection}}
            <div id="pull-section-${{esc(u.universe_id)}}">${{renderPullSection(u)}}</div>
            <div style="margin-top:18px;">
                <button class="btn btn-danger" onclick="deleteUniverse('${{esc(u.universe_id)}}')">Delete this game</button>
            </div>
        </div>`;
    }}
    async function load() {{
        try {{
            const data = await api('GET', '/games/' + encodeURIComponent(guildId) + '/data');
            document.getElementById('loading').classList.add('hidden');
            document.getElementById('content').classList.remove('hidden');
            const guildLabel = data.guild_name || ('Server ' + guildId);
            document.getElementById('guild-context').innerHTML = 'Managing games for <strong>' + esc(guildLabel) + '</strong>';
            document.getElementById('guild-name-inline').textContent = guildLabel;
            document.title = guildLabel + ' - Game Integrations';
            const c = document.getElementById('universes');
            c.innerHTML = data.universes.map(renderUniverse).join('') || '<p style="color:#8a9099;">No games yet — register one above.</p>';
            data.universes.forEach(function(u) {{
                if (u.mode === 'pull') renderMapper(u.universe_id, null, u.stat_field_map || {{}});
            }});
        }} catch (e) {{
            document.getElementById('loading').classList.add('hidden');
            if (e.status === 401) {{ document.getElementById('login-prompt').classList.remove('hidden'); return; }}
            if (e.status === 403) {{ document.getElementById('forbidden').classList.remove('hidden'); return; }}
            showMsg(e.message, 'error');
        }}
    }}
    async function createUniverse() {{
        const universe_id = document.getElementById('new-universe-id').value.trim();
        const display_name = document.getElementById('new-display-name').value.trim();
        const open_cloud_api_key = document.getElementById('new-open-cloud-key').value.trim();
        const modeEl = document.querySelector('input[name="new-mode"]:checked');
        const mode = modeEl ? modeEl.value : 'pull';
        if (!universe_id || !/^[0-9]+$/.test(universe_id)) return showMsg('Universe ID must be numeric', 'error');
        if (!open_cloud_api_key) return showMsg('Open Cloud API key is required to prove you own this universe', 'error');
        try {{
            await api('POST', '/games/' + encodeURIComponent(guildId), {{ universe_id, display_name, open_cloud_api_key, mode }});
            const note = mode === 'push'
                ? 'Registered (push mode). Open the new game card below and click "Generate new secret" when you are ready to install the Studio plugin.'
                : 'Registered (pull mode). Fetching DataStores and detecting fields…';
            showMsg(note, 'success');
            document.getElementById('new-open-cloud-key').value = '';
            await load();
            if (mode === 'pull') {{ await autoSetupPull(universe_id); }}
        }} catch (e) {{ showMsg(e.message, 'error'); }}
    }}
    async function rotateSecret(uid) {{
        if (!confirm('Rotate ingest secret for this universe? Existing scripts will stop working until updated.')) return;
        try {{
            const r = await api('POST', '/games/' + encodeURIComponent(guildId) + '/' + encodeURIComponent(uid) + '/regenerate-secret');
            const wrap = document.getElementById('secret-display-' + uid);
            const val = document.getElementById('secret-value-' + uid);
            val.textContent = r.ingest_secret;
            wrap.classList.remove('hidden');
            showMsg('New ingest secret generated. Copy it from the section below — it will not be shown again.', 'success');
        }} catch (e) {{ showMsg(e.message, 'error'); }}
    }}
    async function copySecret(uid) {{
        const val = document.getElementById('secret-value-' + uid);
        if (!val) return;
        try {{
            await navigator.clipboard.writeText(val.textContent);
            showMsg('Secret copied to clipboard.', 'success');
        }} catch (e) {{ showMsg('Copy failed — select the secret manually.', 'error'); }}
    }}
    async function saveOpenCloud(uid) {{
        const key = document.getElementById('oc-key-' + uid).value.trim();
        const datastore_name = document.getElementById('oc-ds-' + uid).value.trim();
        const poll_interval_seconds = parseInt(document.getElementById('oc-poll-' + uid).value, 10) || 600;
        const stat_field_map = buildMapperJson(uid);
        const entry_key_template = (document.getElementById('oc-key-template-' + uid).value || '').trim() || '{{user_id}}';
        if (entry_key_template.indexOf('{{user_id}}') === -1) {{
            return showMsg('Entry key template must contain {{user_id}} (replaced with each linked player\'s Roblox ID).', 'error');
        }}
        try {{
            await api('POST', '/games/' + encodeURIComponent(guildId) + '/' + encodeURIComponent(uid) + '/open-cloud',
                {{ open_cloud_api_key: key, datastore_name, poll_interval_seconds, stat_field_map, entry_key_template }});
            showMsg('Open Cloud config saved. Waiting for first poll to confirm setup…', 'success');
            const box = document.getElementById('status-' + uid);
            if (box) box.innerHTML = '<div class="status-box"><div class="status-dot status-none"></div><div style="flex:1;"><div class="status-text"><strong>Saved — waiting for first poll…</strong></div><div class="status-meta">Polling has just started. The status will update once the first DataStore fetch completes (usually within 10–30s).</div></div></div>';
            // Poll for fresh status: the worker needs a moment to run the first
            // fetch after save. Try a few times with backoff before giving up.
            const delays = [3000, 5000, 8000, 12000, 20000];
            for (let i = 0; i < delays.length; i++) {{
                await new Promise(function(r) {{ setTimeout(r, delays[i]); }});
                const confirmed = await refreshStatus(uid);
                if (confirmed) break;
            }}
        }} catch (e) {{ showMsg(e.message, 'error'); }}
    }}
    async function refreshStatus(uid) {{
        try {{
            const data = await api('GET', '/games/' + encodeURIComponent(guildId) + '/data');
            const u = (data.universes || []).find(function(x) {{ return x.universe_id === uid; }});
            const box = document.getElementById('status-' + uid);
            const pullBox = document.getElementById('pull-section-' + uid);
            if (!u || !box) return false;
            const lastAny = Math.max(
                u.last_push_at ? new Date(u.last_push_at).getTime() : 0,
                u.last_pull_at ? new Date(u.last_pull_at).getTime() : 0,
                u.last_stat_fetched_at ? new Date(u.last_stat_fetched_at).getTime() : 0
            );
            const confirmed = lastAny > 0 || (u.players_count || 0) > 0;
            if (confirmed) box.innerHTML = renderStatus(u);
            if (pullBox) {{
                // Preserve sample entry content before re-rendering
                const oldSampleEl = document.getElementById('oc-sample-' + uid);
                const sampleContent = oldSampleEl ? oldSampleEl.textContent : '';
                const sampleHidden = oldSampleEl ? oldSampleEl.classList.contains('hidden') : true;
                
                pullBox.innerHTML = renderPullSection(u);
                
                // Restore sample entry content
                const newSampleEl = document.getElementById('oc-sample-' + uid);
                if (newSampleEl && sampleContent) {{
                    newSampleEl.textContent = sampleContent;
                    if (!sampleHidden) newSampleEl.classList.remove('hidden');
                }}
            }}
            if (u.mode === 'pull') renderMapper(uid, null, u.stat_field_map || {{}});
            return confirmed;
        }} catch (e) {{ return false; }}
    }}
    function onDsSelect(uid, value) {{
        if (value) document.getElementById('oc-ds-' + uid).value = value;
    }}
    async function autoSetupPull(uid) {{
        try {{
            const r = await api('GET', '/games/' + encodeURIComponent(guildId) + '/' + encodeURIComponent(uid) + '/datastores');
            const list = r.datastores || [];
            const sel = document.getElementById('oc-ds-select-' + uid);
            if (!sel) return;
            if (list.length === 0) {{
                showMsg('Registered, but no DataStores found in this universe yet.', 'error');
                return;
            }}
            sel.innerHTML = '<option value="">— pick one —</option>' + list.map(function(n) {{
                return '<option value="' + esc(n) + '">' + esc(n) + '</option>';
            }}).join('');
            const first = list[0];
            sel.value = first;
            const dsInput = document.getElementById('oc-ds-' + uid);
            if (dsInput) dsInput.value = first;
            await previewEntry(uid);
        }} catch (e) {{
            showMsg('Auto-setup: ' + e.message, 'error');
        }}
    }}
    async function fetchDatastores(uid) {{
        try {{
            const r = await api('GET', '/games/' + encodeURIComponent(guildId) + '/' + encodeURIComponent(uid) + '/datastores');
            const sel = document.getElementById('oc-ds-select-' + uid);
            const current = document.getElementById('oc-ds-' + uid).value;
            sel.innerHTML = '<option value="">— pick one —</option>' + (r.datastores || []).map(n =>
                '<option value="' + esc(n) + '"' + (n === current ? ' selected' : '') + '>' + esc(n) + '</option>'
            ).join('');
            if (!r.datastores || r.datastores.length === 0) {{
                showMsg('No DataStores found in this universe', 'error');
            }} else {{
                showMsg('Found ' + r.datastores.length + ' DataStore(s). Pick one from the dropdown.', 'success');
            }}
        }} catch (e) {{ showMsg(e.message, 'error'); }}
    }}
    async function previewEntry(uid) {{
        const datastore_name = document.getElementById('oc-ds-' + uid).value.trim();
        if (!datastore_name) return showMsg('Pick a DataStore name first', 'error');
        const pre = document.getElementById('oc-sample-' + uid);
        try {{
            const params = new URLSearchParams({{ datastore_name }});
            const r = await api('GET', '/games/' + encodeURIComponent(guildId) + '/' + encodeURIComponent(uid) + '/sample-entry?' + params);
            if (r.note) {{
                pre.textContent = '(' + r.note + ')';
            }} else {{
                pre.textContent = 'entry_id: ' + r.entry_id + '\\n\\n' + JSON.stringify(r.value, null, 2);
                const current = buildMapperJson(uid);
                renderMapper(uid, r.value, current);
                // Auto-suggest entry-key template by replacing the longest digit
                // run in the sample entry_id with the {{user_id}} placeholder.
                // Only suggest when the user hasn't customised it (still on the
                // default `{{user_id}}`), to avoid clobbering deliberate edits.
                const tplEl = document.getElementById('oc-key-template-' + uid);
                if (tplEl && tplEl.value.trim() === '{{user_id}}' && r.entry_id) {{
                    const digits = r.entry_id.match(/[0-9]{{3,}}/g);
                    if (digits && digits.length > 0) {{
                        const longest = digits.sort(function(a, b) {{ return b.length - a.length; }})[0];
                        const suggested = r.entry_id.replace(longest, '{{user_id}}');
                        if (suggested !== r.entry_id) {{
                            tplEl.value = suggested;
                            showMsg('Entry key template auto-detected: ' + suggested + ' (review and Save).', 'success');
                        }}
                    }}
                }}
            }}
            pre.classList.remove('hidden');
        }} catch (e) {{ showMsg(e.message, 'error'); }}
    }}
    async function clearOpenCloud(uid) {{
        if (!confirm('Disable Open Cloud pull for this universe?')) return;
        try {{
            await api('POST', '/games/' + encodeURIComponent(guildId) + '/' + encodeURIComponent(uid) + '/open-cloud',
                {{ open_cloud_api_key: '', datastore_name: '', stat_field_map: {{}} }});
            showMsg('Pull disabled.', 'success');
            const data = await api('GET', '/games/' + encodeURIComponent(guildId) + '/data');
            const u = (data.universes || []).find(function(x) {{ return x.universe_id === uid; }});
            if (u) {{
                const box = document.getElementById('status-' + uid);
                const pullBox = document.getElementById('pull-section-' + uid);
                if (box) box.innerHTML = renderStatus(u);
                if (pullBox) {{
                    const oldSampleEl = document.getElementById('oc-sample-' + uid);
                    const sampleContent = oldSampleEl ? oldSampleEl.textContent : '';
                    const sampleHidden = oldSampleEl ? oldSampleEl.classList.contains('hidden') : true;
                    
                    pullBox.innerHTML = renderPullSection(u);
                    
                    const newSampleEl = document.getElementById('oc-sample-' + uid);
                    if (newSampleEl && sampleContent) {{
                        newSampleEl.textContent = sampleContent;
                        if (!sampleHidden) newSampleEl.classList.remove('hidden');
                    }}
                }}
                if (u.mode === 'pull') renderMapper(uid, null, u.stat_field_map || {{}});
            }}
        }} catch (e) {{ showMsg(e.message, 'error'); }}
    }}
    async function deleteUniverse(uid) {{
        if (!confirm('Delete this game? All cached in-game stats will be removed.')) return;
        try {{
            await api('POST', '/games/' + encodeURIComponent(guildId) + '/' + encodeURIComponent(uid) + '/delete');
            showMsg('Deleted.', 'success');
            await load();
        }} catch (e) {{ showMsg(e.message, 'error'); }}
    }}
    load();
    </script>
</body>
</html>"##
    )
}

pub async fn landing_page(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        state.games_landing_html.clone(),
    )
}

pub async fn games_page(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        state.games_html.clone(),
    )
}

/// JSON list of caller's guilds (powers the landing page picker).
pub async fn my_guilds_data(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<Value>, AppError> {
    let cookie = jar
        .get(SESSION_COOKIE)
        .ok_or(AppError::Unauthorized)?
        .value()
        .to_string();
    let body = auth_gateway_get(&state, "/auth/my_guilds", &cookie).await?;
    Ok(Json(body))
}

pub async fn games_data(
    State(state): State<Arc<AppState>>,
    Path(guild_id): Path<String>,
    jar: CookieJar,
) -> Result<Json<Value>, AppError> {
    let discord_id = require_manager(&state, &jar, &guild_id).await?;

    // Best-effort: pull guild_name from AG to label the page.
    let cookie = jar.get(SESSION_COOKIE).map(|c| c.value().to_string()).unwrap_or_default();
    let guild_name = auth_gateway_get(
        &state,
        &format!("/auth/guild_members?guild_id={}", urlencoding::encode(&guild_id)),
        &cookie,
    )
    .await
    .ok()
    .and_then(|v| v.get("guild_name").and_then(|n| n.as_str()).map(String::from));

    let rows = sqlx::query_as::<_, (
        String,
        String,
        String,
        bool,
        bool,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
        i32,
        sqlx::types::Json<serde_json::Value>,
        i64,
        Option<chrono::DateTime<chrono::Utc>>,
        String,
    )>(
        "SELECT g.universe_id, g.display_name, g.mode, g.push_enabled, g.pull_enabled, g.datastore_name, \
         g.last_push_at, g.last_pull_at, g.poll_interval_seconds, g.stat_field_map, \
         COALESCE(s.players_count, 0) AS players_count, s.last_fetched_at, g.entry_key_template \
         FROM game_universes g \
         LEFT JOIN ( \
             SELECT universe_id, COUNT(*)::bigint AS players_count, MAX(fetched_at) AS last_fetched_at \
             FROM player_game_stats GROUP BY universe_id \
         ) s ON s.universe_id = g.universe_id \
         WHERE g.owner_discord_id = $1 AND g.guild_id = $2 ORDER BY g.created_at DESC",
    )
    .bind(&discord_id)
    .bind(&guild_id)
    .fetch_all(&state.pool)
    .await?;

    let universes: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "universe_id": r.0,
                "display_name": r.1,
                "mode": r.2,
                "push_enabled": r.3,
                "pull_enabled": r.4,
                "datastore_name": r.5,
                "last_push_at": r.6,
                "last_pull_at": r.7,
                "poll_interval_seconds": r.8,
                "stat_field_map": r.9.0,
                "players_count": r.10,
                "last_stat_fetched_at": r.11,
                "entry_key_template": r.12,
            })
        })
        .collect();

    Ok(Json(json!({
        "universes": universes,
        "guild_id": guild_id,
        "guild_name": guild_name,
    })))
}

#[derive(Deserialize)]
pub struct CreateBody {
    pub universe_id: String,
    pub display_name: String,
    /// Open Cloud API key with read access to the universe. Used to prove that
    /// the registrant actually owns the Roblox game — without this check
    /// anyone could squat on another universe's ID and corrupt its stats.
    pub open_cloud_api_key: String,
    /// "push" (Studio plugin / HttpService) or "pull" (Open Cloud DataStore polling).
    /// Mutually exclusive — push and pull writing the same fields would race.
    pub mode: String,
}

pub async fn create_universe(
    State(state): State<Arc<AppState>>,
    Path(guild_id): Path<String>,
    jar: CookieJar,
    Json(body): Json<CreateBody>,
) -> Result<Json<Value>, AppError> {
    let discord_id = require_manager(&state, &jar, &guild_id).await?;

    if !body.universe_id.chars().all(|c| c.is_ascii_digit()) || body.universe_id.is_empty() {
        return Err(AppError::BadRequest("universe_id must be numeric".into()));
    }
    if body.display_name.trim().is_empty() {
        return Err(AppError::BadRequest("display_name is required".into()));
    }
    let mode = body.mode.trim();
    if mode != "push" && mode != "pull" {
        return Err(AppError::BadRequest("mode must be 'push' or 'pull'".into()));
    }
    let key = body.open_cloud_api_key.trim();
    if key.is_empty() {
        return Err(AppError::BadRequest(
            "Open Cloud API key is required to prove ownership of this universe.".into(),
        ));
    }

    // Ownership proof: hit a Roblox list endpoint that requires per-universe
    // scope and returns 200 only if the key's scope covers THIS universe AND
    // the universe actually exists. 404 is rejected (it's ambiguous between
    // "wrong universe" and "scope missing"), so a fake universe_id can't be
    // squat-registered.
    let oc = OpenCloudClient::new(state.config.open_cloud_rate_limit);
    if let Err(e) = oc.verify_universe_ownership(&body.universe_id, key).await {
        tracing::info!(universe_id = %body.universe_id, "Ownership verify failed: {e}");
        return Err(AppError::BadRequest(
            "Open Cloud key cannot access this universe. In the Roblox dashboard, the key must have Restrict by Experience ON for THIS universe, with the universe-datastores API System added and the controls.list operation ticked. (objects.read alone is not accepted — it returns 404 even for universes you don't own, so we can't use it to prove ownership.)".into(),
        ));
    }

    // For pull mode, also store the key encrypted so the user doesn't have to
    // paste it again to configure the DataStore polling. Pull won't actually
    // run until the user picks a DataStore name + field map. For push mode the
    // key is discarded after the ownership probe.
    let key_encrypted: Option<String> = if mode == "pull" {
        Some(crate::services::crypto::encrypt(&state.encryption_key, key)?)
    } else {
        None
    };

    let secret = random_secret();
    let push_enabled = mode == "push";
    // pull_enabled only flips true once the DataStore name is configured.
    let pull_enabled = false;

    let inserted = sqlx::query(
        "INSERT INTO game_universes \
            (universe_id, display_name, owner_discord_id, ingest_secret, guild_id, \
             mode, push_enabled, pull_enabled, open_cloud_api_key_encrypted) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         ON CONFLICT (universe_id, guild_id) WHERE guild_id IS NOT NULL DO NOTHING",
    )
    .bind(&body.universe_id)
    .bind(body.display_name.trim())
    .bind(&discord_id)
    .bind(&secret)
    .bind(&guild_id)
    .bind(mode)
    .bind(push_enabled)
    .bind(pull_enabled)
    .bind(&key_encrypted)
    .execute(&state.pool)
    .await?;

    if inserted.rows_affected() == 0 {
        return Err(AppError::Conflict(
            "This universe is already registered for this Discord server.".into(),
        ));
    }

    let response = json!({
        "success": true,
        "universe_id": body.universe_id,
        "mode": mode,
    });
    Ok(Json(response))
}

pub async fn regenerate_secret(
    State(state): State<Arc<AppState>>,
    Path((guild_id, universe_id)): Path<(String, String)>,
    jar: CookieJar,
) -> Result<Json<Value>, AppError> {
    let discord_id = require_manager(&state, &jar, &guild_id).await?;
    let secret = random_secret();
    let r = sqlx::query(
        "UPDATE game_universes SET ingest_secret = $1 \
         WHERE universe_id = $2 AND owner_discord_id = $3 AND guild_id = $4 AND mode = 'push'",
    )
    .bind(&secret)
    .bind(&universe_id)
    .bind(&discord_id)
    .bind(&guild_id)
    .execute(&state.pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(AppError::BadRequest(
            "Universe not found, or it's registered in pull mode (no ingest secret to rotate).".into(),
        ));
    }
    Ok(Json(json!({"success": true, "ingest_secret": secret})))
}

#[derive(Deserialize)]
pub struct OpenCloudBody {
    pub open_cloud_api_key: String,
    pub datastore_name: String,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_seconds: i32,
    #[serde(default)]
    pub stat_field_map: serde_json::Value,
    /// Template for the DataStore entry key, with `{user_id}` substituted per
    /// linked Roblox user (e.g. `Player_{user_id}`). Defaults to `{user_id}`
    /// (bare user id) when omitted or empty.
    #[serde(default)]
    pub entry_key_template: Option<String>,
}

fn default_poll_interval() -> i32 {
    600
}

pub async fn save_open_cloud(
    State(state): State<Arc<AppState>>,
    Path((guild_id, universe_id)): Path<(String, String)>,
    jar: CookieJar,
    Json(body): Json<OpenCloudBody>,
) -> Result<Json<Value>, AppError> {
    let discord_id = require_manager(&state, &jar, &guild_id).await?;

    let mode: Option<(String,)> = sqlx::query_as(
        "SELECT mode FROM game_universes \
         WHERE universe_id = $1 AND owner_discord_id = $2 AND guild_id = $3",
    )
    .bind(&universe_id)
    .bind(&discord_id)
    .bind(&guild_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((mode,)) = mode else {
        return Err(AppError::NotFound("Universe not found".into()));
    };
    if mode != "pull" {
        return Err(AppError::BadRequest(
            "This universe is registered in push mode. Pull configuration isn't available — delete and re-register as pull mode if you want to switch.".into(),
        ));
    }

    let key_input = body.open_cloud_api_key.trim();
    let datastore_input = body.datastore_name.trim();

    // Empty key + empty DataStore → pause polling (keeps saved key in case the
    // user wants to resume later by re-saving the DataStore name).
    if key_input.is_empty() && datastore_input.is_empty() {
        let r = sqlx::query(
            "UPDATE game_universes SET pull_enabled = FALSE \
             WHERE universe_id = $1 AND owner_discord_id = $2 AND guild_id = $3",
        )
        .bind(&universe_id)
        .bind(&discord_id)
        .bind(&guild_id)
        .execute(&state.pool)
        .await?;
        if r.rows_affected() == 0 {
            return Err(AppError::NotFound("Universe not found".into()));
        }
        return Ok(Json(json!({"success": true, "pull_enabled": false})));
    }

    if datastore_input.is_empty() {
        return Err(AppError::BadRequest("DataStore name is required to start polling.".into()));
    }
    let interval = body.poll_interval_seconds.clamp(60, 86400);

    // If the user provided a new key, verify it and re-encrypt. Otherwise keep
    // the saved key from registration time.
    let key_encrypted_update: Option<String> = if !key_input.is_empty() {
        let oc = OpenCloudClient::new(state.config.open_cloud_rate_limit);
        if oc.verify_universe_ownership(&universe_id, key_input).await.is_err() {
            return Err(AppError::BadRequest(
                "New Open Cloud key cannot read this universe — in the Roblox dashboard, add the universe-datastores API System with the controls.list operation, Restrict by Experience ON for this universe.".into(),
            ));
        }
        Some(crate::services::crypto::encrypt(&state.encryption_key, key_input)?)
    } else {
        None
    };

    let map: serde_json::Map<String, serde_json::Value> = body
        .stat_field_map
        .as_object()
        .cloned()
        .unwrap_or_default();

    // Default to bare-userId keys if the admin didn't pick a template. Reject
    // templates that don't contain `{user_id}` — they'd resolve to the same
    // key for every player and overwrite each other.
    let template = body
        .entry_key_template
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("{user_id}")
        .to_string();
    if !template.contains("{user_id}") {
        return Err(AppError::BadRequest(
            "Entry key template must contain the literal `{user_id}` placeholder (it gets replaced with each linked player's Roblox ID).".into(),
        ));
    }

    let r = sqlx::query(
        "UPDATE game_universes SET \
            open_cloud_api_key_encrypted = COALESCE($1, open_cloud_api_key_encrypted), \
            datastore_name = $2, poll_interval_seconds = $3, stat_field_map = $4, pull_enabled = TRUE, \
            entry_key_template = $8 \
         WHERE universe_id = $5 AND owner_discord_id = $6 AND guild_id = $7 \
           AND (open_cloud_api_key_encrypted IS NOT NULL OR $1 IS NOT NULL)",
    )
    .bind(&key_encrypted_update)
    .bind(datastore_input)
    .bind(interval)
    .bind(sqlx::types::Json(serde_json::Value::Object(map)))
    .bind(&universe_id)
    .bind(&discord_id)
    .bind(&guild_id)
    .bind(&template)
    .execute(&state.pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(AppError::BadRequest(
            "No saved Open Cloud key found — paste the key again to start polling.".into(),
        ));
    }

    Ok(Json(json!({"success": true, "pull_enabled": true})))
}

/// Decrypt and return the saved Open Cloud key for a pull-mode universe.
/// Returns NotFound if no key is saved or the universe is push-mode.
async fn load_saved_key(
    state: &Arc<AppState>,
    universe_id: &str,
    discord_id: &str,
    guild_id: &str,
) -> Result<String, AppError> {
    let row: Option<(Option<String>, String)> = sqlx::query_as(
        "SELECT open_cloud_api_key_encrypted, mode FROM game_universes \
         WHERE universe_id = $1 AND owner_discord_id = $2 AND guild_id = $3",
    )
    .bind(universe_id)
    .bind(discord_id)
    .bind(guild_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((enc, mode)) = row else {
        return Err(AppError::NotFound("Universe not found".into()));
    };
    if mode != "pull" {
        return Err(AppError::BadRequest(
            "DataStore discovery is only available for pull-mode universes.".into(),
        ));
    }
    let Some(enc) = enc else {
        return Err(AppError::BadRequest(
            "No saved Open Cloud key for this universe — re-register or paste a key in the DataStore form.".into(),
        ));
    };
    crate::services::crypto::decrypt(&state.encryption_key, &enc)
}

/// GET /games/{guild_id}/{universe_id}/datastores
/// Lists DataStore names in the universe via Open Cloud, using the key saved
/// at registration time. Used to populate the DataStore-name dropdown on the
/// pull-mode config form.
pub async fn list_datastores(
    State(state): State<Arc<AppState>>,
    Path((guild_id, universe_id)): Path<(String, String)>,
    jar: CookieJar,
) -> Result<Json<Value>, AppError> {
    let discord_id = require_manager(&state, &jar, &guild_id).await?;
    let key = load_saved_key(&state, &universe_id, &discord_id, &guild_id).await?;
    let oc = OpenCloudClient::new(state.config.open_cloud_rate_limit);
    let names = oc.list_datastores(&universe_id, &key).await.map_err(|e| {
        AppError::BadRequest(format!(
            "Could not list DataStores. Make sure the Open Cloud key has the universe-datastores API System with controls.list ticked, scoped to this universe. ({e})"
        ))
    })?;
    Ok(Json(json!({"datastores": names})))
}

#[derive(Deserialize)]
pub struct SampleQuery {
    pub datastore_name: String,
}

/// GET /games/{guild_id}/{universe_id}/sample-entry?datastore_name=...
/// Fetches a sample entry from the DataStore so the admin can see what JSON
/// paths exist. Used to scaffold the stat_field_map.
pub async fn sample_datastore_entry(
    State(state): State<Arc<AppState>>,
    Path((guild_id, universe_id)): Path<(String, String)>,
    jar: CookieJar,
    axum::extract::Query(q): axum::extract::Query<SampleQuery>,
) -> Result<Json<Value>, AppError> {
    let discord_id = require_manager(&state, &jar, &guild_id).await?;
    let key = load_saved_key(&state, &universe_id, &discord_id, &guild_id).await?;
    if q.datastore_name.trim().is_empty() {
        return Err(AppError::BadRequest("datastore_name is required".into()));
    }
    let oc = OpenCloudClient::new(state.config.open_cloud_rate_limit);
    let ids = oc
        .list_entry_ids(&universe_id, q.datastore_name.trim(), &key, 1)
        .await
        .map_err(|e| {
            AppError::BadRequest(format!(
                "Could not list entries (key needs universe-datastores objects.list + objects.read): {e}"
            ))
        })?;
    let Some(first_id) = ids.into_iter().next() else {
        return Ok(Json(json!({"entry_id": null, "value": null, "note": "DataStore is empty"})));
    };
    let value = oc
        .read_datastore_entry(&universe_id, q.datastore_name.trim(), &first_id, &key)
        .await
        .map_err(|e| AppError::BadRequest(format!("Could not read sample entry: {e}")))?;
    Ok(Json(json!({"entry_id": first_id, "value": value})))
}

pub async fn delete_universe(
    State(state): State<Arc<AppState>>,
    Path((guild_id, universe_id)): Path<(String, String)>,
    jar: CookieJar,
) -> Result<Json<Value>, AppError> {
    let discord_id = require_manager(&state, &jar, &guild_id).await?;
    let mut tx = state.pool.begin().await?;
    let r = sqlx::query(
        "DELETE FROM game_universes \
         WHERE universe_id = $1 AND owner_discord_id = $2 AND guild_id = $3",
    )
    .bind(&universe_id)
    .bind(&discord_id)
    .bind(&guild_id)
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() == 0 {
        return Err(AppError::NotFound("Universe not found".into()));
    }
    // If no other guild has this universe registered, purge cached stats too.
    sqlx::query(
        "DELETE FROM player_game_stats WHERE universe_id = $1 \
         AND NOT EXISTS (SELECT 1 FROM game_universes WHERE universe_id = $1)",
    )
    .bind(&universe_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"success": true})))
}
