# ADR-0009: Strengthen Hypothesis Quality and Fix OpenAI Tool Compatibility

**Status:** Accepted  
**Date:** 2026-04-06  
**Branch:** `hedgehog-poc`

---

## Context

ADR-0008 established the 51Folds integration architecture, including LLM-generated hypotheses. Early testing revealed two categories of issues:

**Technical**: OpenAI API deprecated `web_search_preview` as a tool type in favour of a stricter schema that accepts only `"function"` and `"custom"`. The request was failing with HTTP 400 Bad Request.

**Quality**: The initial hypothesis template was too terse to support proper Bayesian model creation in 51Folds. The LLM was producing hypotheses grounded in historical price anchors (e.g. "$2,200 gold") rather than current market data, and the context narrative was 50-100 words when 51Folds models require ~300 words of substantive background explaining the mechanism, confirming signals, and timeframe justification. The hypothesis itself was phrased as a question rather than a testable claim with explicit mechanism.

Compare:
- **Current output**: "Will gold recover to above $2,200 within the next 30 days?" + 50-word context
- **Required format** (per 51Folds example): "Gold will remain above $4,000 through Q2 as geopolitical hedging demand offsets recession fears." + 300-word narrative grounding this claim in current CB policy, supply data, and market signals

The root cause: the system prompt did not adequately instruct the LLM to (1) use web search for current prices and events, (2) formulate a substantive mechanistic claim rather than a backward-looking question, or (3) build a rich contextual narrative.

---

## Decision

### 1. Fix OpenAI API compatibility

**Problem**: `call_openai()` was including `"tools": [{"type": "web_search_preview"}]` in the request body, causing HTTP 400.

**Solution**: Remove the tools array entirely. Without tools, OpenAI returns plain text in the standard response shape, and the response parser simplifies to a single string lookup. Anthropic retains `web_search_20250305` (which is the correct current format and remains unchanged).

**Code change**: 
- Removed `"tools": [{"type": "web_search_preview"}]` from `call_openai()` request body
- Simplified response parsing from multi-block traversal to direct string access: `response["choices"][0]["message"]["content"].as_str()`

### 2. Strengthen hypothesis structure and system prompt

**Problem**: The LLM was not given clear instructions to produce hypothesis statements suitable for forecasting models.

**Solution**: Restructure the system prompt and template to explicitly call for:

- **Hypothesis statement** (not question): "A substantive, time-bounded claim (7-90 days) explaining a specific price/behaviour change and the mechanism driving it."
  - Example: "Crude oil will remain above $78 through Q2 as OPEC supply discipline holds despite recession headwinds."
  - NOT: "Will crude oil remain above $78?"

- **Outcomes** as mutually exclusive causal paths: "2-4 outcomes representing distinct causal paths" rather than just alternative price points.
  - Example: "Holds above $78 — OPEC discipline | Falls below $70 — demand destruction | Spikes to $95+ — supply shock"

- **Context as a rich narrative (~300 words)**: Explicitly instructed to use web search extensively to gather:
  1. Historical/structural context explaining why this hypothesis is relevant *now*
  2. The specific mechanism of change expected
  3. Market signals that would confirm vs. contradict it
  4. Recent price data, central bank signals, supply disruptions, geopolitical factors
  5. Justification for the 7-90 day timeframe

**Updated system prompt preamble** now states:
```
For the Hypothesis Context section, use web search extensively to ground your 
analysis in current data. Fetch: today's crude oil, gold, copper, and equity index 
prices; recent Fed/ECB/BoE signals; ongoing supply disruptions or geopolitical 
events; recent changes in volatility term structure or sentiment indices. Your 
context must read as a narrative that a sophisticated investor would rely on — 
specific numbers, named events, dated signals.
```

### 3. Expand UI to support richer hypothesis editing

**Outcome alignment**: Change `desired_width()` from `ui.available_width() - 28.0` to `f32::INFINITY`, allowing each outcome text field to fill available space. The delete button (×) now consistently aligns to the right edge, removing ragged left alignment.

**Context field expansion**: 
- Increase from `desired_rows(2)` to `desired_rows(8)` to accommodate ~300-word narratives
- Remove the 150-character limit that was constraining responses
- Add detailed guidance text below the "Context" label:
  ```
  "Historical background, mechanism of change, confirming/contradicting signals, 
   and why this 7-90 day timeframe matters. Use current market data and events."
  ```

**UI label changes**:
- "Question" → "Hypothesis Statement" with guidance: "Substantive claim (not a question), time-bounded 7-90 days, explaining mechanism."
- "Outcomes" guidance added: "2-4 mutually exclusive outcomes (≤60 chars each), representing different causal paths."
- "Context" → "Context (Narrative ~300 words)" with the guidance text above

---

## Files Changed

| File | Change |
|------|--------|
| `src/ai.rs` | Updated `assemble_system_prompt()` with detailed web-search instructions for hypothesis context; reworded template to require substantive claim (not question) and ~300-word mechanistic narrative |
| `src/ai.rs` | Updated `call_openai()` to remove invalid `tools` array |
| `src/ai.rs` | Simplified OpenAI response parsing from multi-block search to direct string access |
| `src/app.rs` | Changed outcome TextEdit `desired_width()` to `f32::INFINITY` for right-aligned delete buttons |
| `src/app.rs` | Expanded context field from 2 to 8 rows; removed character limit |
| `src/app.rs` | Added guidance text below "Hypothesis Statement", "Outcomes", and "Context" labels |

---

## Consequences

- **Immediate**: OpenAI requests now succeed without 400 errors; the API compatibility issue is resolved for both providers.
- **Quality**: The LLM is now explicitly instructed to produce hypothesis statements grounded in current market data, with mechanistic claims and rich contextual narratives. Outputs should align with 51Folds best practices.
- **UX**: The UI better guides the user toward the hypothesis structure 51Folds models require. The expanded context field signals that more substantive writing is expected.
- **Data**: The system prompt instructs web search, but user results will still depend on API key configuration and live market data availability (e.g., commodity prices from Tiingo). The $2,000 gold anchor issue in early tests should resolve as the LLM fetches current prices (~$4,000+) via search.
- **Future work**: Monitor hypothesis output quality in live use; consider adding client-side character count badges to the Context field if user feedback indicates the guidance is unclear.

---

## Notes

This ADR resolves the immediate technical blocker (OpenAI tool incompatibility) and raises the bar for hypothesis quality to match 51Folds requirements, as evidenced by the 51Folds example hypothesis reviewed during planning.
