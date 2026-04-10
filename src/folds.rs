use crate::models::{
    FOLDS_STATUS_FAIL, FOLDS_STATUS_PENDING,
    FOLDS_STATUS_UNDISCLOSED_FAILURE, FOLDS_UNDISCLOSED_AFTER_SECS,
};
use crate::storage;
use chrono::{DateTime, Utc};
use fiftyone_folds::{FoldsClient, FoldsError, ModelResponse, PollConfig};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::Duration;

// ---------------------------------------------------------------------------
// App-side request type (kept as the app's own input structure)
// ---------------------------------------------------------------------------

pub struct FoldsCreateRequest {
    pub question: String,
    pub outcomes: Vec<String>,
    pub additional_context: String,
    pub model_type: String,
}

// ---------------------------------------------------------------------------
// Events sent from background threads to the UI via mpsc
// ---------------------------------------------------------------------------

pub enum FoldsResult {
    /// Model ID received from 51Folds after POST /api/v1/models.
    Created(String),
    /// Full model response once the build is complete. Boxed because the
    /// response is large (drivers, edges, context, justification).
    Completed(Box<ModelResponse>),
    /// Irrecoverable error — network failure, timeout, or model build failed.
    Failed(String),
}

// ---------------------------------------------------------------------------
// Tokio runtime helper
// ---------------------------------------------------------------------------

fn build_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .enable_io()
        .build()
        .expect("failed to build tokio runtime for 51Folds SDK")
}

fn build_client(api_key: &str) -> Result<FoldsClient, FoldsError> {
    // Always pass the token explicitly — the app stores it as FOLDS_API_KEY,
    // not the SDK's default API_TOKEN env var.
    FoldsClient::new(Some(api_key.to_owned()), None, None)
}

// ---------------------------------------------------------------------------
// Create model and poll until complete
// ---------------------------------------------------------------------------

