use std::env;

#[derive(Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub session_secret: String,
    pub base_url: String,
    pub listen_addr: String,
    pub auth_gateway_url: String,
    pub internal_api_key: String,
    pub roblox_client_id: String,
    pub roblox_client_secret: String,
    /// Public Roblox API requests per minute (per host).
    pub roblox_api_rate_limit: u32,
    /// Per-Open-Cloud-key requests per minute.
    pub open_cloud_rate_limit: u32,
    /// 32-byte AES-GCM key, hex-encoded.
    pub token_encryption_key_hex: String,
}

fn derive_origin(base_url: &str) -> String {
    if let Some(scheme_end) = base_url.find("://") {
        let after_scheme = scheme_end + 3;
        if let Some(path_slash) = base_url[after_scheme..].find('/') {
            return base_url[..after_scheme + path_slash].to_string();
        }
    }
    base_url.to_string()
}

impl AppConfig {
    pub fn from_env() -> Self {
        let base_url = env::var("BASE_URL").expect("BASE_URL must be set");
        let auth_gateway_url = env::var("AUTH_GATEWAY_URL")
            .ok()
            .map(|s| s.trim_end_matches('/').to_string())
            .unwrap_or_else(|| derive_origin(&base_url));

        Self {
            database_url: env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
            session_secret: env::var("SESSION_SECRET").expect("SESSION_SECRET must be set"),
            base_url,
            listen_addr: env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8089".to_string()),
            auth_gateway_url,
            internal_api_key: env::var("INTERNAL_API_KEY")
                .expect("INTERNAL_API_KEY must be set (must match the Auth Gateway's value)"),
            roblox_client_id: env::var("ROBLOX_CLIENT_ID").expect("ROBLOX_CLIENT_ID must be set"),
            roblox_client_secret: env::var("ROBLOX_CLIENT_SECRET")
                .expect("ROBLOX_CLIENT_SECRET must be set"),
            roblox_api_rate_limit: env::var("ROBLOX_API_RATE_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60),
            open_cloud_rate_limit: env::var("OPEN_CLOUD_RATE_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
            token_encryption_key_hex: env::var("TOKEN_ENCRYPTION_KEY")
                .expect("TOKEN_ENCRYPTION_KEY must be set (32 bytes / 64 hex chars)"),
        }
    }
}
