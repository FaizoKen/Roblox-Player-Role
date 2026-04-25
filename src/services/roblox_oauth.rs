//! Roblox OAuth 2.0 client. PKCE (S256) is required.
//! Endpoints: apis.roblox.com/oauth/v1/{authorize,token,userinfo}.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppError;

const AUTHORIZE_URL: &str = "https://apis.roblox.com/oauth/v1/authorize";
const TOKEN_URL: &str = "https://apis.roblox.com/oauth/v1/token";
const USERINFO_URL: &str = "https://apis.roblox.com/oauth/v1/userinfo";

pub const SCOPES: &str = "openid profile";

#[derive(Clone)]
pub struct RobloxOAuthClient {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    http: reqwest::Client,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub expires_in: i64,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UserInfo {
    pub sub: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub preferred_username: Option<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub picture: Option<String>,
}

pub struct PkcePair {
    pub verifier: String,
    pub challenge: String,
}

pub fn generate_pkce() -> PkcePair {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let verifier = B64.encode(bytes);
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = B64.encode(hasher.finalize());
    PkcePair { verifier, challenge }
}

pub fn generate_state() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    B64.encode(bytes)
}

impl RobloxOAuthClient {
    pub fn new(client_id: String, client_secret: String, redirect_uri: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("Failed to build HTTP client");
        Self {
            client_id,
            client_secret,
            redirect_uri,
            http,
        }
    }

    pub fn build_authorize_url(&self, state: &str, code_challenge: &str) -> String {
        let mut url = url_query_builder(AUTHORIZE_URL);
        url.append("client_id", &self.client_id);
        url.append("redirect_uri", &self.redirect_uri);
        url.append("scope", SCOPES);
        url.append("response_type", "code");
        url.append("state", state);
        url.append("code_challenge", code_challenge);
        url.append("code_challenge_method", "S256");
        url.finish()
    }

    pub async fn exchange_code(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<TokenResponse, AppError> {
        let params = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", code_verifier),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
        ];

        let resp = self
            .http
            .post(TOKEN_URL)
            .form(&params)
            .send()
            .await
            .map_err(|e| AppError::RobloxOAuth(format!("token request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::RobloxOAuth(format!(
                "token exchange returned {status}: {body}"
            )));
        }

        resp.json::<TokenResponse>()
            .await
            .map_err(|e| AppError::RobloxOAuth(format!("token parse: {e}")))
    }

    /// Use a refresh token to get a fresh access token.
    /// Roblox rotates refresh tokens — always store the returned `refresh_token`.
    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, AppError> {
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
        ];

        let resp = self
            .http
            .post(TOKEN_URL)
            .form(&params)
            .send()
            .await
            .map_err(|e| AppError::RobloxOAuth(format!("refresh request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::RobloxOAuth(format!(
                "refresh returned {status}: {body}"
            )));
        }

        resp.json::<TokenResponse>()
            .await
            .map_err(|e| AppError::RobloxOAuth(format!("refresh parse: {e}")))
    }

    pub async fn userinfo(&self, access_token: &str) -> Result<UserInfo, AppError> {
        let resp = self
            .http
            .get(USERINFO_URL)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| AppError::RobloxOAuth(format!("userinfo request: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::RobloxOAuth(format!(
                "userinfo returned {}",
                resp.status()
            )));
        }
        resp.json::<UserInfo>()
            .await
            .map_err(|e| AppError::RobloxOAuth(format!("userinfo parse: {e}")))
    }
}

struct UrlBuilder {
    base: String,
    first: bool,
}

fn url_query_builder(base: &str) -> UrlBuilder {
    UrlBuilder {
        base: base.to_string(),
        first: true,
    }
}

impl UrlBuilder {
    fn append(&mut self, k: &str, v: &str) {
        let sep = if self.first { '?' } else { '&' };
        self.first = false;
        self.base.push(sep);
        self.base.push_str(k);
        self.base.push('=');
        self.base.push_str(&urlencoding::encode(v));
    }

    fn finish(self) -> String {
        self.base
    }
}
