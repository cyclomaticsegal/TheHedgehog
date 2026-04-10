use serde_json::Value;
use std::fmt;

/// Per-field validation error from the API's `validates[]` response shape.
#[derive(Debug, Clone)]
pub struct FieldError {
    pub key: String,
    pub errors: Vec<String>,
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for err in &self.errors {
            write!(f, "{}: {}", self.key, err)?;
        }
        Ok(())
    }
}

/// All errors that the 51Folds SDK can produce.
#[derive(thiserror::Error, Debug)]
pub enum FoldsError {
    /// 401 — Token missing, malformed, or invalid.
    #[error("Authentication failed: {message}")]
    Authentication {
        message: String,
        status_code: Option<u16>,
        body: Option<Value>,
    },

    /// 403 — Token valid but belongs to a different account.
    #[error("Permission denied: {message}")]
    PermissionDenied {
        message: String,
        status_code: Option<u16>,
        body: Option<Value>,
    },

    /// 404 — Resource not found, or model hasn't finished building.
    #[error("Not found: {message}")]
    NotFound {
        message: String,
        status_code: Option<u16>,
        body: Option<Value>,
    },

    /// 400 — Request validation failed. Carries field_errors (validates[] shape)
    /// and/or reasons (reason[] shape).
    #[error("Validation failed: {message}")]
    Validation {
        message: String,
        field_errors: Vec<FieldError>,
        reasons: Vec<String>,
        status_code: Option<u16>,
        body: Option<Value>,
    },

    /// 429 — Rate limited. Auto-retried before surfacing.
    #[error("Rate limited: {message}")]
    RateLimit {
        message: String,
        retry_after: Option<f64>,
        status_code: Option<u16>,
        body: Option<Value>,
    },

    /// 500+ — Server error after retry exhaustion.
    #[error("Server error: {message}")]
    Server {
        message: String,
        status_code: Option<u16>,
        body: Option<Value>,
    },

    /// Connection, timeout, or DNS failure.
    #[error("Network error: {message}")]
    Network {
        message: String,
        #[source]
        source: Option<reqwest::Error>,
    },

    /// Polling timeout exceeded waiting for async operation.
    #[error("Polling timeout: {message}")]
    PollTimeout { message: String },

    /// Model build reached "Failed" status.
    #[error("Model build failed: {message}")]
    ModelBuildFailed { message: String, model_id: String },
}
