use std::collections::HashMap;

use reqwest::Method;

use crate::client::HttpTransport;
use crate::errors::FoldsError;
use crate::types::{CreditsResponse, TransactionsResponse};

/// Access credit and transaction endpoints.
pub struct CreditsResource<'a> {
    transport: &'a HttpTransport,
}

impl<'a> CreditsResource<'a> {
    pub(crate) fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// Get current credit balance.
    pub async fn me(&self) -> Result<CreditsResponse, FoldsError> {
        self.transport
            .request_data(Method::GET, "/api/v1/credits/me", None, None, None)
            .await
    }

    /// Get transaction history with optional filters.
    pub async fn transactions(
        &self,
        page: Option<i64>,
        page_size: Option<i64>,
        from_date: Option<&str>,
        to_date: Option<&str>,
        transaction_type: Option<&str>,
    ) -> Result<TransactionsResponse, FoldsError> {
        let mut params = HashMap::new();
        params.insert("Page".into(), page.unwrap_or(1).to_string());
        params.insert("PageSize".into(), page_size.unwrap_or(20).to_string());
        if let Some(from) = from_date {
            params.insert("From".into(), from.into());
        }
        if let Some(to) = to_date {
            params.insert("To".into(), to.into());
        }
        if let Some(t) = transaction_type {
            params.insert("Type".into(), t.into());
        }

        self.transport
            .request_data(
                Method::GET,
                "/api/v1/credits/transactions",
                None,
                Some(&params),
                None,
            )
            .await
    }
}
