//! Game-creator admin UI under `/games`. Cookie-authed via Auth Gateway session.
//!
//! Endpoints:
//!   GET  /games                          — HTML dashboard for the signed-in dev's universes
//!   GET  /games/data                     — JSON list of universes for the signed-in dev
//!   POST /games                          — register a new universe (returns ingest_secret once)
//!   POST /games/{universe_id}/regenerate-secret  — rotate ingest_secret
//!   POST /games/{universe_id}/open-cloud — set/clear Open Cloud key + DataStore mapping
//!   POST /games/{universe_id}/delete     — delete the universe (cascades player_game_stats)

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
        .msg {{ padding: 10px 14px; border-radius: 6px; font-size: 13px; }}
        .msg-error {{ background: #1c0a0a; color: #fca5a5; border: 1px solid #7f1d1d; }}
        .msg-success {{ background: #052e16; color: #86efac; border: 1px solid #14532d; }}
        .hidden {{ display: none; }}
        .badge-on {{ display: inline-block; padding: 2px 8px; background: #052e16; color: #4ade80; border: 1px solid #14532d; border-radius: 12px; font-size: 11px; }}
        .badge-off {{ display: inline-block; padding: 2px 8px; background: #1c0a0a; color: #fca5a5; border: 1px solid #7f1d1d; border-radius: 12px; font-size: 11px; }}
    </style>
</head>
<body>
    <h1>Game Integrations</h1>
    <p>Connect your Roblox game so members of your Discord get roles automatically based on their in-game progress.</p>
    <p style="margin-top:6px;"><a href="{base_url}/verify">← Player verification page</a></p>

    <div id="msg" class="hidden"></div>

    <div id="loading" class="card"><p>Loading...</p></div>

    <div id="login-prompt" class="card hidden">
        <p>You're not signed in. <a id="login-link" href="#">Login with Discord</a> to manage your games.</p>
    </div>

    <div id="content" class="hidden">
        <div class="card">
            <h2>Register a new Roblox game</h2>
            <p>You'll need your Roblox <strong>Universe ID</strong> (find it in <a href="https://create.roblox.com/dashboard/creations" target="_blank">Creator Dashboard</a> → game settings).</p>
            <div class="row">
                <input type="text" id="new-universe-id" placeholder="Universe ID (numeric)">
                <input type="text" id="new-display-name" placeholder="Game name (for display)">
                <button class="btn" onclick="createUniverse()">Register</button>
            </div>
        </div>

        <h2>Your registered games</h2>
        <div id="universes"></div>
    </div>

    <script>
    const API = '{base_url}';
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
        return `
        <div class="card universe-card" id="u-${{esc(u.universe_id)}}">
            <h3>${{esc(u.display_name || 'Game ' + u.universe_id)}} <span class="${{u.push_enabled ? 'badge-on' : 'badge-off'}}">push ${{u.push_enabled ? 'on' : 'off'}}</span> <span class="${{u.pull_enabled ? 'badge-on' : 'badge-off'}}">pull ${{u.pull_enabled ? 'on' : 'off'}}</span></h3>
            <p><span class="label">Universe ID</span> <code>${{esc(u.universe_id)}}</code></p>
            <p><span class="label">Webhook URL</span></p>
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

            <details style="margin-top:14px;">
                <summary style="cursor:pointer; color:#00a2ff;">Open Cloud DataStore (optional pull-mode)</summary>
                <div style="margin-top:8px;">
                    <p>Paste an Open Cloud API key from <a href="https://create.roblox.com/dashboard/credentials" target="_blank">Roblox Creator Dashboard</a> with <em>DataStore Read</em> permission scoped to this universe. We'll poll the named DataStore on the schedule below.</p>
                    <div class="row"><input type="text" id="oc-key-${{esc(u.universe_id)}}" placeholder="Open Cloud API key (rbxop_...)"></div>
                    <div class="row"><input type="text" id="oc-ds-${{esc(u.universe_id)}}" placeholder="DataStore name (e.g. PlayerData)" value="${{esc(u.datastore_name || '')}}"></div>
                    <div class="row"><input type="number" id="oc-poll-${{esc(u.universe_id)}}" placeholder="Poll interval (seconds)" value="${{u.poll_interval_seconds || 600}}"></div>
                    <p style="font-size:12px;">Stat field map (JSON): map JSON paths in your DataStore entry to plugin field names.</p>
                    <textarea id="oc-map-${{esc(u.universe_id)}}" placeholder='&#123;"Stats.Level": "level", "Stats.Coins": "currency"&#125;'>${{esc(JSON.stringify(u.stat_field_map || {{}}, null, 2))}}</textarea>
                    <div class="row" style="margin-top:8px;">
                        <button class="btn" onclick="saveOpenCloud('${{esc(u.universe_id)}}')">Save Open Cloud config</button>
                        <button class="btn btn-danger" onclick="clearOpenCloud('${{esc(u.universe_id)}}')">Disable pull</button>
                    </div>
                </div>
            </details>

            <details style="margin-top:14px;" open>
                <summary style="cursor:pointer; color:#00a2ff;"><strong>Studio plugin install guide (recommended)</strong></summary>
                <div style="margin-top:10px;">
                    <p><strong>Step 1 — Download</strong></p>
                    <p style="margin:6px 0;">
                        <a class="btn" href="${{rbxmUrl}}" download>Download Roblox-Player-Role.rbxm</a>
                    </p>

                    <p style="margin-top:14px;"><strong>Step 2 — Install in Roblox Studio</strong></p>
                    <ol style="margin:6px 0 6px 20px; color:#b8bcc1;">
                        <li>Open your game's place file in <a href="https://create.roblox.com/dashboard/creations" target="_blank">Roblox Studio</a>.</li>
                        <li><strong>File → Open</strong> and pick the downloaded <code>Roblox-Player-Role.rbxm</code>. A <code>Roblox-Player-Role</code> Script appears at the top of the Explorer.</li>
                        <li>Drag that Script into <code>ServerScriptService</code>.</li>
                    </ol>

                    <p style="margin-top:14px;"><strong>Step 3 — Configure (paste your credentials)</strong></p>
                    <p style="margin:6px 0;">Run the place once (F5) so the script can create its config holder. A <code>Configuration</code> instance named <code>RoleLogicConfig</code> appears in <code>ServerScriptService</code> with two <code>StringValue</code> children. Stop the test, then paste:</p>
                    <ul style="margin:4px 0 4px 20px; color:#b8bcc1;">
                        <li><code>WebhookUrl</code> →</li>
                    </ul>
                    <div class="secret-box" style="margin:4px 0;">${{esc(ingestUrl)}}</div>
                    <ul style="margin:4px 0 4px 20px; color:#b8bcc1;">
                        <li><code>IngestSecret</code> → the secret shown when you registered (or rotate via <em>Show ingest secret &amp; rotate</em> above).</li>
                    </ul>

                    <p style="margin-top:14px;"><strong>Step 4 — Allow HTTP &amp; publish</strong></p>
                    <ol style="margin:6px 0 6px 20px; color:#b8bcc1;">
                        <li><strong>Game Settings → Security → Allow HTTP Requests = ON</strong>.</li>
                        <li><strong>File → Publish to Roblox</strong>. Within ~60s of a player joining the live game, stats start arriving here (the <em>push</em> badge above flips on).</li>
                    </ol>

                    <p style="margin-top:14px;"><strong>Step 5 — Map your stats (optional)</strong></p>
                    <p style="margin:6px 0;">By default the plugin uploads <code>leaderstats</code> values named <code>Level</code>, <code>Wins</code>, <code>Losses</code>, <code>Coins</code>. To upload other values (e.g. attributes, custom names), open the <code>Config</code> ModuleScript inside the Script and edit <code>StatPaths</code>. Custom keys prefixed <code>custom.</code> become <code>stat_key</code> values usable in role conditions.</p>

                    <p style="margin-top:10px; font-size:12px; color:#8a9099;">Troubleshooting: see Output in Studio for <code>[RoleLogic]</code> warnings. <code>WebhookUrl or IngestSecret not configured</code> means <code>RoleLogicConfig</code> values are still blank.</p>
                </div>
            </details>

            <details style="margin-top:14px;">
                <summary style="cursor:pointer; color:#00a2ff;">Alternative: paste your own server script (HttpService push)</summary>
                <p style="margin-top:8px; font-size:12px;">If you'd rather not install the plugin, drop this into a ServerScript yourself. Same wire format.</p>
                <pre>${{esc(luaSnippet)}}</pre>
            </details>

            <div style="margin-top:18px;">
                <button class="btn btn-danger" onclick="deleteUniverse('${{esc(u.universe_id)}}')">Delete this game</button>
            </div>
        </div>`;
    }}
    async function load() {{
        try {{
            const data = await api('GET', '/games/data');
            document.getElementById('loading').classList.add('hidden');
            document.getElementById('content').classList.remove('hidden');
            const c = document.getElementById('universes');
            c.innerHTML = data.universes.map(renderUniverse).join('') || '<p style="color:#8a9099;">No games yet — register one above.</p>';
        }} catch (e) {{
            document.getElementById('loading').classList.add('hidden');
            if (e.status === 401) {{ document.getElementById('login-prompt').classList.remove('hidden'); return; }}
            showMsg(e.message, 'error');
        }}
    }}
    async function createUniverse() {{
        const universe_id = document.getElementById('new-universe-id').value.trim();
        const display_name = document.getElementById('new-display-name').value.trim();
        if (!universe_id || !/^[0-9]+$/.test(universe_id)) return showMsg('Universe ID must be numeric', 'error');
        try {{
            const r = await api('POST', '/games', {{ universe_id, display_name }});
            showMsg('Registered. Ingest secret (copy now — shown only once): ' + r.ingest_secret, 'success');
            await load();
        }} catch (e) {{ showMsg(e.message, 'error'); }}
    }}
    async function rotateSecret(uid) {{
        if (!confirm('Rotate ingest secret for this universe? Existing scripts will stop working until updated.')) return;
        try {{
            const r = await api('POST', '/games/' + encodeURIComponent(uid) + '/regenerate-secret');
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
        let stat_field_map;
        try {{ stat_field_map = JSON.parse(document.getElementById('oc-map-' + uid).value || '{{}}'); }}
        catch (e) {{ return showMsg('Stat field map: invalid JSON', 'error'); }}
        try {{
            await api('POST', '/games/' + encodeURIComponent(uid) + '/open-cloud',
                {{ open_cloud_api_key: key, datastore_name, poll_interval_seconds, stat_field_map }});
            showMsg('Open Cloud config saved.', 'success');
            await load();
        }} catch (e) {{ showMsg(e.message, 'error'); }}
    }}
    async function clearOpenCloud(uid) {{
        if (!confirm('Disable Open Cloud pull for this universe?')) return;
        try {{
            await api('POST', '/games/' + encodeURIComponent(uid) + '/open-cloud',
                {{ open_cloud_api_key: '', datastore_name: '', stat_field_map: {{}} }});
            showMsg('Pull disabled.', 'success');
            await load();
        }} catch (e) {{ showMsg(e.message, 'error'); }}
    }}
    async function deleteUniverse(uid) {{
        if (!confirm('Delete this game? All cached in-game stats will be removed.')) return;
        try {{
            await api('POST', '/games/' + encodeURIComponent(uid) + '/delete');
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

pub async fn games_page(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        state.games_html.clone(),
    )
}

pub async fn games_data(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<Value>, AppError> {
    let (discord_id, _) = get_session(&jar, &state.config.session_secret)?;

    let rows = sqlx::query_as::<_, (
        String,
        String,
        bool,
        bool,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
        i32,
        sqlx::types::Json<serde_json::Value>,
    )>(
        "SELECT universe_id, display_name, push_enabled, pull_enabled, datastore_name, \
         last_push_at, last_pull_at, poll_interval_seconds, stat_field_map \
         FROM game_universes WHERE owner_discord_id = $1 ORDER BY created_at DESC",
    )
    .bind(&discord_id)
    .fetch_all(&state.pool)
    .await?;

    let universes: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "universe_id": r.0,
                "display_name": r.1,
                "push_enabled": r.2,
                "pull_enabled": r.3,
                "datastore_name": r.4,
                "last_push_at": r.5,
                "last_pull_at": r.6,
                "poll_interval_seconds": r.7,
                "stat_field_map": r.8.0,
            })
        })
        .collect();

    Ok(Json(json!({ "universes": universes })))
}

#[derive(Deserialize)]
pub struct CreateBody {
    pub universe_id: String,
    pub display_name: String,
}

pub async fn create_universe(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateBody>,
) -> Result<Json<Value>, AppError> {
    let (discord_id, _) = get_session(&jar, &state.config.session_secret)?;

    if !body.universe_id.chars().all(|c| c.is_ascii_digit()) || body.universe_id.is_empty() {
        return Err(AppError::BadRequest("universe_id must be numeric".into()));
    }
    if body.display_name.trim().is_empty() {
        return Err(AppError::BadRequest("display_name is required".into()));
    }

    let secret = random_secret();

    let inserted = sqlx::query(
        "INSERT INTO game_universes (universe_id, display_name, owner_discord_id, ingest_secret) \
         VALUES ($1, $2, $3, $4) ON CONFLICT (universe_id) DO NOTHING",
    )
    .bind(&body.universe_id)
    .bind(body.display_name.trim())
    .bind(&discord_id)
    .bind(&secret)
    .execute(&state.pool)
    .await?;

    if inserted.rows_affected() == 0 {
        return Err(AppError::Conflict(
            "Universe already registered (by you or another user)".into(),
        ));
    }

    Ok(Json(json!({
        "success": true,
        "universe_id": body.universe_id,
        "ingest_secret": secret,
    })))
}

pub async fn regenerate_secret(
    State(state): State<Arc<AppState>>,
    Path(universe_id): Path<String>,
    jar: CookieJar,
) -> Result<Json<Value>, AppError> {
    let (discord_id, _) = get_session(&jar, &state.config.session_secret)?;
    let secret = random_secret();
    let r = sqlx::query(
        "UPDATE game_universes SET ingest_secret = $1 WHERE universe_id = $2 AND owner_discord_id = $3",
    )
    .bind(&secret)
    .bind(&universe_id)
    .bind(&discord_id)
    .execute(&state.pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(AppError::NotFound("Universe not found".into()));
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
}

fn default_poll_interval() -> i32 {
    600
}

pub async fn save_open_cloud(
    State(state): State<Arc<AppState>>,
    Path(universe_id): Path<String>,
    jar: CookieJar,
    Json(body): Json<OpenCloudBody>,
) -> Result<Json<Value>, AppError> {
    let (discord_id, _) = get_session(&jar, &state.config.session_secret)?;

    if body.open_cloud_api_key.trim().is_empty() {
        // Disable pull
        let r = sqlx::query(
            "UPDATE game_universes SET open_cloud_api_key_encrypted = NULL, datastore_name = NULL, \
             stat_field_map = '{}', pull_enabled = FALSE \
             WHERE universe_id = $1 AND owner_discord_id = $2",
        )
        .bind(&universe_id)
        .bind(&discord_id)
        .execute(&state.pool)
        .await?;
        if r.rows_affected() == 0 {
            return Err(AppError::NotFound("Universe not found".into()));
        }
        return Ok(Json(json!({"success": true, "pull_enabled": false})));
    }

    if body.datastore_name.trim().is_empty() {
        return Err(AppError::BadRequest("datastore_name is required when an API key is set".into()));
    }
    let interval = body.poll_interval_seconds.clamp(60, 86400);

    // Verify the key actually owns the universe
    let oc = OpenCloudClient::new(state.config.open_cloud_rate_limit);
    let key_trimmed = body.open_cloud_api_key.trim().to_string();
    if oc.get_universe(&universe_id, &key_trimmed).await.is_err() {
        return Err(AppError::BadRequest(
            "Open Cloud API key check failed — make sure the key has DataStore Read permission scoped to this universe.".into(),
        ));
    }

    let encrypted = crate::services::crypto::encrypt(&state.encryption_key, &key_trimmed)?;

    let map: serde_json::Map<String, serde_json::Value> = body
        .stat_field_map
        .as_object()
        .cloned()
        .unwrap_or_default();

    let r = sqlx::query(
        "UPDATE game_universes SET open_cloud_api_key_encrypted = $1, datastore_name = $2, \
         poll_interval_seconds = $3, stat_field_map = $4, pull_enabled = TRUE \
         WHERE universe_id = $5 AND owner_discord_id = $6",
    )
    .bind(&encrypted)
    .bind(body.datastore_name.trim())
    .bind(interval)
    .bind(sqlx::types::Json(serde_json::Value::Object(map)))
    .bind(&universe_id)
    .bind(&discord_id)
    .execute(&state.pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(AppError::NotFound("Universe not found".into()));
    }

    Ok(Json(json!({"success": true, "pull_enabled": true})))
}

pub async fn delete_universe(
    State(state): State<Arc<AppState>>,
    Path(universe_id): Path<String>,
    jar: CookieJar,
) -> Result<Json<Value>, AppError> {
    let (discord_id, _) = get_session(&jar, &state.config.session_secret)?;
    let r = sqlx::query("DELETE FROM game_universes WHERE universe_id = $1 AND owner_discord_id = $2")
        .bind(&universe_id)
        .bind(&discord_id)
        .execute(&state.pool)
        .await?;
    if r.rows_affected() == 0 {
        return Err(AppError::NotFound("Universe not found".into()));
    }
    Ok(Json(json!({"success": true})))
}
