# ADR 0022: Primary / Secondary / Tertiary Prompt Framework and Bias-Feedback Loop

**Date:** 2026-04-16
**Status:** Accepted
**Extends:** ADR 0005 (RAG AI analysis panel), ADR 0009 (hypothesis quality), ADR 0012 (analysis quality + folds persistence), ADR 0017 (commodity-bias eval and cross-model judge), ADR 0021 (instrument set reduction)

## Context

The AI analysis prompt framework (introduced in ADR 0005, hardened through ADRs 0009/0012/0017) assumed a binary split: selected instruments were the PRIMARY subject of the hypothesis; unselected were TERTIARY background. The structural bias checks (`EvalRule` variants) and the cross-model LLM judge (ADR 0017) both tested assertions against that binary.

That model worked when selections were usually 1–2 out of 10+ instruments. ADR 0021 collapsed the instrument set to 6 (5 overlay-eligible), with the default Monetary group now selecting Gold, Silver, and Bitcoin simultaneously — 3 primaries out of 5 eligible, only 2 tertiaries remaining. The selected/tertiary ratio inverted, and the bias framework lost resolving power:

- `PrimarySubjectMatch` demanded the hypothesis be "about the selected instruments" (plural), but an LLM asked to produce one hypothesis covering Gold + Silver + Bitcoin tends either to pick one implicitly or to produce vague multi-subject prose.
- `UnselectedTertiaryFraming` with only two tertiaries (Crude Oil, Natural Gas) becomes almost trivial.
- The LLM judge's SUBJECT_MATCH rubric was ambiguous: is the hypothesis required to name *all* selected or *any* selected?

The system prompt also had a latent naming collision: it referred to three "tiers" where `SECONDARY` meant "the knowledge base" — not "other selected instruments." The term most readers would expect for the corroborative-instrument slot was wasted.

A 6-month test run against the default Gold+Silver+Bitcoin selection was almost guaranteed to trip MECHANISM_RELEVANCE: the LLM produces generic macro narrative rather than Bitcoin-specific market-structure reasoning because the prompt doesn't force a single subject.

## Decisions

### 1. Three-tier framework with real separation

The prompt framework is rebuilt around three disjoint instrument tiers:

- **PRIMARY** — exactly one named subject from the selection. Every outcome band, strike level, and price reference must anchor to this instrument.
- **SECONDARY** — the other selected instruments. Named in the Hypothesis Context as corroborative or challenging signal, never as subject of the hypothesis statement or any outcome band.
- **TERTIARY** — unselected instruments. Background only; mentioned only if their current behaviour materially supports or contradicts the primary thesis.

The knowledge base is reclassified out of the "SECONDARY" slot into a separate "KNOWLEDGE LIBRARY" label — it's a mechanism reference, not a peer tier to primary/tertiary.

### 2. Intent captured at the moment of ambiguity

Selecting the primary from multiple candidates is a decision the user should make, not the LLM. The modal is opened only when the question has a non-obvious answer:

- Single selected instrument → primary is trivially that one. Analyze runs immediately. No modal.
- Multiple selected → clicking Analyze opens a small centered dialog titled "Pick the primary instrument for this analysis" with a radio list of the selected instruments (first one pre-selected) and Cancel / Analyze buttons. Analyze re-enters `start_ai_analysis` with the choice populated; Cancel clears state and aborts.

No sidebar primary picker, no always-on primary radio. The question is asked only when it's actually ambiguous, at the moment of intent.

### 3. Prompt-template branching on SECONDARY presence

`assemble_user_message` (`src/ai.rs`) signature shifted from `(vix, overlay_instruments, snapshots, unselected_snapshots, spikes)` to `(vix, primary, secondary, primary_snapshot, secondary_snapshots, tertiary_snapshots, spikes)`. Section structure:

- `**Primary instrument**: <name>` — single line naming the subject.
- `**Latest close (authoritative — use this in your hypothesis)**` — primary only. The strike-level anchor.
- `**Secondary instruments**` — conditional. Emitted only when secondary is non-empty, with the corroborative-framing header.
- `**Other available instruments (not in the user's selection)**` — tertiary block, unchanged framing.

For single-select runs, the SECONDARY block is absent entirely. No cross-sectional "how do these interact" scaffolding is requested of the model because there's nothing to interact.

### 4. Bias checks rewritten for three tiers

`EvalRule` variants renamed and reshaped (`src/eval.rs`):

