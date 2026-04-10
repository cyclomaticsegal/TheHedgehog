use std::collections::HashMap;
use std::time::Instant;

use reqwest::Method;

use crate::client::HttpTransport;
use crate::constants::DEFAULT_MODEL_TYPE;
use crate::errors::FoldsError;
use crate::polling::PollConfig;
use crate::types::{
    CreateModelRequest, CreateModelResponse, DiagnosticResponse, DriverStateInput,
    GenerateReportRequest, JustificationResponse, ModelResponse, ReportPollResponse,
    ReportTriggerResponse, RevisionsResponse, SchemaResponse, UpdateDriversRequest,
};
use crate::validation::validate_create_model;

/// Access model endpoints: create, get, update, inspect, report.
pub struct ModelsResource<'a> {
    transport: &'a HttpTransport,
}

impl<'a> ModelsResource<'a> {
    pub(crate) fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    // ------------------------------------------------------------------
    // Create
    // ------------------------------------------------------------------

    /// Create a model asynchronously. Returns immediately with the model ID.
    ///
    /// Poll with [`get`] or use [`create_and_wait`] to block until complete.
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        question: &str,
        outcomes: &[String],
        additional_context: &str,
        model_type: Option<&str>,
        count: Option<u32>,
        generate_driver_content: Option<bool>,
        generate_takeaway_content: Option<bool>,
    ) -> Result<CreateModelResponse, FoldsError> {
        let mt = model_type.unwrap_or(DEFAULT_MODEL_TYPE);
        validate_create_model(question, outcomes, additional_context, mt)?;

        let req = CreateModelRequest {
            question,
            outcomes,
            additional_context,
            model_type: mt,
            count: count.unwrap_or(1),
            generate_driver_content: generate_driver_content.unwrap_or(true),
            generate_take_away_content: generate_takeaway_content.unwrap_or(true),
        };

        let body = serde_json::to_value(&req).map_err(|e| FoldsError::Network {
            message: format!("Failed to serialize request: {}", e),
            source: None,
        })?;

        let key = self.transport.generate_idempotency_key();
        self.transport
            .request_data(
                Method::POST,
                "/api/v1/models",
                Some(&body),
                None,
                Some(&key),
            )
            .await
    }

    /// Create a model and block until it finishes building.
    ///
    /// Returns the complete [`ModelResponse`] with all richness flags enabled.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_and_wait(
        &self,
        question: &str,
        outcomes: &[String],
        additional_context: &str,
        model_type: Option<&str>,
        count: Option<u32>,
        generate_driver_content: Option<bool>,
        generate_takeaway_content: Option<bool>,
        poll_config: Option<PollConfig>,
    ) -> Result<ModelResponse, FoldsError> {
        let result = self
            .create(
                question,
                outcomes,
                additional_context,
                model_type,
                count,
                generate_driver_content,
                generate_takeaway_content,
            )
            .await?;

        let config = poll_config.unwrap_or_else(PollConfig::model);
        self.wait_until_complete(result.first_model_id(), Some(config))
            .await
    }

    /// Poll an existing model until it finishes building.
    pub async fn wait_until_complete(
        &self,
        model_id: &str,
        poll_config: Option<PollConfig>,
    ) -> Result<ModelResponse, FoldsError> {
        let config = poll_config.unwrap_or_else(PollConfig::model);
        let start = Instant::now();

        loop {
            let model = self.get(model_id, None, None).await?;

            if model.is_complete() {
                return Ok(model);
            }

            if model.is_failed() {
                return Err(FoldsError::ModelBuildFailed {
                    message: format!(
                        "Model {} build failed. Use client.models().retry(\"{}\") to requeue.",
                        model_id, model_id
                    ),
                    model_id: model_id.to_string(),
                });
            }

            let elapsed = start.elapsed();
            if elapsed + config.interval > config.timeout {
                return Err(FoldsError::PollTimeout {
                    message: format!(
                        "Model {} did not complete within {}s (last status: {})",
                        model_id,
                        config.timeout.as_secs(),
                        model.status
                    ),
                });
            }

            tokio::time::sleep(config.interval).await;
        }
    }

    // ------------------------------------------------------------------
    // Get
    // ------------------------------------------------------------------

    /// Fetch a model by ID.
    ///
    /// Defaults to maximum richness (both Include flags true).
    /// Pass `Some(false)` to opt out.
    pub async fn get(
        &self,
        model_id: &str,
        include_driver_context: Option<bool>,
        include_driver_justification: Option<bool>,
    ) -> Result<ModelResponse, FoldsError> {
        let mut params = HashMap::new();
        params.insert(
            "IncludeDriverContext".into(),
            include_driver_context.unwrap_or(true).to_string(),
        );
        params.insert(
            "IncludeDriverJustification".into(),
            include_driver_justification.unwrap_or(true).to_string(),
        );

        self.transport
            .request_data(
                Method::GET,
                &format!("/api/v1/models/{}", model_id),
                None,
                Some(&params),
                None,
            )
            .await
    }

    /// List models with pagination. Note: status is NOT included per model.
    #[allow(clippy::too_many_arguments)]
    pub async fn list(
        &self,
        page: Option<i64>,
        page_size: Option<i64>,
        search_text: Option<&str>,
        created_from: Option<&str>,
        created_to: Option<&str>,
        updated_from: Option<&str>,
        updated_to: Option<&str>,
        include_batch: Option<bool>,
    ) -> Result<serde_json::Value, FoldsError> {
        let mut params = HashMap::new();
        params.insert("Page".into(), page.unwrap_or(1).to_string());
        params.insert("PageSize".into(), page_size.unwrap_or(20).to_string());
        if let Some(s) = search_text {
            params.insert("SearchText".into(), s.into());
        }
        if let Some(s) = created_from {
            params.insert("CreatedFrom".into(), s.into());
        }
        if let Some(s) = created_to {
            params.insert("CreatedTo".into(), s.into());
        }
        if let Some(s) = updated_from {
            params.insert("UpdatedFrom".into(), s.into());
        }
        if let Some(s) = updated_to {
            params.insert("UpdatedTo".into(), s.into());
        }
        if let Some(b) = include_batch {
            params.insert("IncludeBatch".into(), b.to_string());
        }

        let raw = self
            .transport
            .request_raw(Method::GET, "/api/v1/models", None, Some(&params), None)
            .await?;

        Ok(raw.get("data").cloned().unwrap_or(raw))
    }

    // ------------------------------------------------------------------
    // Inspection
    // ------------------------------------------------------------------

    /// Get the Bayesian network schema (`rawSchemaFile`).
    pub async fn schema(&self, model_id: &str) -> Result<SchemaResponse, FoldsError> {
        self.transport
            .request_data(
                Method::GET,
                &format!("/api/v1/models/{}/schema", model_id),
                None,
                None,
                None,
            )
            .await
    }

    /// Get the 10-section model diagnostic.
    pub async fn diagnostic(&self, model_id: &str) -> Result<DiagnosticResponse, FoldsError> {
        self.transport
            .request_data(
                Method::GET,
                &format!("/api/v1/models/{}/diagnostic", model_id),
                None,
                None,
                None,
            )
            .await
    }

    /// Get the build-time justification report (markdown prose).
    pub async fn justification(&self, model_id: &str) -> Result<JustificationResponse, FoldsError> {
        self.transport
            .request_data(
                Method::GET,
                &format!("/api/v1/models/{}/justification", model_id),
                None,
                None,
                None,
            )
            .await
    }

    /// Get the full revision history.
    pub async fn revisions(&self, model_id: &str) -> Result<RevisionsResponse, FoldsError> {
        self.transport
            .request_data(
                Method::GET,
                &format!("/api/v1/models/{}/revisions", model_id),
                None,
                None,
                None,
            )
            .await
    }

    // ------------------------------------------------------------------
    // Drivers and evidence
    // ------------------------------------------------------------------

    /// Replace all driver states (PUT). Triggers re-inference.
    pub async fn update_drivers(
        &self,
        model_id: &str,
        drivers: &[DriverStateInput],
    ) -> Result<ModelResponse, FoldsError> {
        let req = UpdateDriversRequest { drivers };
        let body = serde_json::to_value(&req).map_err(|e| FoldsError::Network {
            message: format!("Failed to serialize request: {}", e),
            source: None,
        })?;

        self.transport
            .request_data(
                Method::PUT,
                &format!("/api/v1/models/{}/drivers", model_id),
                Some(&body),
                None,
                None,
            )
            .await
    }

    /// Update specific driver states (PATCH). Triggers re-inference.
    pub async fn patch_drivers(
        &self,
        model_id: &str,
        drivers: &[DriverStateInput],
    ) -> Result<ModelResponse, FoldsError> {
        let req = UpdateDriversRequest { drivers };
        let body = serde_json::to_value(&req).map_err(|e| FoldsError::Network {
            message: format!("Failed to serialize request: {}", e),
            source: None,
        })?;

        self.transport
            .request_data(
                Method::PATCH,
                &format!("/api/v1/models/{}/drivers", model_id),
                Some(&body),
                None,
                None,
            )
            .await
    }

    /// Submit evidence to update the model.
    pub async fn submit_evidence(
        &self,
        model_id: &str,
        evidence: &serde_json::Value,
    ) -> Result<serde_json::Value, FoldsError> {
        let key = self.transport.generate_idempotency_key();
        let raw = self
            .transport
            .request_raw(
                Method::POST,
                &format!("/api/v1/models/{}/evidence", model_id),
                Some(evidence),
                None,
                Some(&key),
            )
            .await?;

        Ok(raw.get("data").cloned().unwrap_or(raw))
    }

    // ------------------------------------------------------------------
    // Reports
    // ------------------------------------------------------------------

    /// Trigger async report generation. Poll with [`get_report`].
    pub async fn generate_report(
        &self,
        model_id: &str,
        report_type: &str,
    ) -> Result<ReportTriggerResponse, FoldsError> {
        let req = GenerateReportRequest { report_type };
        let body = serde_json::to_value(&req).map_err(|e| FoldsError::Network {
            message: format!("Failed to serialize request: {}", e),
            source: None,
        })?;

        self.transport
            .request_data(
                Method::POST,
                &format!("/api/v1/models/{}/reports", model_id),
                Some(&body),
                None,
                None,
            )
            .await
    }

    /// Fetch a report. Check [`ReportPollResponse::is_ready`] before reading result.
    pub async fn get_report(
        &self,
        model_id: &str,
        report_type: &str,
    ) -> Result<ReportPollResponse, FoldsError> {
        let mut params = HashMap::new();
        params.insert("reportType".into(), report_type.into());

        self.transport
            .request_data(
                Method::GET,
                &format!("/api/v1/models/{}/reports", model_id),
                None,
                Some(&params),
                None,
            )
            .await
    }

    /// Generate a report and block until ready.
    pub async fn generate_report_and_wait(
        &self,
        model_id: &str,
        report_type: &str,
        poll_config: Option<PollConfig>,
    ) -> Result<ReportPollResponse, FoldsError> {
        self.generate_report(model_id, report_type).await?;

        let config = poll_config.unwrap_or_else(PollConfig::report);
        let start = Instant::now();

        loop {
            let report = self.get_report(model_id, report_type).await?;

            if report.is_ready() {
                return Ok(report);
            }

            let elapsed = start.elapsed();
            if elapsed + config.interval > config.timeout {
                return Err(FoldsError::PollTimeout {
                    message: format!(
                        "Report '{}' for model {} did not complete within {}s (last status: {:?})",
                        report_type,
                        model_id,
                        config.timeout.as_secs(),
                        report.status
                    ),
                });
            }

            tokio::time::sleep(config.interval).await;
        }
    }

    // ------------------------------------------------------------------
    // Retry
    // ------------------------------------------------------------------

    /// Retry a failed model build. Returns 202 Accepted.
    pub async fn retry(&self, model_id: &str) -> Result<serde_json::Value, FoldsError> {
        let raw = self
            .transport
            .request_raw(
                Method::POST,
                &format!("/api/v1/models/{}/retry", model_id),
                None,
                None,
                None,
            )
            .await?;

        Ok(raw.get("data").cloned().unwrap_or(raw))
    }
}
