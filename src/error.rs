//! Error types for skill-translator.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

/// Main error type for the application
#[derive(Debug, Error)]
pub enum AppError {
    #[error("Translation error: {0}")]
    TranslationError(#[from] TranslationError),

    #[error("Cache error: {0}")]
    CacheError(#[source] sqlx::Error),

    #[error("Invalid base64 content: {0}")]
    Base64Error(#[from] base64::DecodeError),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Internal server error: {0}")]
    Internal(String),
}

/// Translation-specific errors
#[derive(Debug, Error)]
pub enum TranslationError {
    #[error("Translation timed out after {0} seconds")]
    Timeout(u64),

    #[error("Translation failed after {attempts} attempts: {error}")]
    RetryFailed { attempts: u32, error: String },

    #[error("Empty response from upstream API")]
    EmptyResponse,

    #[error("OpenAI API error: {0}")]
    OpenAIError(String),
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::CacheError(err)
    }
}

impl From<async_openai::error::OpenAIError> for AppError {
    fn from(err: async_openai::error::OpenAIError) -> Self {
        AppError::TranslationError(TranslationError::OpenAIError(err.to_string()))
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::Base64Error(e) => (StatusCode::BAD_REQUEST, format!("Invalid base64 content: {}", e)),
            AppError::TranslationError(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Translation failed: {}", e)),
            AppError::CacheError(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Cache error: {}", e)),
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };

        let body = Json(json!({
            "detail": error_message
        }));

        (status, body).into_response()
    }
}

/// Result type alias for application errors
pub type AppResult<T> = Result<T, AppError>;
