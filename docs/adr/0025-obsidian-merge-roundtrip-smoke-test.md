# ADR 0025: Obsidian Round-Trip Merge Smoke Test — Vault as Bayesian Co-Editor

**Date:** 2026-05-15
**Status:** Accepted (implementation pending)
**Extends:** ADR 0008 (51Folds integration), ADR 0013 (SDK integration, model explorer)
**Plan document:** `obsidian-merge-smoke-test-plan.md` (repo root, branch `feat/obsidian-merge-smoke-test`)

## Context

A 12 May 2026 catch-up between Brett and Simon produced a unifying frame for 51Folds: **a bounded Obsidian-style note-taking experience overlaid on a Bayesian network**, with a one-click **Merge** button that re-elicits the network from the user's accumulated vault. Brett's literal ask: *"Could you create an Obsidian plugin, just for argument's sake, as a sort of a smoke test … that bounds Obsidian to a note-taking experience overlaid on top of a 51Folds Bayesian network?"*

Hedgehog already ships an Obsidian export (`06d5dc5 Add Export to Obsidian: 51Folds model → standalone vault` on `main`) — driver and outcome notes, frontmatter, wiki-links, `Data/model.json` provenance. It does not yet read changes back. The 51Folds Rust SDK at `vendor/fiftyone-folds/` exposes a limited write surface: `patch_drivers()` and `update_drivers()` for driver *states* (already wired in `src/folds.rs`), plus an opaque `submit_evidence()` and a `revisions()` endpoint that Hedgehog ignores. Edits the user would naturally make in a note-taking vault — notes, renames, edges, descriptions — have no documented server path today.

We want a smoke test of the full round-trip before committing to a real Obsidian plugin contract. Hedgehog is the cheapest place to prove the loop: it owns the export, the SDK, and the build/re-eval lifecycle, and it's a single-user private project so iteration is cheap.

## Decisions

### 1. Build the smoke test inside Hedgehog, not as an Obsidian plugin

A plugin is a different *host* for the same merge logic. What needs validating is the data contract: snapshot schema, diff format, preview UX, selective-rewrite invariants. All plugin-agnostic. Hedgehog already wires the SDK and the export; a real plugin — if it happens — reuses the same `VaultDiff` types and re-implements only the host side. Building plugin-first means inventing a contract under pressure of a third-party host; building Hedgehog-first lets us settle the contract in a controlled environment.

### 2. v1 ships driver-state round-trip only; other edit classes are *detected but parked*

The only knob the SDK exposes today is driver *state*. v1 implements:

> Export → user edits vault → press **Merge from Vault** → diff against snapshot → preview dialog → confirm → `patch_drivers()` → server re-infers → selective write-back into the same vault.

Every other class of edit — `## Notes` additions, driver renames, description edits, wiki-link "edges" added/removed, outcome label edits — is **detected in the diff** and surfaced in the preview dialog under a clearly-labelled **Detected, not applied (pending SDK support)** section. Each maps to one of five open questions for the 51Folds team:

- **Q1.** What does `submit_evidence()` actually do? Unblocks notes-as-evidence.
- **Q2.** Is there a planned endpoint for driver metadata edits? Unblocks renames / descriptions / context-block edits.
- **Q3.** Is edge mutation possible after build, or is the DAG structurally immutable post-elicitation by design? Unblocks structural editing via wiki-links.
- **Q4.** What does re-elicitation with new context require server-side — per-driver text, top-level text, structured expert assertions?
- **Q5.** Are revisions linear or branching, and user-facing? Unblocks revision tagging / comparison UI.

When a question gets answered, the matching row of the preview dialog converts from "client-side only" to "sent to server." The diff types already model every edit class, so no architectural rework is needed.

### 3. Write-back is selective — user-authored content is preserved byte-identical

A naive re-export ("rewrite the whole vault") would destroy any notes the user accumulated between merges. That contradicts the thesis behind the whole feature: **the user's reasoning accumulates over time**. So the write-back rewrites only the *model-derived* layer of each note — frontmatter values, probability bars, the structured sections the export currently emits (`## Possible States`, `## Why This Matters`, `## How It Shifts`, `## What To Monitor`, `## Local Causal Map`, `## In The Model`), outcome posteriors, `Data/model.json`, and the new snapshot. User-authored zones — any `## Notes` section, any free-form additions, any user-added wiki-links in driver bodies — are preserved verbatim.

