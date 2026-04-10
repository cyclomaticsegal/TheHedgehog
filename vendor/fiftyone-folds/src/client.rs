use reqwest::{Client, Method, Response, StatusCode};
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::env;
use std::time::{Duration, SystemTime};
use uuid::Uuid;

use crate::constants::{
    DEFAULT_BASE_URL, DEFAULT_TIMEOUT_SECS, MAX_RETRIES_429, MAX_RETRIES_500,
    RETRY_BASE_DELAY_SECS, RETRY_MAX_DELAY_SECS,
};
use crate::errors::{FieldError, FoldsError};
use crate::types::ApiEnvelope;

/// Low-level HTTP transport with auth, retry, envelope unwrapping, and error parsing.
pub struct HttpTransport {
    client: Client,
    base_url: String,
    token: String,
}

impl HttpTransport {
    pub fn new(
        api_token: Option<String>,
        base_url: Option<String>,
        timeout: Option<Duration>,
    ) -> Result<Self, FoldsError> {
        let token = api_token
            .or_else(|| env::var("API_TOKEN").ok())
            .ok_or_else(|| FoldsError::Authentication {
                message: "No API token found. Set the API_TOKEN environment variable \
                          or pass api_token to FoldsClient::new()."
                    .into(),
                status_code: None,
                body: None,
            })?;

        if !token.starts_with("at_sk_") {
            eprintln!(
                "[51folds] Token does not start with 'at_sk_'. \
                 Verify you are using a service API key, not a browser JWT."
            );
        }

        let resolved_base = base_url
            .or_else(|| {
                let val = env::var("FOLDS_BASE_URL").ok()?;
                if val.is_empty() {
                    None
                } else {
                    Some(val)
                }
            })
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();

        let client = Client::builder()
            .timeout(timeout.unwrap_or(Duration::from_secs(DEFAULT_TIMEOUT_SECS)))
            .user_agent(format!(
                "fiftyone-folds-rust/{} reqwest",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|e| FoldsError::Network {
                message: format!("Failed to build HTTP client: {}", e),
                source: Some(e),
            })?;

        Ok(Self {
            client,
            base_url: resolved_base,
            token,
        })
    }

    /// Generate a fresh UUID for idempotency keys.
    pub fn generate_idempotency_key(&self) -> String {
        Uuid::new_v4().to_string()
    }

    /// Make an HTTP request, unwrap the `{"data": ...}` envelope, and
    /// deserialize the inner value as `T`.
    pub async fn request_data<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
        params: Option<&HashMap<String, String>>,
        idempotency_key: Option<&str>,
    ) -> Result<T, FoldsError> {
        let raw = self
            .request_raw(method, path, body, params, idempotency_key)
            .await?;
        let envelope: ApiEnvelope<T> =
            serde_json::from_value(raw).map_err(|e| FoldsError::Network {
                message: format!("Failed to deserialize response envelope: {}", e),
                source: None,
            })?;
        Ok(envelope.data)
    }

    /// Make an HTTP request and return the raw JSON body.
    pub async fn request_raw(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
        params: Option<&HashMap<String, String>>,
        idempotency_key: Option<&str>,
    ) -> Result<serde_json::Value, FoldsError> {
        let url = format!("{}{}", self.base_url, path);
        let max_attempts = 1 + MAX_RETRIES_429.max(MAX_RETRIES_500);
        let mut retries_429 = 0u32;
        let mut retries_500 = 0u32;
        let last_response: Option<Response> = None;

        for _attempt in 0..max_attempts {
            let mut req = self
                .client
                .request(method.clone(), &url)
                .bearer_auth(&self.token)
                .header("Accept", "application/json");

            if let Some(p) = params {
                req = req.query(p);
            }
            if let Some(key) = idempotency_key {
                req = req.header("X-Idempotency-Key", key);
            }
            if let Some(b) = body {
                req = req.header("Content-Type", "application/json").json(b);
            }

            let response = match req.send().await {
                Ok(r) => r,
                Err(e) if e.is_timeout() => {
                    return Err(FoldsError::Network {
                        message: format!("Request timed out: {}", e),
                        source: Some(e),
                    });
                }
                Err(e) if e.is_connect() => {
                    return Err(FoldsError::Network {
                        message: format!("Connection failed: {}", e),
                        source: Some(e),
                    });
                }
                Err(e) => {
                    return Err(FoldsError::Network {
                        message: format!("HTTP error: {}", e),
                        source: Some(e),
                    });
                }
            };

            let status = response.status();

            if status.is_success() {
                let json: serde_json::Value =
                    response.json().await.map_err(|e| FoldsError::Network {
                        message: format!("Failed to parse response JSON: {}", e),
                        source: Some(e),
                    })?;
                return Ok(json);
            }

            // Retry on 429
            if status == StatusCode::TOO_MANY_REQUESTS && retries_429 < MAX_RETRIES_429 {
                retries_429 += 1;
                let retry_after = response
                    .headers()
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<f64>().ok());
                let delay = retry_after.unwrap_or_else(|| {
                    (RETRY_BASE_DELAY_SECS * 2f64.powi(retries_429 as i32 - 1))
                        .min(RETRY_MAX_DELAY_SECS)
                });
                tokio::time::sleep(Duration::from_secs_f64(delay + jitter())).await;
                continue;
            }

            // Retry on 5xx
            if status.is_server_error() && retries_500 < MAX_RETRIES_500 {
                retries_500 += 1;
                let delay = (RETRY_BASE_DELAY_SECS * 2f64.powi(retries_500 as i32 - 1))
                    .min(RETRY_MAX_DELAY_SECS);
                tokio::time::sleep(Duration::from_secs_f64(delay + jitter())).await;
                continue;
            }

            // Non-retryable error
            return Err(parse_error_response(response).await);
        }

