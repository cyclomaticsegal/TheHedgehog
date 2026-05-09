# ADR-0006: Inference Persistence and Summary Reports

**Status:** Accepted  
**Date:** 2026-04-03  
**Branch:** `perf-imp`

---

## Context

ADR-0005 added LLM-powered analysis of live market data. However, each analysis response is ephemeral — it disappears when the panel closes or a new analysis runs. The user wants to:

1. **Persist every inference** to SQLite with timestamp, enabling a historical log of AI analyses
2. **Generate summary reports** over a date range, aggregating saved inferences into a retrospective view that compares the observed period to historical precedents

These are implemented as two standalone phases: Phase 1 (persistence + history sidebar) works independently; Phase 2 (reports) builds on the persisted data.

---

## Decision

### Phase 1: Inference Persistence

**Table schema:**

```sql
CREATE TABLE IF NOT EXISTS ai_inferences (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at    TEXT NOT NULL,       -- RFC 3339 UTC
    provider      TEXT NOT NULL,       -- "anthropic" or "openai"
    model         TEXT NOT NULL,
    system_prompt TEXT NOT NULL,
    user_message  TEXT NOT NULL,
    response      TEXT NOT NULL,
    vix_close     REAL,               -- denormalized for sidebar display
    vix_level     TEXT                 -- denormalized for filtering
);
```

**Why store system_prompt and user_message?** They encode the exact context the LLM saw — which instruments were selected, what the VIX was, what knowledge chunks were retrieved. Without them, a saved inference loses its provenance. Storage cost is trivial (a few KB per row).

**Why denormalize vix_close and vix_level?** These allow the sidebar history to show a compact display with VIX level indicators (colored dots) and enable Phase 2's date-range queries to optionally filter by VIX regime — all without parsing the markdown response or user_message.

**Persistence flow:** `AiEvent::Response` was changed from carrying a `String` to carrying an `AiInferenceResult` struct that echoes back the provider, model, system_prompt, and user_message alongside the response text. This avoids storing pending state on `DashboardApp` — when `poll_ai()` receives the result, it has everything needed to persist in a single operation.

**History sidebar:** The "AI Analysis" collapsing section shows the 20 most recent inferences, each with a timestamp, VIX level color dot, and truncated first line. Clicking an entry loads the full response into the AI panel, providing instant access to past analyses.

### Phase 2: Summary Reports

**Report generation flow:**
1. User opens the Report window (top bar button)
2. Selects a date range via text fields or quick-select buttons (7d / 30d / 90d / All)
3. Clicks "Load Inferences" — fetches matching rows from `ai_inferences`
4. Clicks "Generate Report" — assembles all loaded inferences into a single LLM prompt asking for a retrospective synthesis

**Report prompt design:** The system prompt instructs a "senior macro-financial analyst" to produce a structured report with sections: Executive Summary, Period Overview, Key Themes, Historical Context, and Outlook. Each saved inference is included with its timestamp and VIX reading. For sets larger than 20 inferences, responses are truncated to 500 characters to prevent context window overflow.

**Why rely on LLM training data instead of web search?** Adding a search API (Brave, Serper, etc.) would introduce a new dependency, API key, error handling, and prompt engineering complexity. The LLM's training data includes extensive financial history — sufficient for comparing observed patterns to historical precedents. Web search could be added as a future enhancement if needed.

**Report window:** Uses `egui::Window` (matching the Help window pattern) rather than a bottom panel. Reports are longer-form content that benefits from a larger, movable, resizable window that keeps the dashboard visible underneath.

**Reports saved to DB:** Generated reports are themselves saved to `ai_inferences` with a `provider` field of `"report:{provider}"`, making them part of the historical record and distinguishable from single analyses.

---

## Files

| File | Phase 1 | Phase 2 |
|------|---------|---------|
| `src/models.rs` | `SavedInference`, `AiInferenceResult`, updated `AiEvent` | — |
| `src/storage.rs` | `ai_inferences` table, `save_inference()`, `load_recent_inferences()`, `load_inferences_in_range()` | — |
| `src/ai.rs` | Updated `run_analysis()` to return `AiInferenceResult` | `assemble_report_prompt()` |
| `src/app.rs` | Updated `poll_ai()` with persistence, `inference_history` field, sidebar history UI, `reload_inference_history()` | Report fields, `start_report_generation()`, `poll_report()`, `load_report_inferences()`, "Report" button, report window UI |

No new files. No new crate dependencies.

---

## Consequences

- Every AI analysis is automatically persisted — no user action required beyond clicking "Analyze"
- History is immediately available in the sidebar and survives app restarts
- Reports provide a higher-order synthesis across time, useful for tracking regime evolution
- The `ai_inferences` table grows unboundedly; a future cleanup/archival mechanism may be needed for very long-running usage
- Reports themselves are persisted, creating a layered historical record (raw analyses + periodic summaries)
