# ADR 0012: Analysis Quality Hardening, Persistent 51Folds Tracking, and AI Editor Consolidation

**Date:** 2026-04-07
**Status:** Accepted
**Supersedes:** None — extends ADR 0008 (51Folds integration) and ADR 0009 (hypothesis quality) with work that happened after those landed.

## Context

ADRs 0008 and 0009 landed the first cut of the 51Folds integration: the LLM produces a hypothesis, the user can submit it to 51Folds, and an outcome/probability result comes back. On the same day that ADR 0011 locked in the single-provider daily cache, live use of the end-to-end flow exposed several sharp edges that needed to be fixed together rather than one at a time, because many of them interacted:

1. **The LLM was inventing prices.** A real analysis on 2026-04-07 produced a hypothesis anchored at gold $2,100 when spot was over $4,600. The user message only carried 30-day percent changes, so the model had no absolute-level anchor and fell back on its training prior. The system prompt told it to web-search, but OpenAI had no search tool wired (we were hitting Chat Completions, which does not expose `web_search_preview`).
2. **The Hypothesis Context — the most important text we send to 51Folds — was being truncated mid-sentence.** `max_tokens = 512` left no room for a 300-word narrative plus the rest of the template.
3. **The hypothesis template did not state what the context was *for*.** The LLM was writing "context" as commentary for a human reader instead of briefing material for a Bayesian driver extractor.
4. **The hypothesis parser only split outcomes on `|`.** When the LLM returned a bullet list instead (which it sometimes did), the entire list collapsed into a single "outcome" and the UI rendered `•  - Stays below $4,800` where `•` was from our label and `-` was the unstripped LLM bullet.
5. **The 51Folds integration had no persistence.** A model creation request was session-only state on `DashboardApp::folds_task`. If the user closed the app during the minutes-long provisioning window, the model ID and status were forgotten — the model itself still lived on the 51Folds server, but the app had no way to find it again.
6. **The AI panel duplicated information.** The markdown response rendered `**Hypothesis**:`, `**Hypothesis Outcomes**:`, `**Hypothesis Context**:` sections, and the 51Folds editor below showed the same three fields as editable textboxes. The user saw every hypothesis twice.
7. **Clicking a history row restored the raw response but not the parsed hypothesis.** The 51Folds editor stayed empty and showed "No hypothesis in this analysis. Re-analyze to generate one."
8. **Inference list entries for different analyses run on the same regime were visually identical.** Two analyses — one with Gold selected, one with Silver — both showed `[Analysis] VIX 23.9 **Regime**: Supply Shock` with only the timestamp to distinguish them.

