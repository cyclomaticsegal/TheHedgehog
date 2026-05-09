use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Internal envelope for unwrapping API responses
// ---------------------------------------------------------------------------

/// Wrapper for the `{"data": ...}` envelope used by all API responses.
///
/// Useful for deserializing raw API JSON in tests or custom integrations.
#[derive(Debug, Deserialize)]
pub struct ApiEnvelope<T> {
    pub data: T,
}

// ---------------------------------------------------------------------------
// Nested types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDescriptor {
    pub name: String,
    pub description: String,
}

/// Analytical context from `IncludeDriverContext=true` on `data.drivers[]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverContext {
    pub importance: String,
    pub shifts: String,
    pub monitor: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub num: String,
    pub source: String,
}

/// Justification from `IncludeDriverJustification=true` on `data.current.drivers[]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverJustification {
    #[serde(default)]
    pub content: Vec<String>,
    #[serde(default)]
    pub citations: Vec<Citation>,
}

/// Driver definition from `data.drivers[]`.
///
/// `state_descriptors` is automatically parsed from JSON string or array.
/// `context` is present only when `IncludeDriverContext=true`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Driver {
    pub code: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_state_descriptors")]
    pub state_descriptors: Vec<StateDescriptor>,
    #[serde(default)]
    pub context: Option<DriverContext>,
}

/// Current driver state from `data.current.drivers[]`.
///
/// `justification` is present only when `IncludeDriverJustification=true`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverState {
    pub code: String,
    pub state: String,
    #[serde(default)]
    pub justification: Option<DriverJustification>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub id: i64,
    pub label: String,
    #[serde(default)]
    pub probability: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub parent: String,
    pub child: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentState {
    #[serde(default)]
    pub outcomes: Vec<Outcome>,
    #[serde(default)]
    pub drivers: Vec<DriverState>,
}

// ---------------------------------------------------------------------------
// Revision types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionOutcome {
    pub id: i64,
    pub probability: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionDriver {
    pub code: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Revision {
    pub id: i64,
    pub updated_at: String,
    pub trigger: String,
    #[serde(default)]
    pub outcomes: Vec<RevisionOutcome>,
    #[serde(default)]
    pub drivers: Vec<RevisionDriver>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Response from `POST /api/v1/models` (202 Accepted).
///
/// `model_id` is always an array, even for `count: 1`.
/// Use [`first_model_id()`](CreateModelResponse::first_model_id) for convenience.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateModelResponse {
    pub model_id: Vec<String>,
    pub question: String,
    pub outcomes: Vec<String>,
    pub additional_context: String,
    #[serde(rename = "type")]
    pub model_type: String,
    pub batch_id: Option<String>,
}

impl CreateModelResponse {
    /// First model ID (convenience for single-model creation).
    pub fn first_model_id(&self) -> &str {
        &self.model_id[0]
    }
}

/// Response from `GET /api/v1/models/{id}`.
///
/// Use [`is_complete`], [`is_failed`], [`is_running`] for defensive status matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelResponse {
    pub model_id: String,
    #[serde(default)]
    pub ownership: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub short_summary: String,
    #[serde(default)]
    pub drivers: Vec<Driver>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub current: CurrentState,
}

impl ModelResponse {
    /// Status matches "Successed" or "Succeeded" (case-insensitive).
    pub fn is_complete(&self) -> bool {
        let s = self.status.to_lowercase();
        s == "successed" || s == "succeeded"
    }

    /// Status matches "Failed" (case-insensitive).
    pub fn is_failed(&self) -> bool {
        self.status.to_lowercase() == "failed"
    }

    /// Model is still building (neither complete nor failed).
    pub fn is_running(&self) -> bool {
        !self.is_complete() && !self.is_failed()
    }
}

/// Response from `GET /api/v1/models/{id}/schema`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaResponse {
    pub model_id: String,
    pub raw_schema_file: String,
}

/// Response from `GET /api/v1/models/{id}/diagnostic`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticResponse {
    pub model_id: String,
    pub raw_diagnostics_file: String,
}

/// Response from `GET /api/v1/models/{id}/justification`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JustificationResponse {
    pub model_id: String,
    pub raw_justification_file: String,
}

/// Response from `GET /api/v1/models/{id}/revisions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionsResponse {
    pub model_id: String,
    #[serde(default)]
    pub ownership: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub outcomes: Vec<Outcome>,
    #[serde(default)]
    pub revisions: Vec<Revision>,
}

/// Response from `POST /api/v1/models/{id}/reports` (202 Accepted).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportTriggerResponse {
    pub status: String,
}

/// Response from `GET /api/v1/models/{id}/reports`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportPollResponse {
    pub status: Option<String>,
    pub result: Option<String>,
}

impl ReportPollResponse {
    pub fn is_ready(&self) -> bool {
        self.status.is_some() && self.result.is_some()
    }
}

/// Response from `GET /api/v1/credits/me`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditsResponse {
    pub amount: f64,
    pub retrieved_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    #[serde(rename = "type")]
    pub transaction_type: i64,
    pub type_name: String,
    pub amount: f64,
    pub description: String,
    pub timestamp: String,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub order_id: Option<String>,
    #[serde(default)]
    pub source: Option<i64>,
    #[serde(default)]
    pub modified_by: Option<i64>,
}

