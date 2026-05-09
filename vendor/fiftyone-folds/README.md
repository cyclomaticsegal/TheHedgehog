# 51Folds Rust SDK

[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org)

Typed, async Rust client for the [51Folds](https://51folds.ai) Bayesian modelling API.

You give it a question, 2-5 possible outcomes, and a paragraph of domain context. The API builds a causal Bayesian network — drivers, edges, outcome probabilities — that you can inspect, update with evidence, and render into reports. This SDK handles the rest: authentication, polling, retries, validation, and parsing every API response into strongly-typed Rust structs.

---

## Installation

```toml
[dependencies]
fiftyone-folds = "0.1"
tokio = { version = "1", features = ["full"] }
```

## Quick start

```rust
use fiftyone_folds::FoldsClient;

#[tokio::main]
async fn main() -> Result<(), fiftyone_folds::FoldsError> {
    // Reads API_TOKEN from environment
    let client = FoldsClient::new(None, None, None)?;

    // Build a model (blocks until the async build completes, ~10-25 min)
    let model = client.models().create_and_wait(
        "Will the product launch succeed in Q2 2026?",
        &["Highly Successful".into(), "Moderately Successful".into(), "Unsuccessful".into()],
        "B2B SaaS targeting hospital procurement committees. The incumbent \
         holds 40% share. Our pricing is 20% below. Three signed LOIs \
         representing 400 beds. Sales cycles run 9-14 months...",
        None, None, None, None, None,  // all defaults: Advanced type, full richness
    ).await?;

    // Read the result
    println!("{}", model.short_summary);
    for outcome in &model.current.outcomes {
        println!("  {}: {:.1}%", outcome.label, outcome.probability.unwrap_or(0.0) * 100.0);
    }

    Ok(())
}
```

```bash
export API_TOKEN=at_sk_your_key_here
cargo run
```

---

## What the SDK handles for you

The 51Folds API has real-world quirks. This SDK absorbs them so your code doesn't have to.

| Quirk | What the SDK does |
|---|---|
| Model builds are async (202 Accepted, 10–25 min) | `create_and_wait()` polls automatically; `create()` returns immediately if you prefer |
| Status field is misspelled (`"Successed"` not `"Succeeded"`) | `is_complete()` matches both spellings, case-insensitive |
| `stateDescriptors` comes back as a JSON string, not an array | Custom serde deserializer parses it into `Vec<StateDescriptor>` automatically |
| Two different error response shapes (`validates[]` vs `reason[]`) | Both parsed into a single `FoldsError::Validation` variant |
| `POST /models` requires `X-Idempotency-Key` | Generated automatically per request; reused across retries |
| Rate limits (429) and server errors (500) | Exponential backoff with jitter, configurable retry counts |
| Response wrapped in `{"data": ...}` envelope | Unwrapped automatically — you get the typed inner struct |
| `additionalContext` in request becomes `context` in response | Response types use the field names the API actually returns |

---

## Authentication

The API uses long-lived keys prefixed `at_sk_` — not browser JWTs.

```bash
export API_TOKEN=at_sk_your_key_here
```

Or pass explicitly:

```rust
let client = FoldsClient::builder()
    .api_token("at_sk_...")
    .build()?;
```

UAT environment:

```rust
let client = FoldsClient::builder()
    .api_token("at_sk_...")
    .base_url("https://api-uat.fiftyonefolds.ai")
    .build()?;
```

| Parameter | Env var | Default |
|---|---|---|
| `api_token` | `API_TOKEN` | *(required)* |
| `base_url` | `FOLDS_BASE_URL` | `https://api.51folds.ai` |
| `timeout` | — | 30s |

---

## Model lifecycle

### Create

```rust
// Async — returns immediately with model ID
let result = client.models().create(
    "Will X happen by 2028?",
    &["Yes".into(), "No".into(), "Partial".into()],
    "250+ words of domain context for best driver quality...",
    Some("Advanced"),  // or "Overview", "Insight"
    None, None, None,
).await?;
let model_id = result.first_model_id();

// Or block until the build finishes
let model = client.models().create_and_wait(
    "...", &outcomes, "...",
    None, None, None, None, None,
).await?;
```

### Inspect

```rust
let model = client.models().get("model-id", None, None).await?;

// Status (defensive — survives the API fixing its typo)
model.is_complete()  // "Successed" or "Succeeded", case-insensitive
model.is_failed()
model.is_running()

// Prose takeaway
println!("{}", model.short_summary);

// Outcome probabilities
for o in &model.current.outcomes {
    println!("{}: {:.1}%", o.label, o.probability.unwrap_or(0.0) * 100.0);
}

// Drivers — stateDescriptors already parsed from JSON string
for driver in &model.drivers {
    println!("{} — {}", driver.code, driver.name);
    for sd in &driver.state_descriptors {
        println!("  {}: {}", sd.name, sd.description);
    }
    // Analytical context (from IncludeDriverContext, on by default)
    if let Some(ctx) = &driver.context {
        println!("  Importance: {}", ctx.importance);
    }
}

// Justifications (from IncludeDriverJustification, on by default)
for ds in &model.current.drivers {
    if let Some(j) = &ds.justification {
        for paragraph in &j.content { println!("{}", paragraph); }
        for c in &j.citations { println!("[{}] {}", c.num, c.source); }
    }
}
```

### Update drivers

```rust
use fiftyone_folds::DriverStateInput;

// Replace all (PUT) — triggers re-inference
client.models().update_drivers("model-id", &[
    DriverStateInput { code: "CEDIG".into(), state: "High".into() },
    DriverStateInput { code: "GSWFIC".into(), state: "Medium".into() },
]).await?;

// Patch specific (PATCH) — triggers re-inference
client.models().patch_drivers("model-id", &[
    DriverStateInput { code: "CEDIG".into(), state: "Low".into() },
]).await?;
```

### Submit evidence

```rust
client.models().submit_evidence("model-id", &serde_json::json!({
    "evidence": "New trade agreement signed"
})).await?;
```

### Deep inspection

```rust
let schema  = client.models().schema("model-id").await?;        // Bayesian network (100–200 KB JSON)
let diag    = client.models().diagnostic("model-id").await?;     // 10-section diagnostic
let just    = client.models().justification("model-id").await?;  // Build-time justification (markdown)
let revs    = client.models().revisions("model-id").await?;      // Full revision history
```

### Reports

```rust
// Fire and forget
client.models().generate_report("model-id", "ExecutiveSummary").await?;

// Or block until ready
let report = client.models()
    .generate_report_and_wait("model-id", "ExecutiveSummary", None)
    .await?;
println!("{}", report.result.unwrap());
```

### Retry failed builds

```rust
client.models().retry("model-id").await?;
let model = client.models().wait_until_complete("model-id", None).await?;
```

### Credits

```rust
let balance = client.credits().me().await?;
println!("Credits: {}", balance.amount);

let txns = client.credits().transactions(None, None, None, None, None).await?;
for t in &txns.transactions {
    println!("{}: {} ({})", t.type_name, t.amount, t.timestamp);
}
```

---

## Smart defaults

The SDK defaults to maximum richness — opt *out*, not in:

| Setting | Default | To override |
|---|---|---|
| Model type | `"Advanced"` | `Some("Overview")` or `Some("Insight")` |
| `generateDriverContent` | `true` | `Some(false)` |
| `generateTakeAwayContent` | `true` | `Some(false)` |
| `IncludeDriverContext` | `true` | `Some(false)` on `get()` |
| `IncludeDriverJustification` | `true` | `Some(false)` on `get()` |

---

## Polling

Async builds (models and reports) are polled automatically by the `*_and_wait` methods.

| | Interval | Timeout |
|---|---|---|
| Model builds | 60s | 35 min |
| Reports | 60s | 10 min |

Override per call:

```rust
use fiftyone_folds::PollConfig;
use std::time::Duration;

let model = client.models().create_and_wait(
    "...", &outcomes, "...",
    None, None, None, None,
    Some(PollConfig {
        interval: Duration::from_secs(30),
        timeout: Duration::from_secs(1800),
    }),
).await?;
```

---

## Error handling

Every error is a variant of `FoldsError`, carrying the HTTP status code and raw response body where applicable. Use Rust's pattern matching:

```rust
use fiftyone_folds::FoldsError;

match client.models().get("bad-id", None, None).await {
    Ok(model) => { /* ... */ }
    Err(FoldsError::NotFound { message, .. }) => {
        eprintln!("Model not found: {}", message);
    }
    Err(FoldsError::Validation { field_errors, reasons, .. }) => {
        for fe in &field_errors { eprintln!("{}: {:?}", fe.key, fe.errors); }
        for r in &reasons { eprintln!("{}", r); }
    }
    Err(e) => eprintln!("{}", e),
}
```

| Variant | HTTP | Description |
|---|---|---|
| `Authentication` | 401 | Token missing, malformed, or not `at_sk_` |
| `PermissionDenied` | 403 | Token valid but wrong account |
| `NotFound` | 404 | Bad model ID or model still building |
| `Validation` | 400 | Bad payload; `field_errors` and `reasons` carry detail |
| `RateLimit` | 429 | Auto-retried (5x, exponential backoff, 2s–60s) |
| `Server` | 500+ | Auto-retried (3x, exponential backoff, 2s–60s) |
| `Network` | — | Connection, timeout, DNS failure |
| `PollTimeout` | — | Polling exceeded configured timeout |
| `ModelBuildFailed` | — | Model status reached `"Failed"` |

### Client-side validation

Inputs are validated before the network call. All failures are collected, not just the first:

| Field | Rule |
|---|---|
| `question` | Min 10 characters |
| `outcomes` | 2–5 items |
| `additional_context` | Hard max 300 words (warns below 250) |
| `type` | Exactly `"Overview"`, `"Insight"`, or `"Advanced"` (case-sensitive) |

---

## API reference

### `client.models()`

| Method | HTTP | Returns |
|---|---|---|
| `create(...)` | `POST /api/v1/models` | `CreateModelResponse` |
| `create_and_wait(...)` | `POST` + poll | `ModelResponse` |
| `wait_until_complete(id, config)` | poll | `ModelResponse` |
| `get(id, ctx?, just?)` | `GET /api/v1/models/{id}` | `ModelResponse` |
| `list(...)` | `GET /api/v1/models` | `serde_json::Value` |
| `schema(id)` | `GET /api/v1/models/{id}/schema` | `SchemaResponse` |
| `diagnostic(id)` | `GET /api/v1/models/{id}/diagnostic` | `DiagnosticResponse` |
| `justification(id)` | `GET /api/v1/models/{id}/justification` | `JustificationResponse` |
| `revisions(id)` | `GET /api/v1/models/{id}/revisions` | `RevisionsResponse` |
| `update_drivers(id, drivers)` | `PUT /api/v1/models/{id}/drivers` | `ModelResponse` |
| `patch_drivers(id, drivers)` | `PATCH /api/v1/models/{id}/drivers` | `ModelResponse` |
| `submit_evidence(id, evidence)` | `POST /api/v1/models/{id}/evidence` | `serde_json::Value` |
| `generate_report(id, type)` | `POST /api/v1/models/{id}/reports` | `ReportTriggerResponse` |
| `get_report(id, type)` | `GET /api/v1/models/{id}/reports` | `ReportPollResponse` |
| `generate_report_and_wait(id, type, config)` | `POST` + poll | `ReportPollResponse` |
| `retry(id)` | `POST /api/v1/models/{id}/retry` | `serde_json::Value` |

### `client.credits()`

| Method | HTTP | Returns |
|---|---|---|
| `me()` | `GET /api/v1/credits/me` | `CreditsResponse` |
| `transactions(...)` | `GET /api/v1/credits/transactions` | `TransactionsResponse` |

---

## Development

```bash
cargo build                    # compile
cargo test                     # 30 tests
cargo clippy -- -D warnings    # lint (zero warnings policy)
cargo fmt -- --check           # formatting check
cargo doc --no-deps --open     # generated docs
```

The test suite validates against fixture files from the [51Folds API Kit](../API-KIT/):

- **Fixture deserialization** — all 7 real API response fixtures (up to 188 KB each) deserialize with zero field loss
- **stateDescriptors parsing** — JSON string, pre-parsed array, null, and missing field
- **Status matching** — `"Successed"`, `"Succeeded"`, case variations, `"Failed"`, `"Running"`
- **Client-side validation** — every rule, every edge case, multiple-error collection
- **Doc-tests** — all code examples in doc comments compile

## Full API documentation

This crate is a typed client layer. For endpoint documentation, request/response field tables, gotchas, and integration patterns, see the [51Folds API Kit](../API-KIT/).

## License

All rights reserved. No license is granted.