- `PrimarySubjectMatch` → `PrimaryNamed`. Checks that the user message contains exactly one `**Primary instrument**: <name>` line and that the named instrument matches the scenario's expected primary.
- `SelectedInLatestCloses` → `PrimaryInLatestClose`. Checks that the primary appears in the "Latest close" block and that no secondary has leaked into it.
- New `SecondaryFraming`. When secondary is non-empty: "Secondary instruments" block is present, lists exactly those instruments, and carries the corroborative-framing header. Empty secondary: block must be absent.
- `NoSelectedInTertiary` expanded — neither primary nor secondary may appear in the tertiary block.
- `UnselectedTertiaryFraming`, `NoUnselectedInPrimary`, `KnowledgeRelevance`, `NoHardcodedSubject`, `SourceAttributionConsistent` kept.

The test helper `build_scenario` signature shifts from `(name, selected, vix)` to `(name, primary, secondary, vix)` to match.

### 5. LLM judge rubric rewritten for three tiers

`assemble_bias_judge_prompt` (`src/eval.rs`) now takes `(primary, secondary, snapshots, response)` and the five rules are:

- **SUBJECT_MATCH**: hypothesis subject is exactly the PRIMARY. Not a secondary, not a tertiary.
- **PRICE_ANCHORING**: strike levels and outcome-band prices come from the PRIMARY's latest close. Secondary prices may appear for context but never as the strike.
- **MECHANISM_RELEVANCE**: mechanism is specific to the PRIMARY. Secondary instruments appear as corroborative evidence but must not drive the mechanism narrative.
- **SECONDARY_FRAMING** (replaces TERTIARY_BOUNDARY as the sole boundary rule): secondaries are named as corroborative or challenging, never as subject. Tertiaries are background only.
- **OUTCOME_ALIGNMENT**: each outcome band references the PRIMARY's price level.

### 6. Bias-failure feedback on Re-analyze

A corrective feedback loop closes the gap between "the judge flagged a failure" and "the next run avoids the same mistake":

- `DashboardApp` stashes `last_bias_failures: Vec<ResponseValidation>` (merging deterministic + LLM-judge failures, filtered to `pass == false`) after every judge run. The `JUDGE_ERROR` sentinel is excluded because it reflects a judge-call failure, not analysis content.
- On the next `start_ai_analysis`, if the stash is non-empty AND the current primary matches `last_analysis_primary` (the primary the failures were judged against), a block is injected into the system prompt right after the three-tier framing and before the response template:

  ```
  PRIOR ATTEMPT — BIAS FAILURES TO ADDRESS:
  - MECHANISM_RELEVANCE: [judge's reason text]
  - [etc.]
  Ground the mechanism in <primary>-specific transmission channels
  rather than generic macro commentary. Anchor every numeric claim
  to the PRIMARY's latest close. Keep SECONDARY instruments as
  corroborative mentions only.
  ```

- The stash clears on dispatch. The next judge run repopulates it (whether with fewer failures, the same ones, or different ones).
- Skipped silently when the user has changed the primary between runs — stale failure text about the previous subject would misalign.
- Only activates on Re-analyze-style full re-runs. Different Outcomes doesn't benefit because it doesn't rewrite the mechanism (see ADR 0005 for the reroll path's scope).

`assemble_system_prompt` gained a second parameter, `prior_failures_block: Option<&str>`, to carry the injected block. Callers that don't need feedback (tests, cold runs) pass `None`.

## Consequences

- The default sidebar load (Gold+Silver+Bitcoin all selected) now produces an analysis with a single named subject, a meaningful secondary block, and two tertiary mentions — the structural checks can actually discriminate, and the judge has unambiguous rules.
- Users picking a single instrument see no new UI friction. The modal appears only when ambiguity exists.
- The bias feedback loop is a meaningful quality improvement: failures the judge flagged are automatically fed back into the next run. Risk of a correction loop (fix one rule, break another) is low because the user sees the new judge results after each Re-analyze and stops when satisfied.
- The `last_bias_failures` stash is session-only — not persisted. Closing the app drops the feedback; the next session starts fresh. Acceptable: the stash is meaningful only for the *next* Re-analyze, not across sessions.
- The test suite gained two scenarios (`eval_default_trio` covering Gold primary + Silver+Bitcoin secondary + Crude+NatGas tertiary, and `secondary_framing_fails_when_block_missing` for the regression path). Total tests: 14.
- The judge rubric change is a semantic break — any downstream consumer of the `TERTIARY_BOUNDARY` rule name (e.g. if the judge response was persisted somewhere) would need to know to look for `SECONDARY_FRAMING` instead. Nothing currently persists judge results across sessions, so no migration needed.
- Because the bias-feedback block is appended to the system prompt, the prompt gets longer over successive re-analyses that keep failing. The stash clears on dispatch, so failures don't accumulate — each run injects only the most recent judge's complaints. Prompt length remains bounded.
