use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::services::roblox_oauth;
use crate::services::session;
use crate::services::sync::PlayerSyncEvent;
use crate::AppState;

const SESSION_COOKIE: &str = "rl_session";

fn get_session(jar: &CookieJar, secret: &str) -> Result<(String, String), AppError> {
    let cookie = jar.get(SESSION_COOKIE).ok_or(AppError::Unauthorized)?;
    session::verify_session(cookie.value(), secret).ok_or(AppError::Unauthorized)
}

pub fn render_verify_page(base_url: &str) -> String {
    let login_url = format!("{base_url}/verify/login");
    format!(
        r##"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Roblox Player Roles - Link Account</title>
    <link rel="icon" href="{base_url}/favicon.ico" type="image/x-icon">
    <meta name="description" content="Link your Discord account with your Roblox profile to automatically receive server roles based on your Roblox data.">
    <meta name="theme-color" content="#393b3d">
    <style>
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{ font-family: system-ui, -apple-system, sans-serif; max-width: 580px; margin: 0 auto; padding: 32px 20px; background: #232527; color: #ebedf0; min-height: 100vh; }}
        h1 {{ color: #00a2ff; font-size: 24px; margin-bottom: 4px; }}
        h2 {{ color: #fff; font-size: 17px; margin-bottom: 14px; }}
        p {{ line-height: 1.6; margin: 6px 0; font-size: 14px; color: #b8bcc1; }}
        a {{ color: #00a2ff; }}
        .subtitle {{ color: #8a9099; font-size: 14px; margin-bottom: 20px; }}
        .card {{ background: #2e3133; padding: 22px; border-radius: 10px; margin: 14px 0; border: 1px solid #3d4144; }}
        .btn {{ display: inline-flex; align-items: center; gap: 8px; padding: 10px 22px; color: #fff; text-decoration: none; border-radius: 6px; font-size: 14px; font-weight: 500; border: none; cursor: pointer; font-family: inherit; transition: background .15s; }}
        .btn-discord {{ background: #5865f2; }}
        .btn-discord:hover {{ background: #4752c4; }}
        .btn-roblox {{ background: #e2231a; }}
        .btn-roblox:hover {{ background: #b71c14; }}
        .btn-danger {{ background: transparent; color: #f87171; border: 1px solid #7f1d1d; font-size: 13px; padding: 8px 16px; }}
        .btn-danger:hover {{ background: #7f1d1d33; }}
        .badge {{ display: inline-block; padding: 3px 10px; border-radius: 20px; font-size: 12px; font-weight: 500; }}
        .badge-ok {{ background: #052e16; color: #4ade80; border: 1px solid #14532d; }}
        .msg {{ padding: 10px 14px; border-radius: 6px; margin: 12px 0; font-size: 13px; line-height: 1.5; }}
        .msg-error {{ background: #1c0a0a; color: #fca5a5; border: 1px solid #7f1d1d; }}
        .msg-success {{ background: #052e16; color: #86efac; border: 1px solid #14532d; }}
        .info-row {{ display: flex; align-items: center; gap: 8px; margin: 6px 0; font-size: 14px; }}
        .info-row .label {{ color: #8a9099; min-width: 80px; }}
        .info-row .val {{ color: #00a2ff; font-weight: 600; }}
        .actions {{ display: flex; gap: 8px; margin-top: 16px; flex-wrap: wrap; }}
        .hidden {{ display: none !important; }}
        .divider {{ border: none; border-top: 1px solid #3d4144; margin: 16px 0; }}
        .trust-note {{ font-size: 13px; color: #a1a7ad; background: #232527; border-left: 3px solid #00a2ff; padding: 10px 14px; border-radius: 0 6px 6px 0; margin: 10px 0; line-height: 1.6; }}
        .trust-note strong {{ color: #ebedf0; }}
        .btn-logout {{ background: transparent; color: #a1a7ad; border: 1px solid #3d4144; padding: 5px 12px; border-radius: 6px; font-size: 12px; cursor: pointer; font-family: inherit; transition: all .15s; }}
        .btn-logout:hover {{ color: #f87171; border-color: #7f1d1d; background: #7f1d1d22; }}
    </style>
</head>
<body>
    <div style="display:flex; align-items:center; justify-content:space-between; margin-bottom:4px;">
        <div style="display:flex; align-items:center; gap:10px;">
            <h1 style="margin:0;">Roblox Player Roles</h1>
            <span style="font-size:11px; color:#8a9099; background:#232527; padding:2px 8px; border-radius:4px;">Powered by <a href="https://rolelogic.faizo.net" target="_blank" rel="noopener" style="color:#00a2ff; text-decoration:none;">RoleLogic</a></span>
        </div>
        <button id="logout-btn" class="btn-logout hidden" onclick="doLogout()">Logout</button>
    </div>
    <p class="subtitle">Link your Discord account with your Roblox profile to automatically receive server roles.</p>

    <div id="loading-section" class="card"><p style="color:#8a9099;">Loading...</p></div>

    <div id="login-section" class="card hidden">
        <h2>Step 1: Sign in with Discord</h2>
        <p>Sign in so we know which Discord account to assign roles to.</p>
        <p class="trust-note">We request the <strong>identify</strong> and <strong>guilds</strong> scopes — we cannot read your messages, join servers, or access anything else on your account.</p>
        <div class="actions">
            <a href="{login_url}" class="btn btn-discord">Login with Discord</a>
        </div>
    </div>

    <div id="linked-section" class="card hidden">
        <div style="display:flex; align-items:center; gap:10px; margin-bottom:14px;">
            <h2 style="margin:0;">Account Linked</h2>
            <span class="badge badge-ok">Verified</span>
        </div>
        <div class="info-row"><span class="label">Roblox</span> <span class="val" id="linked-roblox"></span></div>
        <div class="info-row"><span class="label">Discord</span> <span class="val" id="linked-discord" style="color:#a1a7ad;font-weight:400;font-size:13px;"></span></div>
        <p style="color:#4ade80; margin-top:12px; font-size:13px;">Your roles are assigned automatically based on your Roblox data.</p>
        <hr class="divider">
        <div class="actions">
            <button class="btn btn-danger" onclick="doUnlink()">Unlink Account</button>
        </div>
    </div>

    <div id="roblox-section" class="card hidden">
        <h2>Step 2: Link Your Roblox Account</h2>
        <p>Signed in as <span id="roblox-discord" style="color:#00a2ff;"></span></p>
        <p style="margin-bottom:12px;">Click below to sign in with Roblox. You'll be redirected to Roblox's official login page and then back here.</p>
        <p class="trust-note">Roblox uses OAuth 2.0 — we never see your Roblox password. We only receive your public Roblox ID, username, and (with your permission) account creation date and Premium status.</p>
        <div class="actions">
            <a href="{base_url}/verify/roblox" class="btn btn-roblox">Login with Roblox</a>
        </div>
    </div>

    <div id="msg" class="hidden"></div>

    <noscript><p style="color:#f87171; margin-top:20px;">JavaScript is required.</p></noscript>

    <script>
    const API = '{base_url}';
    async function api(method, path, body) {{
        const opts = {{ method, headers: {{}}, credentials: 'include' }};
        if (body) {{ opts.headers['Content-Type'] = 'application/json'; opts.body = JSON.stringify(body); }}
        const res = await fetch(API + path, opts);
        const data = await res.json();
        if (!res.ok) throw new Error(data.error || 'Request failed');
        return data;
    }}
    function showSection(id) {{
        ['loading-section','login-section','linked-section','roblox-section'].forEach(s =>
            document.getElementById(s).classList.add('hidden'));
        document.getElementById(id).classList.remove('hidden');
    }}
    function showMsg(text, type) {{
        const el = document.getElementById('msg');
        el.className = 'msg msg-' + type;
        el.textContent = text;
        el.classList.remove('hidden');
        if (type === 'success') setTimeout(() => el.classList.add('hidden'), 6000);
    }}
    function clearMsg() {{ document.getElementById('msg').classList.add('hidden'); }}
    let currentName = '';
    async function init() {{
        try {{
            const s = await api('GET', '/verify/status');
            currentName = s.display_name || '';
            document.getElementById('logout-btn').classList.remove('hidden');
            if (s.linked) {{
                document.getElementById('linked-roblox').textContent = s.roblox_username || s.linked;
                document.getElementById('linked-discord').textContent = s.display_name;
                showSection('linked-section');
            }} else {{
                document.getElementById('roblox-discord').textContent = s.display_name;
                showSection('roblox-section');
            }}
        }} catch (e) {{ showSection('login-section'); }}
    }}
    async function doLogout() {{
        clearMsg();
        try {{
            await api('POST', '/verify/logout');
            document.getElementById('logout-btn').classList.add('hidden');
            showSection('login-section');
            showMsg('Logged out.', 'success');
        }} catch (e) {{ showMsg(e.message, 'error'); }}
    }}
    async function doUnlink() {{
        clearMsg();
        if (!confirm('Unlink your Roblox account? You will lose all assigned roles.')) return;
        try {{
            await api('POST', '/verify/unlink');
            document.getElementById('roblox-discord').textContent = currentName;
            showSection('roblox-section');
            showMsg('Account unlinked.', 'success');
        }} catch (e) {{ showMsg(e.message, 'error'); }}
    }}
    const params = new URLSearchParams(window.location.search);
    if (params.get('linked') === 'true') {{
        window.history.replaceState({{}}, '', window.location.pathname);
    }} else if (params.get('error')) {{
        const err = params.get('error');
        window.history.replaceState({{}}, '', window.location.pathname);
        setTimeout(() => showMsg(decodeURIComponent(err), 'error'), 100);
    }}
    init();
    </script>
</body>
</html>"##
    )
}

pub async fn verify_page(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        state.verify_html.clone(),
    )
}

pub async fn login(State(_state): State<Arc<AppState>>) -> Response {
    let return_to = "/roblox-player-role/verify";
    let url = format!("/auth/login?return_to={}", urlencoding::encode(return_to));
    Redirect::temporary(&url).into_response()
}

pub async fn status(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<Value>, AppError> {
    let (discord_id, display_name) = get_session(&jar, &state.config.session_secret)?;

    let account = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT roblox_user_id, roblox_username FROM linked_accounts WHERE discord_id = $1",
    )
    .bind(&discord_id)
    .fetch_optional(&state.pool)
    .await?;

    Ok(Json(json!({
        "discord_id": discord_id,
        "display_name": display_name,
        "linked": account.as_ref().map(|a| &a.0),
        "roblox_username": account.as_ref().and_then(|a| a.1.as_ref()),
    })))
}

/// Begin Roblox OAuth — generate PKCE pair, store in verification_sessions, redirect.
pub async fn roblox_login(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Response, AppError> {
    let (discord_id, _) = get_session(&jar, &state.config.session_secret)?;

    let existing = sqlx::query_scalar::<_, String>(
        "SELECT roblox_user_id FROM linked_accounts WHERE discord_id = $1",
    )
    .bind(&discord_id)
    .fetch_optional(&state.pool)
    .await?;

    if existing.is_some() {
        return Err(AppError::BadRequest(
            "You already have a linked Roblox account. Unlink it first.".into(),
        ));
    }

    let pkce = roblox_oauth::generate_pkce();
    let oauth_state = roblox_oauth::generate_state();
    let expires = chrono::Utc::now() + chrono::Duration::minutes(15);

    sqlx::query("DELETE FROM verification_sessions WHERE discord_id = $1")
        .bind(&discord_id)
        .execute(&state.pool)
        .await?;

    sqlx::query(
        "INSERT INTO verification_sessions (discord_id, state, code_verifier, expires_at) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(&discord_id)
    .bind(&oauth_state)
    .bind(&pkce.verifier)
    .bind(expires)
    .execute(&state.pool)
    .await?;

    let url = state.roblox_oauth.build_authorize_url(&oauth_state, &pkce.challenge);
    Ok(Redirect::temporary(&url).into_response())
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

pub async fn callback(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Query(query): Query<CallbackQuery>,
) -> Result<Response, AppError> {
    let (discord_id, _) = get_session(&jar, &state.config.session_secret)?;

    if let Some(err) = query.error {
        let desc = query.error_description.unwrap_or_else(|| err.clone());
        let url = format!(
            "{}/verify?error={}",
            state.config.base_url,
            urlencoding::encode(&format!("Roblox sign-in failed: {desc}"))
        );
        return Ok(Redirect::temporary(&url).into_response());
    }

    let code = query.code.ok_or_else(|| AppError::VerificationFailed("Missing code".into()))?;
    let oauth_state = query
        .state
        .ok_or_else(|| AppError::VerificationFailed("Missing state".into()))?;

    let session_row = sqlx::query_as::<_, (String,)>(
        "SELECT code_verifier FROM verification_sessions \
         WHERE discord_id = $1 AND state = $2 AND expires_at > now()",
    )
    .bind(&discord_id)
    .bind(&oauth_state)
    .fetch_optional(&state.pool)
    .await?;

    let Some((code_verifier,)) = session_row else {
        let url = format!(
            "{}/verify?error={}",
            state.config.base_url,
            urlencoding::encode("Verification session expired or state mismatch. Please try again.")
        );
        return Ok(Redirect::temporary(&url).into_response());
    };

    // Exchange code for tokens
    let tokens = state.roblox_oauth.exchange_code(&code, &code_verifier).await?;
    let userinfo = state.roblox_oauth.userinfo(&tokens.access_token).await?;

    let roblox_user_id = userinfo.sub.clone();
    let username = userinfo.preferred_username.or(userinfo.nickname.clone());
    let display_name = userinfo.nickname.or(userinfo.name.clone());

    // Conflict check — Roblox account already taken?
    let taken = sqlx::query_scalar::<_, String>(
        "SELECT discord_id FROM linked_accounts WHERE roblox_user_id = $1",
    )
    .bind(&roblox_user_id)
    .fetch_optional(&state.pool)
    .await?;
    if let Some(other) = taken {
        if other != discord_id {
            let url = format!(
                "{}/verify?error={}",
                state.config.base_url,
                urlencoding::encode("This Roblox account is already linked to another Discord user.")
            );
            return Ok(Redirect::temporary(&url).into_response());
        }
    }

    // Encrypt the refresh token before storage. Roblox rotates refresh tokens —
    // always store the latest one returned.
    let refresh_encrypted = match tokens.refresh_token.as_deref() {
        Some(rt) => Some(crate::services::crypto::encrypt(&state.encryption_key, rt)?),
        None => None,
    };

    let account_created = userinfo
        .created_at
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0));

    sqlx::query(
        "INSERT INTO linked_accounts (discord_id, roblox_user_id, roblox_username, roblox_display_name, refresh_token_encrypted) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (discord_id) DO UPDATE SET \
            roblox_user_id = EXCLUDED.roblox_user_id, \
            roblox_username = EXCLUDED.roblox_username, \
            roblox_display_name = EXCLUDED.roblox_display_name, \
            refresh_token_encrypted = EXCLUDED.refresh_token_encrypted, \
            linked_at = now()",
    )
    .bind(&discord_id)
    .bind(&roblox_user_id)
    .bind(&username)
    .bind(&display_name)
    .bind(&refresh_encrypted)
    .execute(&state.pool)
    .await?;

    // Seed user_cache so refresh worker picks it up immediately
    sqlx::query(
        "INSERT INTO user_cache (roblox_user_id, username, display_name, account_created, next_fetch_at) \
         VALUES ($1, $2, $3, $4, now()) \
         ON CONFLICT (roblox_user_id) DO UPDATE SET \
            username = EXCLUDED.username, \
            display_name = EXCLUDED.display_name, \
            account_created = COALESCE(user_cache.account_created, EXCLUDED.account_created), \
            next_fetch_at = LEAST(user_cache.next_fetch_at, now())",
    )
    .bind(&roblox_user_id)
    .bind(&username)
    .bind(&display_name)
    .bind(&account_created)
    .execute(&state.pool)
    .await?;

    sqlx::query("DELETE FROM verification_sessions WHERE discord_id = $1")
        .bind(&discord_id)
        .execute(&state.pool)
        .await?;

    let _ = state
        .player_sync_tx
        .try_send(PlayerSyncEvent::AccountLinked {
            discord_id: discord_id.clone(),
        });

    tracing::info!(discord_id, roblox_user_id, "Roblox account linked");

    let url = format!("{}/verify?linked=true", state.config.base_url);
    Ok(Redirect::temporary(&url).into_response())
}

pub async fn logout(jar: CookieJar) -> (CookieJar, Json<Value>) {
    let cookie = Cookie::build(SESSION_COOKIE).path("/");
    let jar = jar.remove(cookie);
    (jar, Json(json!({"success": true})))
}

pub async fn unlink(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<Value>, AppError> {
    let (discord_id, _) = get_session(&jar, &state.config.session_secret)?;

    let account = sqlx::query_as::<_, (String,)>(
        "SELECT roblox_user_id FROM linked_accounts WHERE discord_id = $1",
    )
    .bind(&discord_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound("No linked account found".into()))?;

    sqlx::query("DELETE FROM linked_accounts WHERE discord_id = $1")
        .bind(&discord_id)
        .execute(&state.pool)
        .await?;

    let _ = state
        .player_sync_tx
        .try_send(PlayerSyncEvent::AccountUnlinked {
            discord_id: discord_id.clone(),
        });

    tracing::info!(discord_id, roblox_user_id = account.0, "Roblox account unlinked");
    Ok(Json(json!({"success": true})))
}