/// Response from `GET /api/v1/credits/transactions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionsResponse {
    pub transactions: Vec<Transaction>,
    pub page: i64,
    pub page_size: i64,
    pub total: i64,
    pub owner_id: i64,
    pub owner_type: i64,
    pub amount: f64,
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateModelRequest<'a> {
    pub question: &'a str,
    pub outcomes: &'a [String],
    pub additional_context: &'a str,
    #[serde(rename = "type")]
    pub model_type: &'a str,
    pub count: u32,
    pub generate_driver_content: bool,
    pub generate_take_away_content: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateDriversRequest<'a> {
    pub drivers: &'a [DriverStateInput],
}

/// Input for updating a driver's state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverStateInput {
    pub code: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GenerateReportRequest<'a> {
    pub report_type: &'a str,
}

// ---------------------------------------------------------------------------
// stateDescriptors custom deserializer
// ---------------------------------------------------------------------------

/// Deserializes `stateDescriptors` from either a JSON-encoded string or a
/// pre-parsed array. Returns an empty Vec on null/missing.
fn deserialize_state_descriptors<'de, D>(deserializer: D) -> Result<Vec<StateDescriptor>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(s) => serde_json::from_str(&s).map_err(de::Error::custom),
        serde_json::Value::Array(_) => serde_json::from_value(value).map_err(de::Error::custom),
        serde_json::Value::Null => Ok(Vec::new()),
        other => Err(de::Error::custom(format!(
            "expected string or array for stateDescriptors, got {}",
            other
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_descriptors_from_json_string() {
        let json = r#"{"code":"X","name":"Test","stateDescriptors":"[{\"name\":\"High\",\"description\":\"desc\"}]"}"#;
        let driver: Driver = serde_json::from_str(json).unwrap();
        assert_eq!(driver.state_descriptors.len(), 1);
        assert_eq!(driver.state_descriptors[0].name, "High");
    }

    #[test]
    fn state_descriptors_from_array() {
        let json = r#"{"code":"X","name":"Test","stateDescriptors":[{"name":"Low","description":"desc"}]}"#;
        let driver: Driver = serde_json::from_str(json).unwrap();
        assert_eq!(driver.state_descriptors.len(), 1);
        assert_eq!(driver.state_descriptors[0].name, "Low");
    }

    #[test]
    fn state_descriptors_null() {
        let json = r#"{"code":"X","name":"Test","stateDescriptors":null}"#;
        let driver: Driver = serde_json::from_str(json).unwrap();
        assert!(driver.state_descriptors.is_empty());
    }

    #[test]
    fn state_descriptors_missing() {
        let json = r#"{"code":"X","name":"Test"}"#;
        let driver: Driver = serde_json::from_str(json).unwrap();
        assert!(driver.state_descriptors.is_empty());
    }

    #[test]
    fn status_matching_successed() {
        let json =
            r#"{"modelId":"m1","status":"Successed","current":{"outcomes":[],"drivers":[]}}"#;
        let model: ModelResponse = serde_json::from_str(json).unwrap();
        assert!(model.is_complete());
        assert!(!model.is_failed());
        assert!(!model.is_running());
    }

    #[test]
    fn status_matching_succeeded() {
        let json =
            r#"{"modelId":"m1","status":"Succeeded","current":{"outcomes":[],"drivers":[]}}"#;
        let model: ModelResponse = serde_json::from_str(json).unwrap();
        assert!(model.is_complete());
    }

    #[test]
    fn status_matching_case_insensitive() {
        let json =
            r#"{"modelId":"m1","status":"SUCCESSED","current":{"outcomes":[],"drivers":[]}}"#;
        let model: ModelResponse = serde_json::from_str(json).unwrap();
        assert!(model.is_complete());
    }

    #[test]
    fn status_matching_failed() {
        let json = r#"{"modelId":"m1","status":"Failed","current":{"outcomes":[],"drivers":[]}}"#;
        let model: ModelResponse = serde_json::from_str(json).unwrap();
        assert!(model.is_failed());
        assert!(!model.is_complete());
        assert!(!model.is_running());
    }

    #[test]
    fn status_matching_running() {
        let json = r#"{"modelId":"m1","status":"Running","current":{"outcomes":[],"drivers":[]}}"#;
        let model: ModelResponse = serde_json::from_str(json).unwrap();
        assert!(model.is_running());
        assert!(!model.is_complete());
        assert!(!model.is_failed());
    }

    #[test]
    fn create_model_response_model_id_is_array() {
        let json = r#"{"modelId":["abc","def"],"question":"q","outcomes":["a","b"],"additionalContext":"c","type":"Advanced","batchId":null}"#;
        let resp: CreateModelResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.model_id.len(), 2);
        assert_eq!(resp.first_model_id(), "abc");
        assert_eq!(resp.model_type, "Advanced");
    }

    #[test]
    fn report_poll_response_ready() {
        let json = r#"{"status":"completed","result":"report content"}"#;
        let resp: ReportPollResponse = serde_json::from_str(json).unwrap();
        assert!(resp.is_ready());
    }

    #[test]
    fn report_poll_response_not_ready() {
        let json = r#"{"status":null,"result":null}"#;
        let resp: ReportPollResponse = serde_json::from_str(json).unwrap();
        assert!(!resp.is_ready());
    }
}