/// Entry point for the foreground "Create 51Folds Model" flow. Runs on a
/// background `std::thread`, bridges to the async SDK via a single-threaded
/// tokio runtime.
///
/// 1. POST create → send `Created(model_id)` on channel
/// 2. Insert pending row into SQLite
/// 3. Poll via SDK `wait_until_complete()` (35 min ceiling)
/// 4. On completion → persist response JSON to DB, send `Completed`
pub fn create_and_poll(
    api_key: String,
    req: FoldsCreateRequest,
    db_path: PathBuf,
    inference_id: Option<i64>,
    created_at: DateTime<Utc>,
    tx: Sender<FoldsResult>,
) {
    let rt = build_runtime();
    rt.block_on(async {
        let client = match build_client(&api_key) {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(FoldsResult::Failed(format!("{e}")));
                return;
            }
        };

        // POST /api/v1/models
        let create_resp = match client
            .models()
            .create(
                &req.question,
                &req.outcomes,
                &req.additional_context,
                Some(&req.model_type),
                None,
                Some(true),
                Some(true),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(FoldsResult::Failed(format!("{e}")));
                return;
            }
        };

        let model_id = create_resp.first_model_id().to_owned();

        // Persist pending row BEFORE polling so resume sweep can pick up
        // if the app closes mid-build.
        persist_pending_row(&db_path, &model_id, &req.question, inference_id, &created_at);

        let _ = tx.send(FoldsResult::Created(model_id.clone()));

        // Poll until terminal. The SDK handles retry on 429/5xx internally.
        let poll_config = PollConfig {
            interval: Duration::from_secs(60),
            timeout: Duration::from_secs(FOLDS_UNDISCLOSED_AFTER_SECS as u64),
        };

        match client
            .models()
            .wait_until_complete(&model_id, Some(poll_config))
            .await
        {
            Ok(model) => {
                persist_completed(&db_path, &model_id, &model);
                let _ = tx.send(FoldsResult::Completed(Box::new(model)));
            }
            Err(FoldsError::ModelBuildFailed { model_id: mid, .. }) => {
                storage::update_folds_model_status_standalone(
                    &db_path,
                    &mid,
                    FOLDS_STATUS_FAIL,
                    Some(Utc::now()),
                );
                let _ = tx.send(FoldsResult::Failed(format!("Model {mid} build failed")));
            }
            Err(FoldsError::PollTimeout { message }) => {
                storage::update_folds_model_status_standalone(
                    &db_path,
                    &model_id,
                    FOLDS_STATUS_UNDISCLOSED_FAILURE,
                    Some(Utc::now()),
                );
                let _ = tx.send(FoldsResult::Failed(message));
            }
            Err(e) => {
                let _ = tx.send(FoldsResult::Failed(format!("{e}")));
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Patch drivers and return updated model (synchronous on API side)
// ---------------------------------------------------------------------------

/// Re-evaluate: change driver states and get updated outcome probabilities.
/// The 51Folds API returns the updated ModelResponse in the same request
/// (no polling needed for driver patches).
pub fn patch_drivers(
    api_key: String,
    model_id: String,
    drivers: Vec<fiftyone_folds::DriverStateInput>,
    tx: Sender<FoldsResult>,
) {
    let rt = build_runtime();
    rt.block_on(async {
        let client = match build_client(&api_key) {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(FoldsResult::Failed(format!("{e}")));
                return;
            }
        };

        match client.models().patch_drivers(&model_id, &drivers).await {
            Ok(model) => {
                let _ = tx.send(FoldsResult::Completed(Box::new(model)));
            }
            Err(e) => {
                let _ = tx.send(FoldsResult::Failed(format!("{e}")));
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Resume polling for models left pending across app restarts
// ---------------------------------------------------------------------------

/// Called from the resume sweep on startup. No UI channel — only updates
/// the database. Uses the remaining time from the 35-minute ceiling.
pub fn resume_poll(
    api_key: String,
    model_id: String,
    db_path: PathBuf,
    created_at: DateTime<Utc>,
) {
    let rt = build_runtime();
    rt.block_on(async {
        let client = match build_client(&api_key) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("warn: resume poll for {model_id} failed to build client: {e}");
                return;
            }
        };

        // Remaining time = ceiling minus elapsed
        let elapsed = (Utc::now() - created_at).num_seconds().max(0) as u64;
        let ceiling = FOLDS_UNDISCLOSED_AFTER_SECS as u64;
        let remaining = ceiling.saturating_sub(elapsed);

        if remaining < 60 {
            // Not enough time left — mark as undisclosed failure
            storage::update_folds_model_status_standalone(
                &db_path,
                &model_id,
                FOLDS_STATUS_UNDISCLOSED_FAILURE,
                Some(Utc::now()),
            );
            return;
        }

        let poll_config = PollConfig {
            interval: Duration::from_secs(60),
            timeout: Duration::from_secs(remaining),
        };

        match client
            .models()
            .wait_until_complete(&model_id, Some(poll_config))
            .await
        {
            Ok(model) => {
                persist_completed(&db_path, &model_id, &model);
            }
            Err(FoldsError::ModelBuildFailed { model_id: mid, .. }) => {
                storage::update_folds_model_status_standalone(
                    &db_path,
                    &mid,
                    FOLDS_STATUS_FAIL,
                    Some(Utc::now()),
                );
            }
            Err(FoldsError::PollTimeout { .. }) => {
                storage::update_folds_model_status_standalone(
                    &db_path,
                    &model_id,
                    FOLDS_STATUS_UNDISCLOSED_FAILURE,
                    Some(Utc::now()),
                );
            }
            Err(e) => {
                eprintln!("warn: resume poll for {model_id} failed: {e}");
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Database persistence helpers
// ---------------------------------------------------------------------------

fn persist_pending_row(
    db_path: &PathBuf,
    model_id: &str,
    question: &str,
    inference_id: Option<i64>,
    created_at: &DateTime<Utc>,
) {
    if let Ok(conn) = rusqlite::Connection::open(db_path) {
        let _ = conn.execute(
            r#"INSERT INTO folds_models
               (model_id, status, created_at, question, inference_id)
               VALUES (?1, ?2, ?3, ?4, ?5)"#,
            rusqlite::params![
                model_id,
                FOLDS_STATUS_PENDING,
                created_at.to_rfc3339(),
                question,
                inference_id,
            ],
        );
    }
}

fn persist_completed(db_path: &std::path::Path, model_id: &str, model: &ModelResponse) {
    let response_json = serde_json::to_string(model).unwrap_or_default();
    let outcomes_json = serde_json::to_string(
        &model
            .current
            .outcomes
            .iter()
            .map(|o| (&o.label, o.probability.unwrap_or(0.0)))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();

    storage::update_folds_model_completed_standalone(
        db_path,
        model_id,
        &response_json,
        &outcomes_json,
        &model.short_summary,
    );
}
