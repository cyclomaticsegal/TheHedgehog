# ADR 0017: Commodity-Bias Evaluation and Cross-Model LLM Judge

**Date:** 2026-04-14
**Status:** Accepted
**Extends:** ADR 0005 (RAG-powered AI analysis), ADR 0012 (analysis quality hardening)

## Context

The AI analysis pipeline assembles a system prompt and user message from market data, then sends them to an LLM for regime classification and hypothesis generation. A commodity-bias bug was discovered where the prompt could steer the LLM toward the wrong instruments — Soybeans attributed to the wrong data source, specific commodity names hardcoded in template examples, and a weak boundary between primary and tertiary instruments.

The fix was landed in `ai.rs`, but there was no way to verify it held or catch future regressions. Issue #6 asked for validation of the commodity-bias prompt fix.

## Decisions

### 1. Deterministic prompt evaluation (`src/eval.rs`)

A new module validates the generated prompts against nine structural rules before they reach any LLM. Each rule is a pure string assertion — no LLM calls, fully deterministic:

- **GroundTruthSources** — FRED attributed only to VIX, Alpha Vantage for all commodities including Soybeans
- **PrimarySubjectMatch** — "Instruments in view" lists exactly the user's selection
- **SelectedInLatestCloses** — selected instruments appear in the price block
- **UnselectedTertiaryFraming** — unselected instruments have the TERTIARY warning
- **NoSelectedInTertiary** / **NoUnselectedInPrimary** — no cross-contamination
- **KnowledgeRelevance** — knowledge chunks match the selection
- **NoHardcodedSubject** — template doesn't name a specific commodity as the hypothesis subject
- **SourceAttributionConsistent** — no stale FRED references for commodities

Four test scenarios (single instrument, metals basket, soybeans-only, energy+agriculture mix) exercise all nine rules via `cargo test -- eval`. The types are always-compiled (`pub`) for potential runtime use; the test harness is `#[cfg(test)]`.

### 2. Deterministic response validation (runtime)

Three checks run instantly after every AI analysis completes:

- **HypothesisNamesSelected** — the hypothesis mentions at least one selected instrument
- **NoUnselectedAsSubject** — no unselected instrument is the grammatical subject
- **PriceAnchoring** — dollar amounts are within 25% of the latest closes (catches training-data price priors)

Results display in a "Bias Check" bar in the AI panel: `3/3 structural`.

### 3. LLM-as-judge (runtime, async)

A second LLM call validates the analysis semantically against five rules: subject match, price anchoring, mechanism relevance, tertiary boundary, outcome alignment. The judge prompt produces structured `PASS|FAIL` output that's machine-parsed.

**Cross-model validation**: when both Anthropic and OpenAI API keys are present, the judge automatically uses the *other* provider — the one that did NOT produce the analysis. This gives independent cross-model validation with no user configuration. If only one key is present, the same provider judges its own work (still useful — the task frame is fundamentally different).

The judge fires asynchronously after each analysis. Results appear as `5/5 semantic (GPT)` in the Bias Check bar, with a spinner while in flight.

### 4. UI integration

The Bias Check bar sits between the analysis response and the 51Folds section. Tooltips on "structural" and "semantic" explain each check. Failures are shown inline with word-boundary-truncated reasons and full-text tooltips.

## Consequences

- Every AI analysis is now validated at three levels: prompt structure (test-time), response structure (runtime instant), response semantics (runtime async)
- Regressions in prompt assembly are caught by `cargo test` before they ship
- The cross-model judge adds one LLM call per analysis when both keys are present — acceptable cost for independent validation
- The eval module's public types can be reused for runtime pre-flight checks if desired