These are not independent bugs. The editor consolidation (#6) depends on the parser being robust (#4). The "load history restores state" fix (#7) depends on the hypothesis fields being persisted (which drives the list-label work in #8). The analysis-quality fixes (#1-3) all feed the same user-facing surface — the 51Folds additionalContext field — so they had to ship together to be evaluable.

## Decision

### 1. Ground-truth anchoring for LLM analysis

- The user message now carries **absolute close prices + dates** for every selected and unselected instrument, via a new `InstrumentSnapshot { instrument, latest_close, latest_date, pct_change_30d }` struct. Format: `- Gold: $4624 as of 2026-04-06 (-10.6% over 30d)`.
- The system prompt has a new **GROUND TRUTH RULE** section: "If your training prior says gold is around $2,000 but the user message says $4,624, the user message wins. The data may be more recent than your training cutoff." Web search is encouraged for dates/headlines/central-bank signals but subordinate to the user-message prices for any numeric claim.
- `max_tokens` bumped from 512 → 2048 so Context can actually complete its 300-word narrative.
- `MAX_RESPONSE_BYTES` bumped from 100KB → 500KB to accommodate web-search scaffolding in the Anthropic `web_search_20250305` and OpenAI `web_search_preview` response envelopes.

### 2. OpenAI Responses API with native web search

- `call_openai` switched from `/v1/chat/completions` to `/v1/responses`. The Responses API is the only OpenAI endpoint that exposes `tools: [{"type": "web_search_preview"}]`. The Chat Completions endpoint we previously used simply has no web search mechanism.
- Request body uses `instructions` (system) + `input` (user) + `tools` + `max_output_tokens`. Response parser prefers the convenience `output_text` field, falls back to walking `output[]` for `output_text` content blocks (which is where the final message lives when the model made tool calls first).

### 3. Hypothesis Context prompt rewrite

- The Hypothesis Context section of the system prompt is rewritten to state its purpose explicitly: **this text is sent verbatim to the 51Folds Bayesian forecasting API as the `additionalContext` field, which the API parses to derive the 15 causal drivers for an Insights-tier model.** It is briefing material for a Bayesian model builder, not prose for a human reader.
- Hard cap: **300 words max**, with an explicit "end on a complete sentence inside the 300-word limit" instruction to prevent truncation artefacts.
- Required content is structured for driver extraction: (1) current macro setup with concrete numbers and dated events, (2) causal mechanism naming transmission factors, (3) at least three confirming and three contradicting signals, (4) time-horizon justification tied to calendar/data/deadline anchors.

### 4. Robust outcomes parser

- New shared helper `ai::split_outcomes_block(body)` handles both pipe-separated (`a | b | c`) and newline-bulleted (`- a\n- b\n- c`) formats, strips `-`/`*`/`•`/`1.`/`1)` bullet prefixes via `strip_bullet`, and caps defensively at 8 outcomes.
- Both `parse_hypothesis` (initial analysis parser) and `parse_outcomes_reroll` (the "Different outcomes" button) call this shared helper. Previously only the reroll parser had the robust logic; the initial parser fell over on bullet-list responses, which was the root cause of the collapsed-single-outcome rendering.

### 5. Persistent 51Folds model tracking

- New `folds_models` table: `(id, model_id UNIQUE, status, created_at, completed_at, last_polled_at, question)`.
- Statuses: `pending`, `success`, `fail`, `undisclosed_failure`. Plus a derived `is_suspect` flag (pending AND elapsed ≥ 1h) computed at read time, never persisted.
- **Lifecycle**: on `Created(model_id)` from the 51Folds POST, insert a `pending` row BEFORE polling begins. The polling thread owns its own SQLite `Connection` (via a standalone helper) and updates the row directly — SQLite WAL mode (enabled in `Storage::init`) handles concurrent writes. On terminal state (`Successed`/`Failed`), the row is updated with `completed_at`.
- **2-hour ceiling**: if elapsed > 2h the polling thread writes `undisclosed_failure` and stops. The user explicitly asked for this — anything past two hours is treated as a silent server-side failure rather than perpetual polling.
- **Polling interval**: 60 seconds. Five seconds was appropriate for a mock but burns API calls without giving an Insights-tier provision time to make progress.
- **Resume on restart**: `App::new` calls `resume_pending_folds_models()` after storage open. It loads every `pending` row, marks too-old ones as `undisclosed_failure`, and spawns a background polling thread (with `tx = None` — no live UI channel) for each live one. A status-line summary reports `Resumed N pending 51Folds model(s) — M suspect (>1h pending); marked K as undisclosed_failure (>2h)`.

### 6. Hypothesis persistence on inferences

- `ai_inferences` table gains four columns via additive migration: `hypothesis_question`, `hypothesis_outcomes` (JSON array), `hypothesis_context`, `overlay_instruments` (JSON array of storage keys). The migration uses `PRAGMA table_info` to detect existing columns and `ALTER TABLE ADD COLUMN` only what is missing, so upgraded databases preserve their rows.
- `poll_ai` is reordered to **parse the hypothesis BEFORE saving**, so the structured fields go into the same INSERT as the raw response. The overlay snapshot is captured from `settings.overlay_instruments` at save time.
- `SavedInference` struct carries the four new fields as `Option`s. A `row_to_saved_inference` helper decodes the JSON best-effort (NULL / malformed JSON → `None`, never fails the whole load).

### 7. AI panel editor consolidation

The previous panel rendered (a) the full markdown response including `**Hypothesis**:` / `**Hypothesis Outcomes**:` / `**Hypothesis Context**:`, then (b) the 51Folds editor with the same three fields as editable textboxes. The user saw every hypothesis twice. The panel is now restructured:

- **Markdown render is truncated at `**Hypothesis**:`** via a new `split_off_hypothesis(response)` helper. The markdown view shows only the regime-classification portion (Regime, Confidence, Signal Reading, Key Confirmation, Key Divergence, Watch For).
- **The 51Folds editor is the authoritative display for the hypothesis** — Hypothesis Statement (editable textbox), Outcomes (read-only framed labels), Context (read-only framed label).
- **Outcomes are rendered as read-only labels in SURFACE-framed rows**, one per line, white bold font. No bullet prefix in the text — the frame is the visual marker. No × delete buttons, no `+ Add outcome`. To change outcomes, the user clicks `↻ Different outcomes` (see below).
- **Context is rendered as a read-only wrapping label in the same SURFACE-framed style**. Both blocks are indented from the panel edge via `ui.horizontal` + `add_space(8.0)` so they don't hug the border.
- **Action buttons live at the bottom of the section**: `[↻ Different outcomes]  [→ Create 51Folds Model]`, left-aligned with an 8px gap. Reroll and primary action sit in reading order.

### 8. "Different outcomes" reroll

- New `ai::assemble_outcomes_reroll_prompt(question, context, previous_outcomes)` returns a tight system+user pair that asks the LLM for **outcomes only** — no regime classification, no preamble, no explanation. The previous outcomes are included explicitly so the model can deliberately diverge.
- On success, only `draft_hypothesis.outcomes` is replaced. The question and context stay untouched, and **nothing is saved to the inference history** (separate `LlmTask` from the main `ai_task`).
- The button renders a spinner with "Getting new outcomes…" during flight. No UI jargon.

### 9. 51Folds model tier fixed to Insights

- The Overview / Insights selector in the UI is gone. `FoldsModelType` enum removed. Every create request sends `type: "Insights"` (via the constant `FOLDS_MODEL_TYPE`). The Insights tier provisions a 15-driver causal graph on 51Folds, which is the right granularity for regime-shift hypotheses.
- The 51Folds `additionalContext` field drives driver extraction, which is why the Hypothesis Context prompt rewrite (#3) ships alongside this decision.

### 10. Historical inference loading

- New helper method `load_historical_inference(&mut self, inf: SavedInference)` centralises the restore logic for both click handlers (sidebar History and Report window inference list). It:
  - Reconstructs a `ParsedHypothesis` from the persisted columns when present, otherwise re-parses `inf.response` markdown via `ai::parse_hypothesis` (fallback for pre-migration rows).
  - Sets both `parsed_hypothesis` (so the Reset button would work if we re-added it) and `draft_hypothesis` (so the editor populates).
  - Resets `folds_task` and `outcomes_task` so previous in-flight background work can't bleed into the loaded analysis.
  - Opens the AI panel.

### 11. Inference list labelling

- Three label helpers in `app.rs`:
  - `inference_label_short(inf)` → `04-07 05:44  [Analysis] VIX 23.9  Gold/Silver/Bitcoin`. Used in the narrow sidebar (50-char manual truncation with `…`).
  - `inference_label(inf)` → the short label plus `· {hypothesis snippet, 60 chars}`. Used in the report window where there is horizontal room.
  - `inference_label_full(inf)` → the short label plus a blank line and the **complete** hypothesis question. Used as the sidebar tooltip's hover text.
- `format_overlay_label` maps storage keys back to display names, caps at 3 with `+N` suffix, joins with `/`. This is the key element that makes Gold-vs-Silver analyses visually distinguishable in the list.
- Sidebar rendering uses **manual truncation** (`truncate_with_ellipsis`), not egui's `Label::truncate()`. egui's built-in truncate attaches its own hover tooltip with the full text, which collided with our explicit `on_hover_text` and produced two tooltips on hover.

## Consequences

### Positive

- **Price hallucination eliminated.** The user message now carries authoritative ground truth so the LLM no longer anchors on training-data priors. The gold-$2,100 incident that drove this work cannot recur for any instrument whose prices the app has loaded.
- **Context never truncates mid-sentence.** 300-word hard cap + 2048 token budget + explicit "end on a complete sentence" directive. The text that goes to 51Folds is now well-formed every time.
- **The 51Folds integration survives app restarts.** Every model creation is persisted immediately; the resume sweep on startup restores polling for anything still pending. Closing the app no longer loses work.
- **No more duplicated hypothesis.** The markdown view ends where the 51Folds editor begins. Users see the hypothesis once, in the shape they can act on.
- **Clicking history actually restores the analysis.** Both the AI panel and the 51Folds editor repopulate from the persisted hypothesis columns. Rerolling outcomes and creating 51Folds models from historical analyses now work.
- **Inference list entries are visually distinct.** Overlay instruments are part of the label, so Gold-only vs Silver-only analyses on the same regime no longer collide.
- **Outcomes parser is forgiving.** The shared `split_outcomes_block` accepts either format, and both the initial-analysis and reroll paths benefit. The doubled `•  -` rendering is impossible.

### Neutral / known limitations

- **51Folds driver content and justification are not surfaced yet.** The polling thread writes `success` / `fail` to the DB row but the app does not yet fetch `?IncludeDriverContext=true&IncludeDriverJustification=true` or render the 15 drivers. That is deferred to a future spec the user said they will write; the present work only satisfies the tracking and persistence requirement.
- **Resumed polling threads do not surface to the activity log.** They update the DB and log to stderr + the transient status line. The activity log is instrument-keyed via `LogEntry { instrument: Instrument, ... }` and would need an `Option<Instrument>` refactor before it could carry folds events. Deferred until the user asks for it.
- **`gpt-5.4` is set as the default OpenAI model** on the user's direction. This ADR's author cannot verify from Anthropic training data that a model by that name exists or supports the Responses API + `web_search_preview` tool. The UI allows overriding the model name at runtime, so this is a reversible default, not a hardcoded dependency.
- **"Suspect" (pending ≥ 1h) is only surfaced in the startup status-line summary**, not elsewhere in the UI. A user who does not catch the startup line will not see that any of their pending models are suspect until the 2-hour ceiling fires.
- **Pre-migration inference rows have `NULL` for the new hypothesis columns.** The loader falls back to re-parsing `inf.response` markdown for old rows, so they still work, but they don't benefit from the structured-column label improvements and their labels omit the overlay segment.

### Negative

- **OpenAI Responses API is a different code path from Anthropic.** The two providers now have meaningfully different request shapes (`instructions` + `input` + `tools` vs `system` + `messages` + `tools`), different parameter names (`max_output_tokens` vs `max_tokens`), and different response parsing (walking `output[]` for `output_text` blocks vs walking `content[]` for `text` blocks). Each gained native web search, but at the cost of two fully distinct HTTP integrations in `ai.rs`.
- **The 51Folds polling thread cannot be cancelled mid-flight.** `folds_task.reset()` drops the receiver in the main thread, but the background thread keeps running until it observes a terminal state or hits the 2-hour ceiling. In practice that is fine — closing the channel makes `tx.send(...)` a no-op on the next tick — but the thread is not strictly bounded by the UI lifecycle.

## References

- Analysis prompt + ground-truth rule: `src/ai.rs::assemble_system_prompt`, `src/ai.rs::assemble_user_message`, `src/ai.rs::InstrumentSnapshot`
- OpenAI Responses API integration: `src/ai.rs::call_openai`
- Outcomes parser: `src/ai.rs::split_outcomes_block`, `src/ai.rs::parse_hypothesis`, `src/ai.rs::parse_outcomes_reroll`
- 51Folds persistence: `src/storage.rs::folds_models` schema, `src/storage.rs::load_pending_folds_models`, `src/storage.rs::update_folds_model_status_standalone`
- 51Folds polling: `src/folds.rs::poll_until_terminal`
- Resume sweep: `src/app.rs::resume_pending_folds_models`
- Editor consolidation: `src/app.rs::render_folds_section`, `src/app.rs::split_off_hypothesis`
- Historical load: `src/app.rs::load_historical_inference`
- Label helpers: `src/app.rs::inference_label`, `src/app.rs::inference_label_short`, `src/app.rs::inference_label_full`, `src/app.rs::format_overlay_label`, `src/app.rs::truncate_with_ellipsis`
- Inference schema migration: `src/storage.rs::Storage::init`, `src/storage.rs::Storage::column_set`
- Related: ADR 0008 (51Folds integration spec), ADR 0009 (hypothesis quality + OpenAI tool compatibility), ADR 0011 (single-provider daily cache)
