//! # fiftyone-folds
//!
//! Rust client library for the 51Folds Bayesian modelling API.
//!
//! ```no_run
//! use fiftyone_folds::FoldsClient;
//!
//! # async fn example() -> Result<(), fiftyone_folds::FoldsError> {
//! let client = FoldsClient::new(None, None, None)?;  // reads API_TOKEN from env
//! let credits = client.credits().me().await?;
//! println!("Balance: {}", credits.amount);
//!
//! let model = client.models().create_and_wait(
//!     "Will X happen by Y?",
//!     &["Yes".into(), "No".into()],
//!     "Provide 250+ words of context for best results...",
//!     None, None, None, None, None,
//! ).await?;
//! println!("Status: {}", model.status);
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod constants;
pub mod errors;
pub mod polling;
pub mod resources;
pub mod types;
pub mod validation;

use std::time::Duration;

use client::HttpTransport;
pub use errors::*;
pub use polling::PollConfig;
pub use resources::{CreditsResource, ModelsResource};
pub use types::*;

/// Client for the 51Folds Bayesian modelling API.
///
/// # Examples
///
/// ```no_run
/// # async fn example() -> Result<(), fiftyone_folds::FoldsError> {
/// use fiftyone_folds::FoldsClient;
///
/// // Reads API_TOKEN from environment
/// let client = FoldsClient::new(None, None, None)?;
///
/// // Or configure explicitly
/// let client = FoldsClient::builder()
///     .api_token("at_sk_...")
///     .build()?;
///
/// let credits = client.credits().me().await?;
/// # Ok(())
/// # }
/// ```
pub struct FoldsClient {
    transport: HttpTransport,
}

impl FoldsClient {
    /// Create a new client.
    ///
    /// - `api_token`: Falls back to `API_TOKEN` env var. Must start with `at_sk_`.
    /// - `base_url`: Falls back to `FOLDS_BASE_URL` env var, then `https://api.51folds.ai`.
    /// - `timeout`: HTTP request timeout. Defaults to 30 seconds.
    pub fn new(
        api_token: Option<String>,
        base_url: Option<String>,
        timeout: Option<Duration>,
    ) -> Result<Self, FoldsError> {
        let transport = HttpTransport::new(api_token, base_url, timeout)?;
        Ok(Self { transport })
    }

    /// Start building a client with the builder pattern.
    pub fn builder() -> FoldsClientBuilder {
        FoldsClientBuilder::default()
    }

    /// Access model endpoints.
    pub fn models(&self) -> ModelsResource<'_> {
        ModelsResource::new(&self.transport)
    }

    /// Access credit and transaction endpoints.
    pub fn credits(&self) -> CreditsResource<'_> {
        CreditsResource::new(&self.transport)
    }
}

/// Builder for [`FoldsClient`].
#[derive(Default)]
pub struct FoldsClientBuilder {
    api_token: Option<String>,
    base_url: Option<String>,
    timeout: Option<Duration>,
}

impl FoldsClientBuilder {
    pub fn api_token(mut self, token: impl Into<String>) -> Self {
        self.api_token = Some(token.into());
        self
    }

    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn build(self) -> Result<FoldsClient, FoldsError> {
        FoldsClient::new(self.api_token, self.base_url, self.timeout)
    }
}
