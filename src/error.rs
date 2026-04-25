use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Roblox API error: {0}")]
    RobloxApi(String),

    #[error("Roblox OAuth error: {0}")]
    RobloxOAuth(String),

    #[error("Open Cloud error: {0}")]
    OpenCloud(String),

    #[error("RoleLogic API error: {0}")]
    RoleLogic(String),

    #[error("Role link user limit reached ({limit})")]
    UserLimitReached { limit: usize },

    #[error("Upload too large ({count} > 30M)")]
    UploadTooLarge { count: usize },

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Unauthorized: {0}")]
    UnauthorizedWith(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Verification failed: {0}")]
    VerificationFailed(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::Database(e) => {
                tracing::error!("Database error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
            AppError::RobloxApi(e) => {
                tracing::error!("Roblox API error: {e}");
                (StatusCode::BAD_GATEWAY, "Failed to fetch Roblox data. Please try again later.".to_string())
            }
            AppError::RobloxOAuth(e) => {
                tracing::error!("Roblox OAuth error: {e}");
                (StatusCode::BAD_GATEWAY, "Roblox sign-in failed. Please try again.".to_string())
            }
            AppError::OpenCloud(e) => {
                tracing::error!("Open Cloud error: {e}");
                (StatusCode::BAD_GATEWAY, "Open Cloud request failed".to_string())
            }
            AppError::RoleLogic(e) => {
                tracing::error!("RoleLogic API error: {e}");
                (StatusCode::BAD_GATEWAY, "Failed to sync roles".to_string())
            }
            AppError::UserLimitReached { limit } => {
                tracing::warn!("Role link user limit reached: {limit}");
                (StatusCode::FORBIDDEN, "Role link user limit reached".to_string())
            }
            AppError::UploadTooLarge { count } => (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("Upload of {count} users exceeds RoleLogic's 30M-per-role ceiling"),
            ),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Unauthorized => {
                (StatusCode::UNAUTHORIZED, "Invalid or missing authorization".to_string())
            }
            AppError::UnauthorizedWith(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::VerificationFailed(msg) => (StatusCode::UNPROCESSABLE_ENTITY, msg.clone()),
            AppError::Internal(e) => {
                tracing::error!("Internal error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
        };

        let body = json!({ "error": message });
        (status, axum::Json(body)).into_response()
    }
}
