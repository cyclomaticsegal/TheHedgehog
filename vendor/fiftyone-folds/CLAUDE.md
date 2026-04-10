# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Rust client library for the 51Folds Bayesian modelling API (`https://api.51folds.ai`). Follows Mode B (Startup/Evolving API) from the sdk-architect methodology — the SDK absorbs API quirks so consumers don't have to.

Sibling implementations exist in Python (`../51F-SDK-PYTHON/`) and TypeScript (`../51F-SDK-TYPESCRIPT/`). Both are complete and serve as reference implementations for identical behaviour.

## Authoritative source

The API Kit at `../API-KIT/` is the single source of truth. Read these before writing any code:

| Need | File |
|---|---|
| Non-negotiable rules (12 items) | `../API-KIT/CLAUDE.md` |
| Response schemas to type | `../API-KIT/SDK-GUIDE.md` |
| Full endpoint reference | `../API-KIT/docs/api-reference.md` |
| Gotchas | `../API-KIT/docs/gotchas.md` |
| OpenAPI spec | `../API-KIT/openapi/swagger.json` |
| Test fixtures (7 JSON files) | `../API-KIT/examples/responses/` |

Do not duplicate API documentation here — reference the Kit.

## Build commands

```bash
cargo build                    # compile
cargo test                     # run all tests
cargo test test_name           # run a single test
cargo clippy -- -D warnings    # lint (treat warnings as errors)
cargo fmt                      # format
cargo fmt -- --check           # check formatting without modifying
cargo doc --no-deps --open     # generate and view docs
```

## Crate name and structure

Crate: `fiftyone-folds` (lib crate, not binary).

```
src/
  lib.rs              Public API surface: FoldsClient, FoldsClientBuilder, re-exports
  client.rs           HttpTransport — auth, retry, envelope unwrap, error parsing
  types.rs            All request/response structs with serde, stateDescriptors deserializer
  errors.rs           FoldsError enum + FieldError struct
  constants.rs        All magic numbers and default values
  validation.rs       Client-side validation before network calls
  polling.rs          PollConfig struct for configurable intervals/timeouts
  resources/
    mod.rs            Re-exports ModelsResource, CreditsResource
    models.rs         ModelsResource — all model endpoints + inline polling loops
    credits.rs        CreditsResource — balance and transactions
tests/
  fixtures/           7 JSON fixtures copied from API-KIT/examples/responses/
  test_fixtures.rs    Deserialization tests against all fixtures
```

## Key design decisions for Rust

- **reqwest** with tokio runtime for async HTTP. Expose both async and blocking clients (reqwest supports both via feature flags).
- **serde + serde_json** for all serialisation. Derive `Serialize`/`Deserialize` on every request and response type. Use `#[serde(rename_all = "camelCase")]` since the API uses camelCase field names.
- **thiserror** for the error enum. Map HTTP status codes to typed variants.
- **uuid** crate for idempotency key generation.
- **Zero-copy where practical** but correctness over performance — own all strings in response types.
- **Builder pattern** for `FoldsClient` construction (token, base_url, timeout).
- **No feature-gated response fields.** Every response struct includes all fields the API can return. Use `Option<T>` for fields that are conditionally present (e.g., driver context, justification).
- **`stateDescriptors`**: Custom deserialiser that parses the JSON-encoded string into `Vec<StateDescriptor>` automatically. Never expose the raw string.

## Non-negotiables

These come from the API Kit. Violating any of them produces broken behaviour.

1. **Base URL** defaults to `https://api.51folds.ai`. Override via `FOLDS_BASE_URL` env var.
2. **Auth token** is `at_sk_...` (not a browser JWT). Read from `API_TOKEN` env var. Fail loudly if missing or doesn't start with `at_sk_`.
3. **`type` values are capitalised**: `"Overview"`, `"Insight"`, `"Advanced"`. Use a Rust enum with serde rename. Reject anything else client-side.
4. **`X-Idempotency-Key`** header on every `POST /api/v1/models`. Fresh UUID per request. Reuse same key across retries.
5. **Response envelope** — unwrap `.data` automatically in the HTTP client layer.
6. **Model creation is async** (202 Accepted). Poll until status matches `"Successed"`. Accept both `Successed` and `Succeeded` case-insensitively.
7. **Default to maximum richness**: both generate flags true on creation, both Include flags true on fetch, type defaults to Advanced.
8. **Both error shapes** must be handled: `validates[]` pattern and `reason[]` pattern.
9. **Client-side validation**: question >= 10 chars, 2-5 outcomes, additionalContext <= 300 words, type must be valid enum variant.

## Error handling

```
FoldsError (enum)
  ├── Authentication       — 401
  ├── PermissionDenied     — 403
  ├── NotFound             — 404
  ├── Validation { fields } — 400 (with parsed validates[])
  ├── RateLimit            — 429 (auto-retried: 2s base, 60s cap, 5 attempts)
  ├── Server               — 500 (auto-retried: 3 attempts)
  ├── Network              — connection/timeout errors
  ├── PollTimeout          — polling exceeded max wait
  └── ModelBuildFailed     — model status reached "Failed"
```

## Status matching

The API misspells "Succeeded" as "Successed". Match defensively:

```rust
fn is_complete(status: &str) -> bool {
    matches!(status.to_lowercase().as_str(), "successed" | "succeeded")
}
```

## Polling contract

- Model builds: poll every 60s, hard timeout 35 minutes.
- Report generation: poll every 60s, hard timeout 10 minutes.
- Provide both `create_and_wait()` (blocks until done) and `create()` (returns ID immediately).

## Test strategy

- Copy all 7 fixture files from `../API-KIT/examples/responses/` into `tests/fixtures/`.
- Every response struct must roundtrip-deserialise its corresponding fixture with zero field loss.
- Test `stateDescriptors` JSON string parsing.
- Test both error shapes parse correctly.
- Test client-side validation rejects bad input.
- Test defensive status matching.
- Use `mockito` or `wiremock` for HTTP-level tests.

## Dependencies (expected Cargo.toml)

```toml
[dependencies]
reqwest = { version = "0.12", features = ["json"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
uuid = { version = "1", features = ["v4"] }

[dev-dependencies]
tokio = { version = "1", features = ["full", "test-util"] }
wiremock = "0.6"
```
