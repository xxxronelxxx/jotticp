use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use serde_json::json;
use thiserror::Error;

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("Authentication required")]
    Unauthorized,

    #[error("Invalid credentials")]
    InvalidCredentials,

    #[error("Invalid or expired TOTP code")]
    InvalidTotp,

    #[error("Account locked — too many failed attempts. Try again in {retry_after} seconds.")]
    AccountLocked { retry_after: u64 },

    #[error("Permission denied: {0}")]
    Forbidden(String),

    #[error("{resource} not found")]
    NotFound { resource: &'static str },

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("The domain {domain} is already in use")]
    DomainConflict { domain: String },

    #[error("Rate limit exceeded. Please wait {retry_after} seconds.")]
    RateLimited { retry_after: u64 },

    #[error("This feature requires OrbitCP Pro. Upgrade at /settings/billing")]
    FeatureGated { feature: &'static str },

    #[error("Server capacity exceeded: {0}")]
    Capacity(String),

    #[error("The operation timed out. Please try again.")]
    Timeout,

    #[error("External service error: {0}")]
    ExternalService(String),

    #[error("An internal error occurred")]
    Internal(#[from] anyhow::Error),

    #[error("Database error")]
    Database(#[from] sqlx::Error),
}

impl ApiError {
    /// Construct an `ApiError` from an arbitrary HTTP status code and message.
    /// Used by agent-generated code that calls `ApiError::new(StatusCode::X, "msg")`.
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        match status {
            StatusCode::NOT_FOUND       => ApiError::Validation(message.into()), // closest without resource
            StatusCode::FORBIDDEN       => ApiError::Forbidden(message.into()),
            StatusCode::UNAUTHORIZED    => ApiError::Unauthorized,
            StatusCode::CONFLICT        => ApiError::DomainConflict { domain: message.into() },
            StatusCode::PAYMENT_REQUIRED => ApiError::FeatureGated { feature: "pro" },
            _                           => ApiError::Validation(message.into()),
        }
    }

    /// Wrap an `anyhow::Error`-compatible string as an internal error.
    pub fn internal(message: impl Into<String>) -> Self {
        ApiError::Internal(anyhow::anyhow!("{}", message.into()))
    }

    /// Wrap a validation / bad-request error.
    pub fn bad_request(message: impl Into<String>) -> Self {
        ApiError::Validation(message.into())
    }

    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Unauthorized                     => StatusCode::UNAUTHORIZED,
            Self::InvalidCredentials               => StatusCode::UNAUTHORIZED,
            Self::InvalidTotp                      => StatusCode::UNAUTHORIZED,
            Self::AccountLocked { .. }             => StatusCode::TOO_MANY_REQUESTS,
            Self::Forbidden(_)                     => StatusCode::FORBIDDEN,
            Self::NotFound { .. }                  => StatusCode::NOT_FOUND,
            Self::Validation(_)                    => StatusCode::UNPROCESSABLE_ENTITY,
            Self::DomainConflict { .. }            => StatusCode::CONFLICT,
            Self::RateLimited { .. }               => StatusCode::TOO_MANY_REQUESTS,
            Self::FeatureGated { .. }              => StatusCode::PAYMENT_REQUIRED,
            Self::Capacity(_)                      => StatusCode::INSUFFICIENT_STORAGE,
            Self::Timeout                          => StatusCode::GATEWAY_TIMEOUT,
            Self::ExternalService(_)               => StatusCode::BAD_GATEWAY,
            Self::Internal(_) | Self::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn error_code(&self) -> &'static str {
        match self {
            Self::Unauthorized         => "unauthorized",
            Self::InvalidCredentials   => "invalid_credentials",
            Self::InvalidTotp          => "invalid_totp",
            Self::AccountLocked { .. } => "account_locked",
            Self::Forbidden(_)         => "forbidden",
            Self::NotFound { .. }      => "not_found",
            Self::Validation(_)        => "validation_error",
            Self::DomainConflict { .. }=> "domain_conflict",
            Self::RateLimited { .. }   => "rate_limited",
            Self::FeatureGated { .. }  => "feature_gated",
            Self::Capacity(_)          => "capacity_exceeded",
            Self::Timeout              => "timeout",
            Self::ExternalService(_)   => "external_service_error",
            Self::Internal(_)
            | Self::Database(_)        => "internal_error",
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let code   = self.error_code();

        if matches!(&self, ApiError::Internal(_) | ApiError::Database(_)) {
            tracing::error!(error = %self, "Internal error");
            return (status, Json(json!({ "error": code, "message": "An internal error occurred." }))).into_response();
        }

        let body = match &self {
            ApiError::AccountLocked { retry_after } => json!({
                "error": code, "message": self.to_string(), "retry_after": retry_after }),
            ApiError::RateLimited { retry_after } => json!({
                "error": code, "message": self.to_string(), "retry_after": retry_after }),
            ApiError::FeatureGated { feature } => json!({
                "error": code, "message": self.to_string(),
                "feature": feature, "upgrade_url": "/settings/billing" }),
            ApiError::NotFound { resource } => json!({
                "error": code, "message": format!("{} not found", resource) }),
            _ => json!({ "error": code, "message": self.to_string() }),
        };

        (status, Json(body)).into_response()
    }
}