        // Exhausted retries — parse the last error
        match last_response {
            Some(resp) => Err(parse_error_response(resp).await),
            None => Err(FoldsError::Network {
                message: "Request failed after all retry attempts".into(),
                source: None,
            }),
        }
    }
}

/// Cheap jitter using system time nanos (avoids a `rand` dependency).
fn jitter() -> f64 {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos % 1000) as f64 / 1000.0
}

/// Parse an error HTTP response into the appropriate FoldsError variant.
///
/// Handles both error shapes:
/// 1. validates pattern: `{"success": false, "validates": [...], "error": "..."}`
/// 2. reason pattern: `{"reason": ["..."]}`
async fn parse_error_response(response: Response) -> FoldsError {
    let status = response.status().as_u16();
    let body: serde_json::Value = response.json().await.unwrap_or(serde_json::Value::Null);

    // Shape 1: validates pattern
    if let Some(validates) = body.get("validates").and_then(|v| v.as_array()) {
        if !validates.is_empty() {
            let field_errors: Vec<FieldError> = validates
                .iter()
                .filter_map(|v| {
                    let key = v.get("key")?.as_str()?.to_string();
                    let errors = v
                        .get("errors")?
                        .as_array()?
                        .iter()
                        .filter_map(|e| e.as_str().map(String::from))
                        .collect();
                    Some(FieldError { key, errors })
                })
                .collect();
            let detail = field_errors
                .iter()
                .flat_map(|fe| fe.errors.iter().map(move |e| format!("{}: {}", fe.key, e)))
                .collect::<Vec<_>>()
                .join("\n  ");
            return FoldsError::Validation {
                message: format!("Validation failed:\n  {}", detail),
                field_errors,
                reasons: Vec::new(),
                status_code: Some(status),
                body: Some(body),
            };
        }
    }

    // Shape 2: reason pattern
    if let Some(reasons) = body.get("reason").and_then(|v| v.as_array()) {
        let reason_strings: Vec<String> = reasons
            .iter()
            .filter_map(|r| r.as_str().map(String::from))
            .collect();
        if !reason_strings.is_empty() {
            return FoldsError::Validation {
                message: reason_strings.join("; "),
                field_errors: Vec::new(),
                reasons: reason_strings,
                status_code: Some(status),
                body: Some(body),
            };
        }
    }

    // Fallback: map by HTTP status
    let error_msg = body
        .get("error")
        .or_else(|| body.get("message"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| format!("HTTP {}", status));

    match status {
        401 => FoldsError::Authentication {
            message: format!(
                "Authentication failed: {}. Check that API_TOKEN is the long-lived at_sk_... key, not a browser JWT.",
                error_msg
            ),
            status_code: Some(status),
            body: Some(body),
        },
        403 => FoldsError::PermissionDenied {
            message: format!(
                "Permission denied: {}. The token may belong to a different account.",
                error_msg
            ),
            status_code: Some(status),
            body: Some(body),
        },
        404 => FoldsError::NotFound {
            message: error_msg,
            status_code: Some(status),
            body: Some(body),
        },
        429 => FoldsError::RateLimit {
            message: error_msg,
            retry_after: None,
            status_code: Some(status),
            body: Some(body),
        },
        s if s >= 500 => FoldsError::Server {
            message: format!("Server error ({}): {}", s, error_msg),
            status_code: Some(status),
            body: Some(body),
        },
        _ => FoldsError::Server {
            message: error_msg,
            status_code: Some(status),
            body: Some(body),
        },
    }
}
