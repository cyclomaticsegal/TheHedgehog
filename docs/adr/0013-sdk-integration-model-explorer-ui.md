# ADR 0013: 51Folds Rust SDK Integration, Rich Model Explorer, and Tabbed Central Panel

**Date:** 2026-04-10
**Status:** Accepted
**Extends:** ADR 0008 (51Folds integration), ADR 0012 (persistence and editor consolidation)

## Context

ADRs 0008 and 0012 established the initial 51Folds integration: the LLM generates a hypothesis, the user submits it, and outcome probabilities come back. However, the implementation had three significant limitations:

1. **Hand-rolled HTTP client.** `src/folds.rs` (235 lines) manually constructed reqwest requests, parsed JSON envelopes, and managed polling loops. Meanwhile, a production Rust SDK (`fiftyone-folds`) existed at `../51Folds/51F-SDK-RUST/` that already handled authentication, retry with exponential backoff (429/5xx), idempotency keys, response envelope unwrapping, and the "Successed" status typo. The hand-rolled client only extracted `model_id`, `status`, and outcome probabilities — discarding the rich model data (drivers, edges, causal context, justifications, state descriptors) that the API returns.

2. **Minimal post-creation UI.** After a model completed, the AI side panel showed plain text: `"Model abc123 — succeeded"` followed by outcome labels and percentages. No probability bars, no driver information, no ability to explore the Bayesian network's causal structure, and no way to change driver states and see how outcome probabilities shift.

3. **No linkage between AI analyses and 51Folds models.** The `folds_models` table stored model_id, status, and question, but had no foreign key back to the `ai_inferences` row that spawned it. Loading a historical inference from the sidebar did not restore its linked model — the user would see "No hypothesis in this analysis" even if a completed model existed for it.

4. **Model type mismatch.** The app hardcoded `"Insights"` as the model tier, but the SDK validates against `["Overview", "Insight", "Advanced"]`. The API was lenient about this, but the SDK would reject it client-side. Additionally, the polling ceiling was 2 hours — far too long for models that complete in 25-30 minutes.

5. **Cramped results display.** Even after building rich UI elements (outcome bars, driver list), the 360px AI side panel was too narrow. Outcome labels were barely readable, driver details were squashed, and there was no space for a future causal graph visualization.

## Decision

### 1. Replace hand-rolled HTTP with the Rust SDK

`src/folds.rs` was fully rewritten. The old 235 lines of manual reqwest/JSON code were replaced with ~300 lines that bridge between the app's synchronous `std::thread::spawn` pattern and the SDK's async (tokio) API. Three entry points:

- **`create_and_poll()`** — spawns a single-threaded tokio runtime (`Builder::new_current_thread().enable_time().enable_io()`), calls `client.models().create()`, sends `Created(model_id)` on the mpsc channel, then `client.models().wait_until_complete()` with a 35-minute timeout. On completion, persists the full `ModelResponse` JSON to the database and sends `Completed(Box<ModelResponse>)`.
- **`patch_drivers()`** — calls `client.models().patch_drivers()` which returns the updated `ModelResponse` synchronously (no polling needed for driver re-evaluation). Sends `Completed(Box<ModelResponse>)`.
- **`resume_poll()`** — for startup resume sweep. No UI channel, only database updates. Uses remaining time from the 35-minute ceiling.

The token is always passed explicitly via `FoldsClient::new(Some(api_key), None, None)` — the app stores it as `FOLDS_API_KEY`, not the SDK's default `API_TOKEN` env var.

### 2. Model type changed to Advanced, timeouts tightened

- `FOLDS_MODEL_TYPE` changed from `"Insights"` to `"Advanced"` — the richest tier with the most drivers and deepest analysis, ~25-30 min build time.
- `FOLDS_SUSPECT_AFTER_SECS` reduced from 3600 (1h) to 1500 (25 min).
- `FOLDS_UNDISCLOSED_AFTER_SECS` reduced from 7200 (2h) to 2100 (35 min).

### 3. Expanded FoldsTask to carry full model response

The `FoldsTask` struct was expanded from 6 fields to hold the complete SDK response:

- `model: Option<Box<ModelResponse>>` — the full model with drivers, edges, outcomes, context, justifications.
- `draft_drivers: Vec<DraftDriverState>` — mutable copies of driver states for the re-evaluate UI. Each tracks `code`, `name`, `selected_state`, `original_state`, `state_options` (from state descriptors), and `expanded` flag.
- `previous_outcomes: Option<Vec<(String, f64)>>` — snapshot of outcome probabilities before a re-evaluate, for rendering before/after deltas.
- `reevaluating: bool` — distinguishes a driver re-evaluate (quick) from initial creation (slow).
- `load_from_json()` — deserializes a stored `ModelResponse` from the database, enabling instant restoration of completed models.

