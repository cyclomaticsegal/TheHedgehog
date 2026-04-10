use std::time::Duration;

use crate::constants::{
    MODEL_POLL_INTERVAL_SECS, MODEL_POLL_TIMEOUT_SECS, REPORT_POLL_INTERVAL_SECS,
    REPORT_POLL_TIMEOUT_SECS,
};

/// Configuration for polling an async operation.
#[derive(Debug, Clone)]
pub struct PollConfig {
    pub interval: Duration,
    pub timeout: Duration,
}

impl PollConfig {
    /// Default config for model build polling (60s interval, 35min timeout).
    pub fn model() -> Self {
        Self {
            interval: Duration::from_secs(MODEL_POLL_INTERVAL_SECS),
            timeout: Duration::from_secs(MODEL_POLL_TIMEOUT_SECS),
        }
    }

    /// Default config for report generation polling (60s interval, 10min timeout).
    pub fn report() -> Self {
        Self {
            interval: Duration::from_secs(REPORT_POLL_INTERVAL_SECS),
            timeout: Duration::from_secs(REPORT_POLL_TIMEOUT_SECS),
        }
    }
}
