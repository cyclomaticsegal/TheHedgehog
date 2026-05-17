# Hedgehog ↔ Obsidian Round-Trip Smoke Test

## Context

Hedgehog is the proof-of-concept vehicle for an idea that Brett and Simon worked through in their 12 May 2026 catch-up: **a bounded Obsidian-style experience overlaid on top of a 51Folds Bayesian network, with a "Merge" button that re-runs the underlying model using whatever the user has accumulated in the vault as new evidence and structural input.**

The aim of this work is to land the smallest possible *real round-trip* in Hedgehog — export a 51Folds model as an Obsidian vault, let the user make changes inside that vault, and have a one-click action in Hedgehog read those changes back and re-run the model. Hedgehog is the right home for the smoke test because:

- It already exports a rich vault (per-node files, wiki-links, frontmatter, `Data/model.json` provenance).
- It already wires the 51Folds Rust SDK and owns the build / re-eval lifecycle.
- It is private (single-user, personal project), so we can iterate without committing to a plugin contract or a paying customer.

If the loop works inside Hedgehog, the same data contract ports to a real Obsidian plugin later — the plugin is just a different host for the same merge logic. Brett's literal ask in the transcript: *"Could you create an Obsidian plugin, just for argument's sake, as a sort of a smoke test, could you create a plugin that bounds Obsidian to a note-taking experience overlaid on top of a 51 folds Bayesian network?"* — answered by proving the loop client-side first.

---

## Target outcome (the full vision — what "done" eventually looks like)

These bullets describe the *complete* round-trip we are working toward. v1 in this plan delivers only the subset the 51Folds SDK currently supports; the rest is gated on open questions to the 51Folds team (next section).

1. **Notes attach to nodes, not to the vault.** Every note the user writes in the exported vault hangs off a specific driver or outcome — folder-per-driver structure, with a `## Notes` section that appends to the driver markdown. The vault's graph view, dumb as Obsidian's is, then reflects something *causally* meaningful because the wiki-links between notes inherit the model's DAG.

2. **Edges are wiki-links.** Obsidian's graph is read-only, but the underlying edges are just `[[X]]` references. The user adds an edge by adding a wiki-link in a driver's body; removes one by deleting the link. Hedgehog reads the diff and treats it as a structural change to the Bayesian net.

3. **Node renames propagate.** The user can rename a driver file in Obsidian (Obsidian itself rewrites references). On merge, Hedgehog detects the rename and pushes the new name + description to 51Folds.

4. **Semantic redefinition.** The user edits the driver's description, possible states, or context blocks (the "Why This Matters", "How It Shifts", "What To Monitor" sections). On merge, those flow back as new context for re-elicitation.

5. **Notes as evidence (RAG).** Accumulated notes in `## Notes` sections become evidence packets pushed to the server's elicitation pipeline. The server re-considers driver states *given the user's accumulated reasoning*.

6. **Driver state changes round-trip.** The user can mark a driver in a different state inside the vault. Merge pushes that as a scenario, server re-infers, vault gets the updated probability bars.

7. **Merge is an explicit user action with a confirmation step.** Not file-watching, not continuous sync. User edits over time, presses Merge once, sees a preview of what will be sent, confirms, and gets updated probabilities written back into the same vault.

8. **Re-write is an extension, not a fork.** The merge updates the same model_id; revisions are tracked on the 51Folds side (the SDK already exposes `revisions()`), so the user can compare before/after.

9. **The vault remembers.** Each merge writes a versioned snapshot, an audit note, and a row to per-driver and per-outcome inline `## History` tables. The user can scroll any driver and see its state journey, or open the dashboard at the bottom of `Overview.md` to flip between merges.

---

## Why this matters (rationale, in Simon and Brett's words)

The thinking that shaped this target outcome, distilled from the Catchup transcript:

