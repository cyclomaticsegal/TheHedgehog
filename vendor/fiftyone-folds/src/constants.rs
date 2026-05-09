pub const DEFAULT_BASE_URL: &str = "https://api.51folds.ai";
pub const UAT_BASE_URL: &str = "https://api-uat.fiftyonefolds.ai";
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

// Polling
pub const MODEL_POLL_INTERVAL_SECS: u64 = 60;
pub const MODEL_POLL_TIMEOUT_SECS: u64 = 35 * 60; // 35 minutes
pub const REPORT_POLL_INTERVAL_SECS: u64 = 60;
pub const REPORT_POLL_TIMEOUT_SECS: u64 = 10 * 60; // 10 minutes

// Retry
pub const MAX_RETRIES_429: u32 = 5;
pub const MAX_RETRIES_500: u32 = 3;
pub const RETRY_BASE_DELAY_SECS: f64 = 2.0;
pub const RETRY_MAX_DELAY_SECS: f64 = 60.0;

// Model types
pub const VALID_MODEL_TYPES: &[&str] = &["Overview", "Insight", "Advanced"];
pub const DEFAULT_MODEL_TYPE: &str = "Advanced";

// Status matching — case-insensitive, survives API fixing the typo
pub const COMPLETE_STATUSES: &[&str] = &["successed", "succeeded"];
pub const FAILED_STATUSES: &[&str] = &["failed"];

// Validation
pub const MIN_QUESTION_LENGTH: usize = 10;
pub const MIN_OUTCOMES: usize = 2;
pub const MAX_OUTCOMES: usize = 5;
pub const MAX_CONTEXT_WORDS: usize = 300;
pub const WARN_CONTEXT_WORDS: usize = 250;
