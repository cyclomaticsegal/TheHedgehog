//! Deserialization tests against real API response fixtures.
//!
//! Every response type must roundtrip-deserialize its corresponding fixture
//! from API-KIT/examples/responses/ with zero field loss.

use fiftyone_folds::types::{
    ApiEnvelope, CreateModelResponse, JustificationResponse, ModelResponse, RevisionsResponse,
};

#[test]
fn deserialize_create_model_response() {
    let json = include_str!("fixtures/create-model.response.json");
    let envelope: ApiEnvelope<CreateModelResponse> = serde_json::from_str(json).unwrap();
    let resp = &envelope.data;

    assert!(
        !resp.model_id.is_empty(),
        "modelId should be a non-empty array"
    );
    assert_eq!(resp.first_model_id(), "Ja");
    assert!(resp.question.len() > 10);
    assert_eq!(resp.outcomes.len(), 3);
    assert!(!resp.additional_context.is_empty());
    assert_eq!(resp.model_type, "Advanced");
    assert!(resp.batch_id.is_none());
}

#[test]
fn deserialize_model_baseline() {
    let json = include_str!("fixtures/model-baseline.response.json");
    let envelope: ApiEnvelope<ModelResponse> = serde_json::from_str(json).unwrap();
    let model = &envelope.data;

    assert_eq!(model.model_id, "Ja");
    assert_eq!(model.status, "Successed");
    assert!(model.is_complete());
    assert!(!model.is_failed());
    assert!(!model.is_running());

    // Drivers present
    assert!(!model.drivers.is_empty(), "drivers should be populated");
    assert!(
        !model.drivers[0].state_descriptors.is_empty(),
        "stateDescriptors should be parsed from array"
    );
    assert!(
        !model.drivers[0].name.is_empty(),
        "driver names should be populated"
    );

    // Edges present
    assert!(!model.edges.is_empty());

    // Current state
    assert!(!model.current.outcomes.is_empty());
    assert!(!model.current.drivers.is_empty());

    // Probabilities sum roughly to 1
    let prob_sum: f64 = model
        .current
        .outcomes
        .iter()
        .filter_map(|o| o.probability)
        .sum();
    assert!(
        (prob_sum - 1.0).abs() < 0.01,
        "probabilities should sum to ~1.0, got {}",
        prob_sum
    );

    // Baseline: driver context should NOT be present (no Include flags)
    assert!(
        model.drivers[0].context.is_none(),
        "baseline model should not have driver context"
    );

    // Baseline: justification should NOT be present
    assert!(
        model.current.drivers[0].justification.is_none(),
        "baseline model should not have justification"
    );

    // Context and short_summary should be populated
    assert!(!model.context.is_empty());
    assert!(!model.short_summary.is_empty());
}

#[test]
fn deserialize_model_rich() {
    let json = include_str!("fixtures/model-rich.response.json");
    let envelope: ApiEnvelope<ModelResponse> = serde_json::from_str(json).unwrap();
    let model = &envelope.data;

    assert_eq!(model.model_id, "Ja");
    assert!(model.is_complete());

    // IncludeDriverContext=true: context should be present on drivers
    let first_driver = &model.drivers[0];
    assert!(
        first_driver.context.is_some(),
        "rich model should have driver context"
    );
    let ctx = first_driver.context.as_ref().unwrap();
    assert!(!ctx.importance.is_empty());
    assert!(!ctx.shifts.is_empty());
    assert!(!ctx.monitor.is_empty());

    // IncludeDriverJustification=true: justification on current.drivers
    let first_current_driver = &model.current.drivers[0];
    assert!(
        first_current_driver.justification.is_some(),
        "rich model should have driver justification"
    );
    let just = first_current_driver.justification.as_ref().unwrap();
    assert!(!just.content.is_empty());
    assert!(!just.citations.is_empty());
    assert!(!just.citations[0].source.is_empty());
}

#[test]
fn deserialize_justification_response() {
    let json = include_str!("fixtures/justification.response.json");
    let envelope: ApiEnvelope<JustificationResponse> = serde_json::from_str(json).unwrap();
    let resp = &envelope.data;

    assert_eq!(resp.model_id, "Ja");
    assert!(!resp.raw_justification_file.is_empty());
    // The justification is markdown prose, should contain citations
    assert!(resp.raw_justification_file.contains("http"));
}

#[test]
fn deserialize_revisions_response() {
    let json = include_str!("fixtures/revisions.response.json");
    // This fixture has the full envelope with success/errorCode/etc alongside data
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    let data = &value["data"];

    let resp: RevisionsResponse = serde_json::from_value(data.clone()).unwrap();

    assert_eq!(resp.model_id, "Ja");
    assert!(!resp.outcomes.is_empty());
    assert!(!resp.revisions.is_empty());

    let rev = &resp.revisions[0];
    assert_eq!(rev.id, 1);
    assert_eq!(rev.trigger, "system");
    assert!(!rev.outcomes.is_empty());
    assert!(!rev.drivers.is_empty());

    // Probabilities should sum to ~1.0
    let prob_sum: f64 = rev.outcomes.iter().map(|o| o.probability).sum();
    assert!(
        (prob_sum - 1.0).abs() < 0.01,
        "revision probabilities should sum to ~1.0, got {}",
        prob_sum
    );
}

// Schema and diagnostic fixtures are raw string content (not enveloped),
// so we test them by constructing the envelope wrapper.

#[test]
fn deserialize_schema_response_constructed() {
    use fiftyone_folds::types::SchemaResponse;

    let raw_content = include_str!("fixtures/schema-example.json");
    let envelope = serde_json::json!({
        "data": {
            "modelId": "test-model",
            "rawSchemaFile": raw_content
        }
    });

    let resp: ApiEnvelope<SchemaResponse> = serde_json::from_value(envelope).unwrap();
    assert_eq!(resp.data.model_id, "test-model");
    assert!(!resp.data.raw_schema_file.is_empty());
    assert!(resp.data.raw_schema_file.contains("Dependent Variable"));
}

#[test]
fn deserialize_diagnostic_response_constructed() {
    use fiftyone_folds::types::DiagnosticResponse;

    let raw_content = include_str!("fixtures/diagnostic-example.json");
    let envelope = serde_json::json!({
        "data": {
            "modelId": "test-model",
            "rawDiagnosticsFile": raw_content
        }
    });

    let resp: ApiEnvelope<DiagnosticResponse> = serde_json::from_value(envelope).unwrap();
    assert_eq!(resp.data.model_id, "test-model");
    assert!(!resp.data.raw_diagnostics_file.is_empty());
    assert!(resp.data.raw_diagnostics_file.contains("Diagnostic Report"));
}