- **Obsidian alone is "a graveyard of data."** Simon: *"It just lurches out of control… after you've got 100 notes in there, you can't make head or tail out of it. It's just connecting words."* The free-form graph view connects text as nodes but has no causal structure — adding utility means imposing structure.

- **51Folds is the structure.** Brett: *"What we build, we build a meta narrative and we tell them. We bound the user to build across the top of a structured meta-narrative that makes sense, and because it's like this, you know you can then push your notes and you can your notes have a value beyond the semantic… there's something beyond it."* The Bayesian network gives Obsidian-style note-taking a coherent scaffold to hang off.

- **The merge button is the killer pattern.** Brett: *"You press a button, your expertise, your worldview now gets sucked into the underlying scaffold, and we we re-elicit probabilities against the semantic layer that you've deposited over some amount of time."* A single explicit action — not continuous sync — is cheap to build and easy to reason about.

- **It's a Bayesian experience without the user knowing.** Brett: *"They're playing with semantics. They're playing with the drivers, they're in their own head, connecting how it works, and then we just fly around that… The object represents my opinion better."* The user does Bayesian operations (intervention, evidence injection, structural editing) through the natural verbs of notes and links.

- **Structured reasoning is the revolution we are after.** Brett: *"There's the automation revolution and there's the second pillar, which I think is the structured reasoning revolution… AI is the mother of dragons and it has, there's two dragons in here."* Prediction is one pillar; this round-trip is the wedge that lets 51Folds claim the structured-reasoning pillar without sounding too abstract.

- **The moat is server-side re-elicitation.** Simon: *"If we don't produce rag, then we're giving up too much opportunity because the kind of rag we can produce is gold."* The plugin / Hedgehog stays a thin client; the merge logic and re-elicitation live behind the 51Folds API. That keeps the defensible niche on the server side, where any third-party host (Obsidian, Claude Co-work, our own UI) consumes the same contract.

- **Prove it client-side first.** Brett: *"Yeah, well, look, start the other way around. Could you create an Obsidian plugin, just for argument say, like as a sort of a smoke test… How does that work? What does that look like?"* — but the cheaper smoke test is *inside Hedgehog*, where the SDK is already wired and we control both ends of the loop.

---

## What the 51Folds SDK currently supports — and what it doesn't

The vendored 51Folds Rust SDK at `vendor/fiftyone-folds/` exposes 11 model-related methods. Of those, only **two paths** can mutate a built model:

- `patch_drivers()` — PATCH `/api/v1/models/{id}/drivers`, partial merge of driver *states*, triggers server re-inference. **Wired in Hedgehog** at `src/folds.rs:217` (used by the existing Re-evaluate button).
- `update_drivers()` — PUT `/api/v1/models/{id}/drivers`, atomic replace of all driver states, triggers server re-inference. **Wired in Hedgehog** at `src/folds.rs:299` (used by Revert-to-Original).

Also present but unused:
- `submit_evidence()` — POST `/api/v1/models/{id}/evidence`, accepts opaque `serde_json::Value`, returns opaque `serde_json::Value`. **Not wired in Hedgehog.** Contract is undocumented in the SDK.
- `revisions()` — GET `/api/v1/models/{id}/revisions`, returns full revision history. **Not wired.**

**Not supported by the SDK at all:**
- Driver rename / description / context edit.
- Outcome rename / edit.
- Edge add / remove (causal structure is immutable post-build).
- Model question / context edit.
- "Re-elicit with this text as new context" (no documented call path).
- Fork / branch a model from an existing one.

---

## Open questions for the 51Folds team (gate items 1–5 of the target outcome)

Until these are answered, items 1, 2, 3, 4, and 5 of the target outcome cannot be implemented as real round-trips. The merge button can *detect* those edits in the vault and *display* them in the preview, but it cannot push them to the server. v1 surfaces them as "client-side only — pending 51Folds support."

