use std::time::Duration;

use crate::error::AppError;

/// Hard limit per Role Link API for PUT /users and chunked uploads.
pub const CHUNK_SIZE: usize = 100_000;

/// RoleLogic's documented per-role member ceiling.
pub const MAX_USERS_PER_ROLE: usize = 30_000_000;

const COMMIT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CHUNK_TIMEOUT: Duration = Duration::from_secs(2 * 60);

#[derive(Clone)]
pub struct RoleLogicClient {
    http: reqwest::Client,
    base_url: String,
}

impl RoleLogicClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(16)
            .build()
            .expect("Failed to build HTTP client");

        Self {
            http,
            base_url: "https://api-rolelogic.faizo.net".to_string(),
        }
    }

    fn users_url(&self, guild_id: &str, role_id: &str) -> String {
        format!(
            "{}/api/role-link/{}/{}/users",
            self.base_url, guild_id, role_id
        )
    }

    /// Get user count and limit for a role link.
    pub async fn get_user_info(
        &self,
        guild_id: &str,
        role_id: &str,
        token: &str,
    ) -> Result<(usize, usize), AppError> {
        let resp = self
            .http
            .get(self.users_url(guild_id, role_id))
            .header("Authorization", format!("Token {token}"))
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::RoleLogic(format!(
                "Get user info failed: {status} - {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        let user_count = body["data"]["user_count"].as_u64().unwrap_or(0) as usize;
        let user_limit = body["data"]["user_limit"].as_u64().unwrap_or(100) as usize;

        Ok((user_count, user_limit))
    }

    pub async fn add_user(
        &self,
        guild_id: &str,
        role_id: &str,
        user_id: &str,
        token: &str,
    ) -> Result<bool, AppError> {
        let url = format!("{}/{}", self.users_url(guild_id, role_id), user_id);

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Token {token}"))
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();

            if (status == reqwest::StatusCode::BAD_REQUEST
                || status == reqwest::StatusCode::FORBIDDEN)
                && body.to_lowercase().contains("limit")
            {
                let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                let limit = parsed["data"]["user_limit"].as_u64().unwrap_or(100) as usize;
                return Err(AppError::UserLimitReached { limit });
            }

            return Err(AppError::RoleLogic(format!(
                "Add user failed: {status} - {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        Ok(body["data"]["added"].as_bool().unwrap_or(false))
    }

    pub async fn remove_user(
        &self,
        guild_id: &str,
        role_id: &str,
        user_id: &str,
        token: &str,
    ) -> Result<bool, AppError> {
        let url = format!("{}/{}", self.users_url(guild_id, role_id), user_id);

        let resp = self
            .http
            .delete(&url)
            .header("Authorization", format!("Token {token}"))
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::RoleLogic(format!(
                "Remove user failed: {status} - {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        Ok(body["data"]["removed"].as_bool().unwrap_or(false))
    }

    /// Atomic full replace for lists of up to CHUNK_SIZE users.
    pub async fn replace_users(
        &self,
        guild_id: &str,
        role_id: &str,
        user_ids: &[String],
        token: &str,
    ) -> Result<usize, AppError> {
        if user_ids.len() > CHUNK_SIZE {
            return Err(AppError::RoleLogic(format!(
                "replace_users called with {} ids; use replace_users_scalable for > {CHUNK_SIZE}",
                user_ids.len()
            )));
        }

        let resp = self
            .http
            .put(self.users_url(guild_id, role_id))
            .header("Authorization", format!("Token {token}"))
            .json(user_ids)
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::BAD_REQUEST
                && body.to_lowercase().contains("maximum")
            {
                return Err(AppError::UserLimitReached { limit: 0 });
            }
            return Err(AppError::RoleLogic(format!(
                "Replace users failed: {status} - {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        Ok(body["data"]["user_count"].as_u64().unwrap_or(0) as usize)
    }

    /// Full replace that transparently switches to the chunked-upload flow when
    /// the list exceeds the single-PUT limit. Atomic — the live list only swaps
    /// in on commit. Pre-flights against the documented 30M-per-role ceiling.
    pub async fn replace_users_scalable(
        &self,
        guild_id: &str,
        role_id: &str,
        user_ids: &[String],
        token: &str,
    ) -> Result<usize, AppError> {
        if user_ids.len() > MAX_USERS_PER_ROLE {
            return Err(AppError::UploadTooLarge {
                count: user_ids.len(),
            });
        }

        if user_ids.len() <= CHUNK_SIZE {
            return self.replace_users(guild_id, role_id, user_ids, token).await;
        }

        let upload_id = self.start_upload(guild_id, role_id, token).await?;
        tracing::info!(
            guild_id,
            role_id,
            total = user_ids.len(),
            upload_id,
            "Starting chunked user upload"
        );

        for (idx, chunk) in user_ids.chunks(CHUNK_SIZE).enumerate() {
            if let Err(e) = self
                .append_chunk(guild_id, role_id, &upload_id, chunk, token)
                .await
            {
                tracing::error!(
                    guild_id,
                    role_id,
                    upload_id,
                    chunk_index = idx,
                    "Chunk upload failed, cancelling session: {e}"
                );
                let _ = self
                    .cancel_upload(guild_id, role_id, &upload_id, token)
                    .await;
                return Err(e);
            }
        }

        let final_count = self
            .commit_upload(guild_id, role_id, &upload_id, token)
            .await?;
        tracing::info!(
            guild_id,
            role_id,
            upload_id,
            final_count,
            "Chunked user upload committed"
        );
        Ok(final_count)
    }

    async fn start_upload(
        &self,
        guild_id: &str,
        role_id: &str,
        token: &str,
    ) -> Result<String, AppError> {
        let url = format!("{}/upload", self.users_url(guild_id, role_id));
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Token {token}"))
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::RoleLogic(format!(
                "Start upload failed: {status} - {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        body["data"]["upload_id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| AppError::RoleLogic("Start upload: missing upload_id".into()))
    }

    async fn append_chunk(
        &self,
        guild_id: &str,
        role_id: &str,
        upload_id: &str,
        chunk: &[String],
        token: &str,
    ) -> Result<(), AppError> {
        let url = format!(
            "{}/upload/{}/chunk",
            self.users_url(guild_id, role_id),
            upload_id
        );
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Token {token}"))
            .timeout(CHUNK_TIMEOUT)
            .json(chunk)
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::RoleLogic(format!(
                "Append chunk failed: {status} - {body}"
            )));
        }
        Ok(())
    }

    async fn commit_upload(
        &self,
        guild_id: &str,
        role_id: &str,
        upload_id: &str,
        token: &str,
    ) -> Result<usize, AppError> {
        let url = format!(
            "{}/upload/{}/commit",
            self.users_url(guild_id, role_id),
            upload_id
        );
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Token {token}"))
            .timeout(COMMIT_TIMEOUT)
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::BAD_REQUEST
                && body.to_lowercase().contains("maximum")
            {
                return Err(AppError::UserLimitReached { limit: 0 });
            }
            return Err(AppError::RoleLogic(format!(
                "Commit upload failed: {status} - {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        Ok(body["data"]["user_count"].as_u64().unwrap_or(0) as usize)
    }

    async fn cancel_upload(
        &self,
        guild_id: &str,
        role_id: &str,
        upload_id: &str,
        token: &str,
    ) -> Result<(), AppError> {
        let url = format!(
            "{}/upload/{}",
            self.users_url(guild_id, role_id),
            upload_id
        );
        let _ = self
            .http
            .delete(&url)
            .header("Authorization", format!("Token {token}"))
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;
        Ok(())
    }
}
