# ADR-0007: Roll Back Tabbed Workspace; Focus Integration on 51Folds

**Status:** Accepted  
**Date:** 2026-04-04  
**Branch:** `hedgehog-poc`

---

## Context

FIP-0001 introduced a three-tab workspace (Regime Shift, Dexter, 51Folds) and FIP-0002 specified the Dexter autonomous research agent. Both were implemented in a single commit on the `hedgehog-poc` branch alongside the AI analysis features from ADR-0005 and ADR-0006.

After review, the scope was judged too broad. The Dexter agent duplicates research capabilities that 51Folds already provides through its neuro-symbolic modelling pipeline. Maintaining two separate AI-powered research paths (Dexter's agentic tool-use loop and 51Folds' structured Bayesian approach) adds complexity without clear differentiation for the end user. The tabbed UI also fragments what works well as a single focused dashboard.

---

## Decision

Roll back the tabbed workspace and Dexter agent. Retain the AI analysis integration (ADR-0005, ADR-0006) and scale the app to integrate directly with 51Folds as the sole advanced analysis pathway.

### Removed

- **Tab system** -- `WorkspaceTab` enum, tab bar UI, tab dispatch routing
- **Dexter agent** -- entire `src/dexter/` module (agent, tools, search, finance, scratchpad, prompts, formatting, compaction), all associated state in `DashboardApp`, event polling, and the Dexter tab UI (~2,000 lines)
- **51Folds placeholder tab** -- stub UI; the real integration will be built differently
- **`call_llm_public`** -- public wrapper in `src/ai.rs` that only existed for Dexter's use

### Retained

- **AI analysis panel** (OpenAI / Anthropic) with RAG knowledge base (ADR-0005)
- **Inference persistence and summary reports** (ADR-0006)
- **Single-page dashboard layout** -- VIX charts, commodity correlation, spike detection, sidebar controls, activity log
- **All data providers** -- FRED, Alpha Vantage, Tiingo

### Additional fix

- **AI panel resize overflow** -- removed the hardcoded `max_height(400.0)` on the bottom AI panel. The panel now measures its content height each frame and caps `max_height` to the actual content size plus header, preventing a black gap when the panel is dragged beyond its text content.

---

## Consequences

- The app returns to a single-view dashboard with AI analysis, matching the state after ADR-0006 but on the `hedgehog-poc` branch.
- FIP-0001 and FIP-0002 are now superseded. They remain in `FIPs/` as historical record.
- Future 51Folds integration should be designed as a direct feature of the dashboard (e.g. sidebar section, dedicated panel) rather than a separate tab.
- The `src/dexter/` module and its ~2,000 lines of agent code are fully removed, not just disabled. If agentic research is revisited, it should be re-evaluated against whatever 51Folds provides at that point.