### Q1. What is `submit_evidence()` meant to do?
The method exists in the SDK but is opaque. Concretely:
- What JSON shape does the server accept?
- Is the body interpreted as RAG / unstructured evidence, or as structured updates (e.g. `{ driver_code, new_belief }`)?
- Does it trigger a full re-elicitation, or is it a passive log?
- Is it idempotent? Cumulative? Does each call build on prior submissions, or replace them?
- What does the response payload contain (re-elicited probabilities, justification, nothing)?

**Why this question matters:** If `submit_evidence()` accepts a text body + per-driver context, it unlocks item 5 (notes as evidence). If it accepts structural overrides, it might unlock items 3/4 too.

### Q2. Is there a planned endpoint for driver metadata edits?
Driver `name`, `description`, `state_descriptors`, and `context` blocks are currently read-only post-build. The full vision (items 3 and 4) requires PATCHing these. Is there:
- An existing internal endpoint not yet in the SDK?
- A planned endpoint on the roadmap?
- A reason this is intentionally read-only (e.g. would invalidate the build's expert elicitation)?

### Q3. Is edge mutation possible after build?
Item 2 of the target outcome — "edges are wiki-links" — requires the user's edge add/remove in the vault to flow to the server. The SDK has no edge-mutation surface. Is the causal structure immutable by design (a Bayesian invariant after elicitation), or is it just unsupported?

If immutable: the vision needs to reframe "scrubbing edges" as "scrubbing edges in the user's *local* view" — useful for the user's reasoning but not propagated to the server's network. That's still valuable (it changes what the user pays attention to next time) but it's a different product story.

### Q4. What does re-elicitation with new context actually mean server-side?
Brett's framing in the transcript: *"I want you to give me better predictions, given you've just spent days or weeks adding notes, whatever, and now you want better predictions."* What does the server need from us to do that?
- Just driver state values? (Today's PATCH.)
- A blob of text per driver? (Possibly `submit_evidence()` with a per-driver payload.)
- A blob of text at the model level? (Possibly `submit_evidence()` with a top-level payload.)
- Structured "expert assertions" (e.g. `driver A should be in state X with probability Y under condition Z`)?

The answer determines the shape of the merge-time request and the contract Hedgehog should target.

### Q5. Are revisions linear or branching, and are they user-facing?
The SDK exposes `revisions()`. The Hedgehog ignores it today. For the round-trip:
- Does each merge create a new revision the user can compare against?
- Can revisions be tagged ("baseline", "after Q1 notes", "after Q2 notes")?
- Can a user revert to a specific revision?

If revisions are first-class, the merge UI can show the diff as `Revision N → Revision N+1` rather than as a one-off probability shift.

These five questions form a single document we can send to Chris / the Bayesian specialist. They aren't blockers for v1, but each is a blocker for one of items 1–5 of the full vision.

---

## v1 scope (what we can ship honestly today)

A real round-trip on the only knob the SDK exposes: **driver state changes**. The user opens an exported vault, edits a driver's `current_state:` value in its frontmatter (or via a `state: <new value>` callout we standardize), presses **Merge from Vault** in Hedgehog, sees a confirmation preview of *every* detected change (driver state edits + every other class of edit, the latter clearly flagged as "client-side only — pending SDK support"), confirms, and Hedgehog calls `patch_drivers()` for the state changes only. The server re-infers; Hedgehog writes the updated probabilities and a fresh `Data/model.json` back into the same vault.

The user can see the loop work. Brett's "press merge" affordance exists. The architecture supports adding the remaining round-trips as soon as Q1–Q5 are answered — nothing in v1 dead-ends the larger plan.

The vault is the historian. Each merge writes a new versioned snapshot, a per-merge audit note, and a new row to every affected driver's and outcome's `## History` table; `Overview.md`'s new `## History` section acts as the dashboard. User-authored content (`## Notes`, free-form wiki-links, body edits outside model-derived sections) is preserved verbatim across merges — the model-derived layer rewrites cleanly underneath.

---

## First steps (after exiting plan mode)

1. Create the feature branch from `main`:
   ```bash
   git checkout -b feat/obsidian-merge-smoke-test
   ```
2. Persist this plan into the repo root so the context, rationale, target outcome, and SDK questions survive future plan-mode sessions (which overwrite the sandboxed plan file):
   ```
   /Users/simonsegal/lws/TheHedgehog/obsidian-merge-smoke-test-plan.md
   ```
   Verbatim copy of this plan. Tracked in git on the feature branch so it travels with the work.

All v1 work happens on the feature branch.

---

## v1 implementation

### A. Export schema additions (`src/obsidian/`)

The existing export module is already well-organized (see `src/obsidian/mod.rs` → `write_vault()`, with sub-modules `driver.rs`, `outcome.rs`, `overview.rs`, `source.rs`, `canvas.rs`, `base.rs`, etc.). Six additions, in order from the data layer up to the user-facing surface:

1. **`Data/snapshots/v001.json, v002.json, …`** — a versioned directory replacing the single `Data/snapshot.json`. Each file captures canonical state at the moment it was written: `{ model_id, exported_at, driver_states: { code: state }, outcome_probabilities: { id: p }, schema_version: 2 }`. The original export writes `v001.json`; each successful merge appends `v{N+1}.json`. The highest-numbered file is the diff baseline for the *next* merge. Written by a new function in `src/obsidian/snapshot.rs`. The existing `Data/model.json` keeps its role as the most recent raw response.

2. **`Merges/<YYYY-MM-DD-HHMM>.md`** — one markdown audit note per successful merge. Lists driver state changes applied, parked edits detected but not sent (per Q1–Q3), and the outcome posterior shifts the server returned. Navigable like any Obsidian note; back-linked from per-driver / per-outcome `## History` rows.

3. **`## History` section in `Drivers/*.md`** — emitted between the existing `## Current State Rationale` and `## Local Causal Map` sections. Inline table: `vN | timestamp | before → after | [[merge note]]`. A row is appended on every merge that changed this driver.

4. **`## History` section in `Outcomes/*.md`** — emitted after the existing `## Causal Path` section. Same table shape, tracking probability changes per merge.

5. **`## History` section appended to `Overview.md`** — after the existing `## Sources` section. Table of every merge with timestamp, change count, wiki-link to the per-merge note, wiki-link to its snapshot. Acts as the vault-side dashboard; no new top-level file in v1 (a standalone `History.md` is a possible follow-up if this section grows past Overview's natural fit).

6. **Stable canonical state convention in `Drivers/*.md`** — already in place: `current_state:` in YAML frontmatter (`src/obsidian/driver.rs:139–205`). No format change; v1 locks this as the authoritative location the merge reader looks at.

### B. New module `src/obsidian/merge.rs` — read-back + diff

A pure-Rust module that takes a vault path and returns a typed diff:

```rust
pub struct VaultDiff {
    pub model_id: String,                                  // from latest Data/snapshots/vN.json
    pub vault_path: PathBuf,
    pub driver_state_changes: Vec<DriverStateChange>,      // → PATCH /drivers
    pub note_additions: Vec<NoteAddition>,                 // client-side only (Q1)
    pub driver_metadata_changes: Vec<DriverMetadataChange>, // client-side only (Q2)
    pub edge_changes: Vec<EdgeChange>,                     // client-side only (Q3)
    pub stale: bool,                                       // schema_version mismatch
}

pub struct DriverStateChange { pub code: String, pub old_state: String, pub new_state: String }
pub struct NoteAddition       { pub driver_code: String, pub appended_text: String }
pub struct DriverMetadataChange { pub code: String, pub field: MetadataField, pub old: String, pub new: String }
pub struct EdgeChange         { pub parent: String, pub child: String, pub kind: EdgeChangeKind }

pub fn read_vault_diff(vault_path: &Path) -> Result<VaultDiff>
```

The reader:
1. Loads the highest-numbered `Data/snapshots/v*.json` to anchor the diff. If the directory is missing or empty → error: "this vault wasn't produced by Hedgehog with snapshot support; re-export to enable merge."
2. Walks `Drivers/*.md`, parses YAML frontmatter (use the existing `serde_yaml` already in the tree, or `gray_matter`).
3. For each driver: diff `current_state`, `name`, body sections; detect `## Notes` content beyond what the export would have produced; diff wiki-link sets in body against snapshot edges.
4. Returns a `VaultDiff`. Pure function; no I/O beyond file reads.

### C. New SDK call path — already exists

No new SDK code. The existing `patch_drivers()` in `src/folds.rs:217` is reused as-is. The merge action collects `DriverStateChange`s into a `Vec<DriverStateInput>` and calls the same path the Re-evaluate button uses.

### D. UI surface (`src/app.rs`, `src/app/dialogs.rs`)

1. **A "Merge from Vault" button** placed adjacent to "Export to Obsidian" on the model detail toolbar (around `src/app.rs:6547–6592`). Enabled when:
   - A model is loaded (foreground or viewed) — same gate as Export.
   - The user has previously exported this model *or* picks a vault path via folder picker.
   - No merge / re-eval is currently in flight.

2. **A new `MergeTask`** on `DashboardApp`, mirroring `ObsidianTask` (`src/app/tasks.rs:651–703`). Carries `in_flight`, `rx`, `error`, `last_merged_at`. Background thread reads the diff and posts a `MergeEvent::DiffReady(VaultDiff)` or `MergeEvent::Failed(msg)` back to the main loop.

3. **A new modal dialog** `render_merge_preview_dialog` in `src/app/dialogs.rs`. Shows:
   - Header: "Merge changes from Obsidian vault into model {model_id}?"
   - Section **Will be applied (driver state changes):** Each `DriverStateChange` as a row `CODE  old_state → new_state`. With a checkbox per row so the user can deselect specific ones.
   - Section **Detected, not applied (pending SDK support):** Counts of `NoteAddition` / `DriverMetadataChange` / `EdgeChange`, each with a "Why not applied?" expander linking to the matching Q1–Q3 in the open-questions doc. Phrased so it's clearly aspirational, not a bug.
   - Buttons: **Cancel** / **Apply selected state changes** (only enabled when ≥1 state change is checked).

4. **On Apply** — collect selected state changes into a `Vec<DriverStateInput>`, call `start_folds_merge()` (a new method on DashboardApp) which delegates to a new `folds::merge_drivers()` thin wrapper around the existing `patch_drivers()` path. While the call is in flight, the toolbar shows a "Merging…" spinner. On success: run a *selective* re-export over the same vault. Only model-derived content rewrites — driver frontmatter, probability bars, structured sections (`## Possible States`, `## Why This Matters`, `## How It Shifts`, `## What To Monitor`, `## Local Causal Map`, `## In The Model`), outcome posteriors, and the new `Data/snapshots/v{N+1}.json`. User-authored content is preserved byte-identical: any `## Notes` section, any free-form additions, any user-added wiki-links in driver bodies. The export also appends a new `Merges/<timestamp>.md`, appends rows to each affected driver's and outcome's `## History` tables, and appends a row to `Overview.md`'s `## History` table. On failure: surface the error inline; vault and server stay untouched.

### E. Persistence

`MergeTask::last_merged_at` and `last_merged_vault_path` are session-only (mirror `ObsidianTask::last_exported`). A `data/merge.log` line is appended on each successful merge for auditability — same daily-rotated logging pattern used elsewhere.

### F. Files to be created / modified

| File | New / Modified | Purpose |
|---|---|---|
| `src/obsidian/snapshot.rs` | new | Write versioned snapshots to `Data/snapshots/vN.json`; read latest as diff baseline |
| `src/obsidian/history.rs` | new | Emit `Merges/<timestamp>.md`; append rows to per-note `## History` tables and to `Overview.md`'s `## History` section |
| `src/obsidian/merge.rs` | new | `read_vault_diff()` + diff types |
| `src/obsidian/mod.rs` | modified | Call `snapshot::write()` and `history::write()` from `write_vault()`; introduce a "selective" rewrite mode that preserves user zones |
| `src/obsidian/driver.rs` | modified | Emit `## History` section between `## Current State Rationale` and `## Local Causal Map`; selective-rewrite mode preserving `## Notes` and any non-canonical user-authored sections / wiki-links |
| `src/obsidian/outcome.rs` | modified | Emit `## History` section after `## Causal Path`; same selective-rewrite contract |
| `src/obsidian/overview.rs` | modified | Append `## History` section after `## Sources` |
| `src/app/tasks.rs` | modified | Add `MergeTask`, `MergeEvent` |
| `src/app/dialogs.rs` | modified | Add `render_merge_preview_dialog` |
| `src/app.rs` | modified | Add `merge_task` field, `start_folds_merge()`, the toolbar button next to Export, the dialog dispatch |
| `src/folds.rs` | modified | Thin `merge_drivers()` wrapper around existing `patch_drivers()` flow (just for naming clarity in the merge call site) |
| `docs/adr/0025-obsidian-merge-roundtrip-smoke-test.md` | new (post-implementation) | Documents the decision, the schema bump, the deferred items per Q1–Q5 |

### G. Verification

1. **Round-trip a single driver state edit.** Open the app on a built model. Click **Export to Obsidian**, pick a folder. Open the vault. Open `Drivers/<code> — <name>.md`. Edit `current_state:` to a different value (one of the entries in `possible_states:`). Save. Back in Hedgehog, click **Merge from Vault**. Dialog shows exactly one change in "Will be applied"; zero detected in the client-side sections. Apply. Wait for re-inference. Re-open the vault; outcomes have new probabilities; a fresh `Data/snapshots/v002.json` exists alongside the original `v001.json`; the driver's `## History` table has a new row; a `Merges/<timestamp>.md` audit note exists; `Overview.md`'s `## History` section has a new row.

2. **Client-side-only detection works.** Edit the driver file's `name:` and append a paragraph under `## Notes`. Merge. Dialog shows one detected metadata change and one note addition in the **Detected, not applied** section, each with a "Why not applied?" expander citing Q1/Q2. Apply with no state-change rows checked → button disabled; user is forced to cancel.

3. **Stale snapshot guard.** Manually delete the `Data/snapshots/` directory from the vault. Merge. Dialog (or inline error) says "this vault has no Hedgehog snapshot; re-export to enable merge."

4. **No model loaded → no button.** Switch to a tab where no model is in the viewer; "Merge from Vault" should not be visible (gate identical to Export).

5. **Concurrent build doesn't break merge.** Start a fresh model build (`◌ 1 building` chip showing). Merge against a different completed model in the viewer. Foreground build keeps polling; merge fires PATCH against the viewed model_id; results write back to that model's vault. Tray chip count unchanged.

6. **Selective re-export preserves user content.** Edit a driver's `current_state` AND append a paragraph under a fresh `## Notes` section. Merge. After write-back, the `## Notes` paragraph is byte-identical to what the user typed; the driver's model-derived sections and probability bars reflect the new server inference.

7. **Versioned snapshots accumulate.** After two merges, `Data/snapshots/v001.json`, `v002.json`, and `v003.json` all exist (v001 = original export; v002, v003 = post-merge baselines). The diff reader uses the highest-numbered file as the baseline for the *next* merge.

8. **History accumulates inline.** After a merge that changed driver `X`'s state, `Drivers/X — *.md` has a new row in its `## History` table; `Merges/<timestamp>.md` exists with a summary; `Overview.md`'s `## History` section has a new row linking to that merge note. Affected outcomes also have new `## History` rows.

9. **Build clean.** `cargo build --release` and `cargo clippy --release` both pass with no new warnings.

---

## What v1 explicitly does **not** do, and why

Each deferred item maps to an open question above. v1's preview dialog surfaces them so the user sees the loop is *partial*, not broken.

| Deferred item | Blocked on | Detected in vault? | Sent to server? |
|---|---|---|---|
| Notes-as-evidence (target outcome #1, #5) | Q1 (`submit_evidence` semantics) | Yes | No |
| Driver rename / description / context edits (#3, #4) | Q2 (no metadata PATCH endpoint) | Yes | No |
| Edge add / remove via wiki-link diff (#2) | Q3 (no edge-mutation endpoint) | Yes | No |
| Outcome label edits | Q2 | Yes | No |
| Model-question / context edits | Q2 | No (not surfaced in driver files) | No |
| Revision tagging / comparison UI | Q5 | n/a | n/a |

The moment any of Q1–Q5 is answered, the matching deferred item becomes a follow-up slice: the diff types already model the edits, the dialog already surfaces them — only the "Sent to server?" column changes.

---

## Sequencing

Six independent slices, low-risk first:

1. **Snapshot writer.** `src/obsidian/snapshot.rs` + `mod.rs` integration. Writes `Data/snapshots/v001.json` on first export and `v{N+1}.json` on each subsequent re-export. Zero UI consumers; verify via `cat` on a freshly exported vault.
2. **History emitter + selective-rewrite plumbing.** `src/obsidian/history.rs` plus the selective-rewrite extension to `driver.rs` / `outcome.rs` (`## History` section emission, preservation of user-authored zones), plus the `## History` section appended to `Overview.md` via `overview.rs`. Slices 1 and 2 compose: both must land before any merge action can write back coherently. Verifiable by hand-constructing a second export of an existing vault and inspecting the new sections / files — no merge UI needed yet.
3. **Vault diff reader.** `src/obsidian/merge.rs` as a pure module. Unit-tested against a fixture vault (the existing rich-model fixture at `vendor/fiftyone-folds/tests/fixtures/model-rich.response.json` can seed a vault, then we hand-edit it to produce diff cases).
4. **`MergeTask` + background thread.** No UI yet; threaded path returning `MergeEvent::DiffReady` is verifiable by logging.
5. **Preview dialog.** `render_merge_preview_dialog` in `dialogs.rs`. Render-only first — show the dialog from a temporary debug button before wiring real flow.
6. **Toolbar button + apply path.** Wire `start_folds_merge` to the existing `patch_drivers()` path; trigger the selective re-export on success. End-to-end working.

The user can stop after step 3 and still ship a useful audit tool ("show me what changed in my vault"); the round-trip lights up at step 6.

---

## Out of scope for v1 (independent of the SDK questions)

- A real Obsidian plugin. The smoke test stays inside Hedgehog. The plugin work — if it happens — uses the same `VaultDiff` data contract and re-implements only the host side.
- Multiple vaults per model, or multiple models per vault.
- Conflict resolution if the model changed on the 51Folds side between export and merge. v1 detects the case via the snapshot's `model_id` + `exported_at` and refuses to merge if the server's model timestamp is newer than the snapshot.
- Ambient / file-watcher merge. Explicit user action only, per Brett's framing.
- An ADR document — landed after implementation, not before.
- **History pruning.** v1's `## History` tables grow unbounded per merge. If a vault becomes unwieldy after many merges, a later slice can introduce "last N rows + link to archive note."
- **Cross-revision diff UI inside Hedgehog.** v1's history surface is vault-side only. The server's `revisions()` endpoint stays unwired in v1; the vault is source of truth until Q5 is answered. A possible follow-up: extract `## History` into a standalone `History.md` if the section grows past Overview's natural fit.