### 4. Database linkage between analyses and models

Four columns added to `folds_models` via additive migration (same pattern as the hypothesis columns in ADR 0012):

- `inference_id INTEGER REFERENCES ai_inferences(id)` — the FK linking a model back to the AI analysis that spawned it.
- `response_json TEXT` — the full serialized `ModelResponse` for reload on restart.
- `outcomes TEXT` — denormalized JSON array of `{label, probability}` for sidebar display.
- `short_summary TEXT` — the model's prose takeaway.

`save_inference()` now captures the returned row ID as `last_inference_id`, which is threaded through to `create_and_poll()` and written into the `folds_models` row. `load_historical_inference()` queries `load_folds_response_for_inference(inference_id)` and, if found, calls `folds_task.load_from_json()` so the model results appear immediately.

### 5. Tabbed central panel with model explorer

The central panel — previously chart-only — now supports two views controlled by `CentralView` enum:

- **Charts** — the existing VIX, correlation, and price charts (unchanged).
- **Model** — the 51Folds model explorer, with two sub-tabs controlled by `ModelTab` enum:
  - **Outcome** — question header, outcome probability bars (full-width, white bold labels, blue fills, percentage right-aligned), before/after delta annotations, and Take Away summary in a framed block.
  - **Drivers** — driver list sorted by influence, each with name + code badge, segmented state selector (blue fill for selected, amber name when modified), and expandable details (state descriptions, "Why was X selected?" with citations, "Why does this matter?", "What could shift?", "What should we monitor?"). Re-evaluate and Reset buttons at the bottom.

The toolbar gains Charts/51Folds tab selectors. Chart-specific controls (1M/3M/6M/1Y/All, zoom, Report) only render in Charts view. Outcome/Drivers sub-tabs only render in Model view. The 51Folds tab label turns blue when a completed model exists.

### 6. Compact side panel summary for completed models

When a model is complete, the AI side panel's 51Folds section shows a compact summary (model ID, outcome percentages as text, "View in 51Folds tab" button) instead of the full model results. The hypothesis editor, spinner, and error states remain in the side panel.

### 7. Auto-switch on model completion

The central view automatically switches to 51Folds/Outcome when:
- A model build completes (detected in `poll_folds()` by comparing `is_complete()` before and after poll).
- A historical inference with a linked model is loaded from the sidebar history.

### 8. Re-evaluate flow

Users can change driver states via the segmented selectors in the Drivers sub-tab and click Re-evaluate. This:
1. Snapshots current outcome probabilities into `previous_outcomes`.
2. Builds `Vec<DriverStateInput>` from modified drafts only.
3. Spawns a thread calling `folds::patch_drivers()` (synchronous API response, no polling).
4. On completion, the model is updated, draft drivers re-initialized, and outcome bars show "Previously: X.XX% up/down" deltas in green/red.

Reset restores all driver states to their original values and clears delta annotations.

## Files Changed

| File | Summary |
|---|---|
| `Cargo.toml` | Added `fiftyone-folds` (path dep) and `tokio` (rt + time) |
| `src/folds.rs` | Full rewrite: SDK bridge replacing hand-rolled HTTP |
| `src/app.rs` | `CentralView`/`ModelTab` enums, expanded `FoldsTask` with `DraftDriverState`, tabbed central panel, model explorer with Outcome/Drivers sub-tabs, compact side panel summary, auto-switch, re-evaluate flow, label color fixes |
| `src/models.rs` | Updated timeout constants (25/35 min), model type to "Advanced" |
| `src/storage.rs` | Column migrations (inference_id, response_json, outcomes, short_summary), new save/load methods |

## Consequences

- The app now consumes the full richness of the 51Folds API — drivers, causal context, justifications, state descriptors — not just outcome probabilities.
- Users can explore *why* the model assigns probabilities via driver details, and run "what if" scenarios via driver re-evaluation.
- Historical models are linked to their source analyses and survive restarts.
- The central panel is now a shared space, establishing the pattern for future additions (causal graph visualization, report viewer).
- The SDK handles retry, idempotency, and envelope parsing — removing ~100 lines of fragile hand-rolled HTTP logic.
- `tokio` is now a direct dependency (with `rt` + `time` features). The runtime is only created inside background threads — the egui main loop remains synchronous.
