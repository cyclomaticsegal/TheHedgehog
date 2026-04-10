# Spec: 51Folds Integration

**Status:** Planned — start here next session  
**Date:** 2026-04-06  
**Branch:** `hedgehog-poc`  
**Preceded by:** ADR-0007 (rolled back Dexter/tabs; decided 51Folds is the next integration target)

---

## Goal

Take the Hedgehog LLM regime analysis and use it to create Bayesian models in 51Folds — turning the AI's "Watch For" signal into a testable, probability-tracked hypothesis.

---

## Concept

The LLM already outputs a structured "Watch For" line — one specific signal that would change the regime assessment. That is a hypothesis. We extend the prompt to make it explicit and structured, then POST it to the 51Folds API as a model question with 2–4 outcomes.

---

## Phase 0 — Broaden the analysis input and enable web search

### 0a — Send all instruments to the LLM, not just the selected ones

`assemble_user_message()` currently only includes instruments in `overlay_instruments` (the chart selection). Change it to always receive the full instrument snapshot — selected instruments with their 30-day changes, plus a separate list of unselected instruments with their 30-day changes.

The unselected instruments must be labelled with explicit framing so the LLM understands its role:

> **Other available instruments (not in the user's current chart view)**: The user has not selected these for analysis, but their current market behaviour may be relevant to the regime or to the instruments the user *has* selected. Include them in your assessment only if they materially affect the interpretation — for example, crude oil price action affecting gold miners, or copper signalling a demand shock that context the user's gold vs VIX view. Do not analyse them for their own sake.

### 0b — Add web search tool to the API request

Add the provider's native search tool to each API request body so the LLM can fetch current macro context before forming its assessment.

**Anthropic:** Add to the request body:
```json
"tools": [{"type": "web_search_20250305"}]
```

**OpenAI:** Add to the request body:
```json
"tools": [{"type": "web_search_preview"}]
```

The LLM decides when and what to search. No tool-use loop needs to be built in Rust — both providers handle search execution natively and return the final response in the same envelope.

Update `assemble_system_prompt()` to instruct the LLM to use search for current macro context:

> Before classifying the regime, use web search to get a current picture of the macro environment — recent VIX moves, commodity price action, and any relevant geopolitical or economic developments. If instruments not in the current chart view are doing something notable in the context of the regime, flag them explicitly in your assessment.

### Files changed in Phase 0

| File | Change |
|------|--------|
| `src/ai.rs` | `assemble_user_message()` receives full instrument list; search tool added to `call_anthropic()` and `call_openai()` request bodies; search instruction added to `assemble_system_prompt()` |
| `src/app.rs` | Pass full instrument list (selected + unselected with changes) into `assemble_user_message()` |

---

## Phase 1 — Extend the LLM prompt to emit a structured hypothesis

Modify `assemble_system_prompt()` in `src/ai.rs`.

Add three new required fields to the response template **after** "Watch For":

```
**Hypothesis**: Will crude oil remain above $78 over the next 30 days?
**Hypothesis Outcomes**: Crude holds above $78 | Crude falls below $78 — demand destruction | Crude spikes above $95 — supply escalation
**Hypothesis Context**: Geopolitical spike regime driven by supply-side shock; VIX elevated at 28; copper diverging from energy signals. Crude is the key confirming/negating signal.
```

Rules to add to the prompt:
- `Hypothesis` — single forward-looking question, time-bounded (7–90 days)
- `Hypothesis Outcomes` — 2–4 pipe-separated outcomes, each ≤ 60 chars
- `Hypothesis Context` — 1–2 sentences: the data conditions motivating this hypothesis

---

## Phase 2 — Parse the hypothesis

Add to `src/models.rs`:

```rust
#[derive(Debug, Clone)]
pub struct ParsedHypothesis {
    pub question: String,
    pub outcomes: Vec<String>,
    pub context: String,
}
```

Add to `src/ai.rs`:

```rust
pub fn parse_hypothesis(response: &str) -> Option<ParsedHypothesis>
```

Plain string parsing on the `**Hypothesis**:`, `**Hypothesis Outcomes**:`, `**Hypothesis Context**:` markers. Store in `DashboardApp` as `parsed_hypothesis: Option<ParsedHypothesis>`, populated when a new AI response arrives in `poll_ai()`.

---

## Phase 3 — New module `src/folds.rs`

Thin `reqwest::blocking::Client` wrapper (same pattern as `src/ai.rs`).

```rust
pub struct FoldsCreateRequest {
    pub question: String,
    pub outcomes: Vec<String>,
    pub additional_context: String,
    pub model_type: String,   // "Overview" | "Insight"
    // both always set to true — 51Folds auto-generates factor explanations and insights
    pub generate_driver_content: bool,
    pub generate_take_away_content: bool,
}

pub struct FoldsModelSummary {
    pub id: String,
    pub status: String,       // "processing" | "succeeded" | "failed"
    pub outcomes_with_probabilities: Vec<(String, f64)>,
}

pub fn create_model(base_url: &str, api_key: &str, req: FoldsCreateRequest) -> Result<String>
pub fn get_model(base_url: &str, api_key: &str, model_id: &str) -> Result<FoldsModelSummary>
```

Key implementation details:
- Base URL: `https://api.51folds.ai` (hardcoded; can be overridden via `FOLDS_BASE_URL` env var)
- Auth: `Authorization: Bearer {api_key}` header
- `X-Idempotency-Key` header on POST: generate with `uuid::Uuid::new_v4()` — add `uuid` crate to `Cargo.toml`
- Response envelope: `{ "success": bool, "data": {...} }` — check `success` field, surface `error` field on failure
- `create_model` returns the model `id` string from `data.id`
- `get_model` returns a `FoldsModelSummary` from `data` (id, status, outcomes_with_probabilities)

---

## Phase 4 — Settings

Add to `ApiKeys` in `src/models.rs`:
```rust
pub folds_api_key: String,
```

Add `has_folds()` method (same pattern as `has_fred()`).

Add to `.env.example`:
```
FOLDS_API_KEY=at_sk_...
```

Add to `ApiKeys::from_env()`:
```rust
folds: std::env::var("FOLDS_API_KEY").unwrap_or_default(),
```

Wire into the settings sidebar under a new "51Folds" collapsible section (password TextEdit, same pattern as Anthropic/OpenAI keys).

---

## Phase 5 — Async state in `DashboardApp`

Add to struct (session-only, not persisted):

```rust
parsed_hypothesis: Option<ParsedHypothesis>,  // LLM-generated, read-only source of truth
draft_hypothesis: Option<ParsedHypothesis>,   // user-editable copy, populated from parsed_hypothesis on arrival
folds_model_id: Option<String>,
folds_model_status: Option<String>,
folds_in_flight: bool,
folds_rx: Option<Receiver<FoldsResult>>,
```

When a new AI response arrives in `poll_ai()`, populate both `parsed_hypothesis` and `draft_hypothesis` from the parsed result. `draft_hypothesis` is what the user edits and what gets POSTed to 51Folds. `parsed_hypothesis` is kept so the UI can show a "Reset to AI suggestion" option if the user wants to revert their edits.

Where:
```rust
enum FoldsResult {
    Created(String),                    // model ID — poll for status next
    StatusUpdate(FoldsModelSummary),    // polled status
    Failed(String),                     // error message
}
```

Spawn a thread on "Create Model" button click. Thread calls `folds::create_model()`, then polls `folds::get_model()` every 5 seconds until `status == "succeeded"` or `"failed"`. Sends `FoldsResult` events back via mpsc channel. App polls in `update()` (same pattern as `poll_ai()`).

---

## Phase 6 — UI in the AI panel

In `render_ai_panel_contents()`, below the response markdown, add a "51Folds" section.

The hypothesis is always editable before posting. The LLM's suggestion pre-fills the fields; the user can tweak any field or clear and rewrite from scratch. This is the only path to posting — there is no one-click "just use the LLM's hypothesis" shortcut.

**When hypothesis is available + folds key is set:**
```
── 51Folds ──────────────────────────────────────────
Question:  [ Will crude oil remain above $78 over the  ]
           [ next 30 days?                              ]

Outcomes:  [ Crude holds above $78                     ]  [×]
           [ Crude falls below $78 — demand destruction ]  [×]
           [ Crude spikes above $95 — supply escalation ]  [×]
           [ + Add outcome ]

Context:   [ Geopolitical spike regime; VIX at 28;     ]
           [ copper diverging from energy signals.      ]

           (Overview) (Insight)   [ Reset to AI suggestion ]
           [ → Create 51Folds Model ]
```

Each outcome is an individual editable text field with a remove button. "Add outcome" appends a new blank field (max 4). "Reset to AI suggestion" repopulates all fields from `parsed_hypothesis`.

**While processing:**
```
Model abc123 — processing ⟳
```

**When succeeded:**
```
Model abc123 — ✓ succeeded
  Crude holds above $78:        48%
  Crude falls below $78:        38%
  Crude spikes above $95:       14%
```

**When no folds key is configured:**
```
(muted) Configure 51Folds key in settings to create a model from this analysis.
```

---

## Files to change

| File | Change |
|------|--------|
| `src/ai.rs` | Full instrument list in user message; web search tool in request bodies; search instruction in system prompt; extend response template; add `parse_hypothesis()` |
| `src/models.rs` | Add `ParsedHypothesis`; add `folds_api_key` to `ApiKeys` |
| `src/folds.rs` | **New module** — HTTP client |
| `src/main.rs` | Add `mod folds;` |
| `src/app.rs` | Pass full instrument list into analysis; new state fields; poll loop; UI in AI panel; settings section |
| `.env.example` | Add `FOLDS_API_KEY` |
| `Cargo.toml` | Add `uuid` crate |

---

## Notes

Driver state management is handled automatically by the 51Folds backend at model creation time — no `PATCH /api/v1/models/{id}/drivers` call is required from Hedgehog. The create→poll→display loop is the complete integration.

---

## API reference

Full API docs: See the 51Folds API-KIT repository (private) — `docs/api-reference.md`  
Swagger: See the 51Folds API-KIT repository (private) — `openapi/swagger.json`  
Credentials: Stored in `.secret.env` within the API-KIT repository (gitignored, never committed)  
Prior model example (BV): Australian election model — 15 drivers, 3 outcomes, Advanced type
