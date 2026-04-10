# ADR-0008: 51Folds Integration — Hypothesis Generation and Bayesian Model Creation

**Status:** Accepted  
**Date:** 2026-04-06  
**Branch:** `hedgehog-poc`

---

## Context

ADR-0007 established 51Folds as the sole advanced analysis pathway for The Hedgehog, replacing the Dexter agent. The next logical step is a concrete integration: taking the LLM's regime analysis and converting it into a testable, probability-tracked Bayesian model in 51Folds.

The AI analysis panel already produces a "Watch For" signal — one specific observable that would change the regime assessment. That is a hypothesis. The question was how to bridge the gap from a qualitative regime read to a structured 51Folds model.

Several design questions were resolved during planning:

**Static vs. LLM-generated hypotheses**: The system prompt is a static template that strictly controls the LLM's output format. Adding hypothesis fields to this template was the natural extension — the LLM already reasons about forward-looking signals, it just wasn't asked to formalise them.

**Who decides the hypothesis**: The LLM proposes, the user disposes. The UI pre-fills editable fields from the LLM's suggestion, but every field is fully editable before posting to 51Folds. The user can tweak or completely rewrite the hypothesis. There is no one-click bypass — all paths to creating a 51Folds model go through the editable fields.

**Breadth of analysis input**: The LLM was previously only given data for instruments the user had selected on the chart. A user analysing gold vs VIX wouldn't know that crude oil was doing something anomalous that contextualised the regime. This was addressed by sending all instruments to the LLM, with unselected instruments explicitly framed so the LLM understands its role: flag them only if they materially affect the interpretation of the chosen instruments.

**Live macro context**: The LLM's knowledge base is static (baked-in KB chunks seeded at first launch). Regime analysis benefits from knowing what is happening right now — recent VIX moves, current commodity price action, geopolitical developments. Neither Anthropic nor OpenAI model endpoints have internet access by default; however, both providers expose native first-party search tools (`web_search_20250305` for Anthropic, `web_search_preview` for OpenAI) that are declared in the API request body. The provider handles search execution server-side and returns the final response in the same envelope — no tool-use loop is required in the client code.

**Driver management**: 51Folds driver state updates (`PATCH /api/v1/models/{id}/drivers`) were initially scoped as future work. Confirmed during implementation that driver assignment is handled automatically by the 51Folds backend at model creation time. No driver mapping from Hedgehog is required.

---

## Decision

Integrate 51Folds directly into the AI analysis panel as a post-analysis section. The integration is structured across six phases:

### Phase 0 — Broaden analysis input and enable web search

**0a**: `assemble_user_message()` now receives all instruments, not just the chart selection. Unselected instruments are passed with explicit framing instructing the LLM to flag them only if they materially affect the chosen instruments' interpretation (e.g. crude oil affecting gold miners).

**0b**: Both `call_anthropic()` and `call_openai()` include the provider's native search tool in the request body. The system prompt instructs the LLM to use web search before classifying the regime. Response parsers updated to find the text content block across possibly multiple content blocks.

### Phase 1 — Extend LLM output template

Three new required fields added to `assemble_system_prompt()` after "Watch For":

- `**Hypothesis**` — single forward-looking question, time-bounded 7–90 days
- `**Hypothesis Outcomes**` — 2–4 pipe-separated outcomes, each ≤ 60 chars
- `**Hypothesis Context**` — 1–2 sentences: the data conditions motivating the hypothesis

### Phase 2 — Parse the hypothesis

`ParsedHypothesis` struct added to `src/models.rs`. `parse_hypothesis()` and `extract_field()` added to `src/ai.rs`. On each successful analysis, `poll_ai()` populates both `parsed_hypothesis` (LLM source, read-only) and `draft_hypothesis` (user-editable copy). The split preserves the LLM's original suggestion for the "Reset to AI" path.

### Phase 3 — 51Folds HTTP client (`src/folds.rs`)

Thin `reqwest::blocking::Client` wrapper with two functions:

- `create_model()` — POSTs with `generateDriverContent: true` and `generateTakeAwayContent: true`. Handles the API's array-valued `modelId` response by taking the first element.
- `get_model()` — GETs model status and reads probabilities from `data.current.outcomes[].{label, probability}`.

Key implementation detail: the 51Folds API requires `X-Idempotency-Key` as a valid UUID on POST. Generated with `uuid::Uuid::new_v4()`.

Actual API response shapes confirmed by live API calls:
- `POST /api/v1/models` returns `data.modelId` as an array: `["Iu"]`
- `GET /api/v1/models/{id}` returns status as `"Running"` (processing) or `"Successed"` (note: API typo) and probabilities at `data.current.outcomes`

### Phase 4 — Settings

`folds_api_key` added to `ApiKeys`. `has_folds()` method added. `.env.example` and `save_keys_to_env()` updated. A "51Folds" collapsing sidebar section added with a password TextEdit for the key.

### Phase 5 — Async state and polling

`FoldsTask` struct manages the background thread lifecycle (same mpsc channel pattern as `LlmTask`). On "Create Model" click, a thread is spawned that calls `create_model()` then polls `get_model()` every 5 seconds until status is `"Successed"` or `"Failed"`, sending `FoldsResult` events back via channel. `poll_folds()` is called each frame in `update()`.

### Phase 6 — UI in the AI panel

`render_folds_section()` renders below the analysis markdown. States:

- **No key configured**: muted hint to add key in settings
- **No hypothesis**: muted hint to re-analyze
- **Fields ready**: editable question, per-outcome text fields with add/remove, editable context, model type selector (Overview / Insight), "Reset to AI" link, "→ Create 51Folds Model" button
- **Processing**: model ID + spinner
- **Succeeded**: model ID + probability table
- **Failed / error**: error message in red

---

## Files Changed

| File | Change |
|------|--------|
| `src/ai.rs` | Full instrument list in user message; web search tools in request bodies; response parsing for multi-block content; hypothesis fields in system prompt; `parse_hypothesis()` |
| `src/models.rs` | `ParsedHypothesis` struct; `folds_api_key` in `ApiKeys`; `has_folds()` method |
| `src/folds.rs` | **New module** — `create_model()`, `get_model()`, `send_and_parse()` |
| `src/main.rs` | `mod folds` |
| `src/app.rs` | Unselected instrument changes; `FoldsTask`, `FoldsResult`, `FoldsModelType`; new state fields; `start_folds_create()`, `poll_folds()`; `render_folds_section()`; 51Folds sidebar section; `save_keys_to_env()` updated |
| `.env.example` | `FOLDS_API_KEY` entry |
| `Cargo.toml` | `uuid = { version = "1", features = ["v4"] }` |

---

## Consequences

- Every AI analysis now produces a structured hypothesis ready to post to 51Folds.
- The user retains full control — the LLM proposes, the user edits, nothing is posted without explicit action.
- The analysis is richer: the LLM sees all instruments and can access live macro context via web search before forming its regime assessment.
- The create→poll→display loop is the complete integration. Driver state management is handled by 51Folds automatically.
- Future work: driver state updates from Hedgehog instrument signals once the 51Folds driver API is fully stabilised; evidence submission as new VIX data arrives.