The selective-rewrite invariant is **load-bearing**. If it leaks, the round-trip becomes lossy, the user loses trust in the vault, and the larger thesis collapses. Implementation requires reliable section-boundary detection in `src/obsidian/driver.rs` and `src/obsidian/outcome.rs`.

### 4. The vault carries its own history — versioned snapshots, audit notes, inline `## History` tables, Overview dashboard

Without on-vault history, every merge erases the prior model state from the user's view. Same contradiction with accumulation. Each merge therefore writes four artifacts:

- **`Data/snapshots/v001.json, v002.json, …`** — versioned directory replacing the single-file `Data/snapshot.json`. Each file is the diff baseline that produced the *next* merge. Snapshot `schema_version` bumped to `2`.
- **`Merges/<YYYY-MM-DD-HHMM>.md`** — one markdown audit note per merge: state changes applied, parked edits detected, outcome posterior shifts. Navigable like any Obsidian note; back-linked from per-node `## History` rows.
- **`## History` sections inline in `Drivers/*.md` and `Outcomes/*.md`** — a small table appended per merge: `vN | timestamp | before → after | [[merge note]]`. The journey of each node is visible where the user is already reading.
- **`## History` section in `Overview.md`** — appended after the existing `## Sources` section. Table of every merge with timestamps, change counts, wiki-links into the per-merge notes and snapshots. Acts as the vault-side dashboard; no new top-level file in v1. A standalone `History.md` is a possible follow-up if this section outgrows Overview's natural fit.

The server already exposes `revisions()` (Q5). When wired, vault history and server history reconcile via shared revision IDs. Until then, the vault is source of truth and the server is opaque.

### 5. Sequencing — six slices, partial value at each step

1. Snapshot writer (`src/obsidian/snapshot.rs`).
2. History emitter + selective-rewrite plumbing (`src/obsidian/history.rs`, extensions to `driver.rs` / `outcome.rs` / `overview.rs`).
3. Vault diff reader (`src/obsidian/merge.rs`).
4. Background `MergeTask` + `MergeEvent` channel.
5. Preview dialog (`src/app/dialogs.rs::render_merge_preview_dialog`).
6. Toolbar button + apply path (`start_folds_merge()` on `DashboardApp`, wrapping the existing `patch_drivers()` flow).

Slices 1–3 yield a usable "vault audit" tool — show the user what they've changed in their vault — without the apply path. The full round-trip activates at slice 6.

See `obsidian-merge-smoke-test-plan.md` for file-by-file changes, the full verification matrix, and module-level breakdown.

## Consequences

- **The smoke test ships without a plugin contract.** The data contract gets validated client-side first; a future Obsidian plugin reuses `VaultDiff` and the snapshot / history schemas unchanged. Brett's "press merge" affordance becomes a real UI surface in Hedgehog rather than a hypothetical.
- **Server-side re-elicitation stays the moat.** Hedgehog is a thin client; the merge logic and re-elicitation live behind the 51Folds API. Any third-party host consumes the same contract.
- **The vault grows on every merge.** `Data/snapshots/` accumulates JSON files; `Merges/` accumulates audit notes; `## History` tables in every driver and outcome lengthen indefinitely. Power-user benefit (full audit trail, navigable dashboard); deferred risk (vault bloat after many merges). Pruning is out of scope for v1 — a later slice can introduce "last N rows + archive note" once we have feel for cadence.
- **The selective-rewrite invariant is the single biggest implementation risk.** Section-boundary detection has to stay reliable across the structured sections the export emits today, plus whatever future export changes add. Verification tests 6–8 check this end-to-end; the v1 cut explicitly asserts a byte-identical `## Notes` section after merge.
- **Five SDK gaps are now explicit and documented (Q1–Q5).** They are not blockers for v1, but each gates one class of edit from round-tripping. Detected-but-parked edits stay in the vault after merge (preserved by the selective rewrite) and re-surface in the next merge's preview — useful as a "what's not yet possible" affordance, but they will appear as recurring entries in the diff until the corresponding SDK path lands. Closing each Q is a follow-up slice with no architectural cost.
- **Schema bump `v1` → `v2` of the snapshot format.** Vaults exported pre-v2 (i.e., from commit `06d5dc5`) cannot be merged without re-export. The merge reader fails fast on missing `Data/snapshots/` with an explicit "re-export to enable merge" error rather than guessing.
- **Six-slice sequencing means partial value at each step.** Slices 1–3 alone — snapshot writer, history emitter, diff reader — yield a "show me what I've changed" tool. The apply path (slices 4–6) lights up the full loop, but slipping the UI work doesn't waste the data-layer investment.
