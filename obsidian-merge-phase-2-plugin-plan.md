# Phase 2 — From In-Hedgehog Smoke Test to a Real Obsidian Plugin

> **Status:** planning · not yet greenlit · superseded by no one
> **Companion to:** `obsidian-merge-smoke-test-plan.md` (phase 1) and `docs/adr/0025-obsidian-merge-roundtrip-smoke-test.md`
> **Branch convention:** `feat/obsidian-plugin-phase-2-*` when work starts

## Context

Phase 1 — the in-Hedgehog merge round-trip — exists to **validate the data contract** before committing to a plugin host that lives in someone else's product. With the smoke test working, phase 2 promotes the same mechanism into Obsidian itself, so the user never has to leave the tool they're already taking notes in.

Brett's original framing in the 12 May 2026 catch-up was that the Obsidian plugin *is* the eventual product — Hedgehog was just a controlled environment for proving the loop. Once Q1–Q5 are answered (the SDK gaps documented in phase 1 / ADR 0025), the plugin becomes the user-facing surface for what is otherwise a thin client over the 51Folds API.

Phase 2 does **not** subsume Hedgehog. Hedgehog keeps owning model *creation*, theme management, the cross-model registry, and the broader market-monitoring surface. The plugin owns *only* the merge loop on an already-exported vault.

---

## What carries over from phase 1 unchanged

Everything in the data layer ports across — it was designed to.

- **Data/snapshots/vNNN.json** schema (`schema_version: 2`).
- **Data/history.json** schema (`schema_version: 1`).
- **Merges/`<YYYY-MM-DD-HHMM>`.md** layout (frontmatter, applied/shifts/parked sections).
- Per-driver / per-outcome / overview `## History` table shape.
- Vault directory layout: `Drivers/`, `Outcomes/`, `Sources/`, `Data/`, `Merges/`, `Model.canvas`, `Drivers.base`, `Sources Index.base`, the `.obsidian/` scaffold.
- YAML frontmatter conventions in driver files — `current_state`, `possible_states`, `name`, `code`, `tier`, `entity_type`, `tags`, `parents`, `children`.
- **The selective-rewrite invariant** — H2 headings emitted by the writer are the only ones overwritten; everything else (`## Notes`, custom headings, free-form body edits) is preserved byte-identical.
- **The Q1–Q5 framework.** Notes / renames / edge wiki-links remain "detected but parked" with identical rationale until each SDK gap closes.
- Server-side endpoints — same `patch_drivers()` PATCH + poll dance; same `wait_until_complete` semantics; same opaque `submit_evidence()` and `revisions()` waiting for Q1 / Q5 answers.

---

## What's new in phase 2

### A. Host language and packaging

- **Plugin language:** TypeScript (Obsidian's plugin API is JS/TS-first).
- **Package shape:** `manifest.json` + `main.ts` + `styles.css` per Obsidian's plugin spec.
- **Plugin ID:** `fiftyone-folds-merge` (placeholder — final ID per Obsidian community-plugin naming review).

### B. Ports of phase-1 Rust modules into TypeScript

| Rust module (phase 1) | TypeScript equivalent (phase 2) | Reused logic |
|---|---|---|
| `src/obsidian/snapshot.rs` | `src/snapshot.ts` | Versioned read/write of `Data/snapshots/vNNN.json` |
| `src/obsidian/history.rs` | `src/history.ts` | `Data/history.json` plus per-note `## History` renderers |
| `src/obsidian/merge.rs` | `src/diff.ts` | `read_vault_diff()` algorithm — frontmatter parse, `## Notes` extraction, wiki-link detection |
| `src/obsidian/{driver,outcome,overview}.rs` (selective mode) | `src/rewrite.ts` | Section-boundary detection + user-zone preservation |
| `src/folds.rs::merge_drivers()` | wrap `@fiftyone-folds/sdk` (TS SDK) | PATCH + poll until re-inferred |

The TypeScript ports are mechanical — same algorithm, different syntax. The phase-1 e2e tests (vault round-trip with fixture model) become the acceptance criteria for the ports.

### C. 51Folds TypeScript SDK consumption

The TS SDK exists already (sibling repo at `../51F-SDK-TYPESCRIPT/` per `vendor/fiftyone-folds/CLAUDE.md`). Phase 2 calls:

- `client.models.patchDrivers(modelId, driverStates)` — same wire format as Rust.
- `client.models.waitUntilComplete(modelId, { interval, timeout })` — same polling contract.
- Once Q1 lands: `client.models.submitEvidence(modelId, payload)` for notes-as-evidence.
- Once Q5 lands: `client.models.revisions(modelId)` for the revision picker UI.

### D. Obsidian-side UI surface

- **Command palette entries**
  - "51Folds: Merge current vault" — runs the diff and opens the preview modal.
  - "51Folds: Apply merge" — re-opens the most recent preview if it was dismissed.
  - "51Folds: Open model dashboard" (later) — link to `Overview.md`, then to History.
- **Preview modal** — port of `render_merge_preview_dialog`. Same rows, same checkboxes, same "Apply N state changes" button. Built on Obsidian's `Modal` class.
- **Settings tab** — API key field, optional base URL override (for staging), default vault path inference behaviour, "always confirm before merge" toggle.
- **Status bar item** — small "vN" indicator showing the latest snapshot version of the current vault when the user is inside a `Drivers/*.md` or `Outcomes/*.md` file.

### E. Distribution

Three rollout tiers, in order:

1. **Sideload** — drop the plugin folder into `<vault>/.obsidian/plugins/fiftyone-folds-merge/`, enable in Obsidian's community-plugins settings. Earliest dogfood; no review process.
2. **BRAT** (Beta Reviewer's Auto-update Tool) — third-party plugin that fetches GitHub releases into Obsidian. Lets a small group of external testers install via a single setting line.
3. **Obsidian Community Plugin Browser** — official directory, requires review by the Obsidian team. Last step; only after Q1/Q2/Q3 land so the plugin's value prop is more than "press button, see same probabilities."

---

## Architecture

The plugin is a **thin client over 51Folds**, exactly like Hedgehog's merge button. The only thing different is the host.

```
┌───────────────────────────────────────────────────────────────────┐
│                          Obsidian process                         │
│                                                                   │
│  ┌───────────────────────────┐    ┌────────────────────────────┐ │
│  │ Vault (markdown + Data/)  │    │ Plugin                     │ │
│  │   Drivers/*.md            │◄───┤   diff.ts                  │ │
│  │   Outcomes/*.md           │    │   rewrite.ts (selective)   │ │
│  │   Data/snapshots/vNNN.json│    │   history.ts               │ │
│  │   Data/history.json       │    │   modal (preview)          │ │
│  │   Merges/<ts>.md          │    │   settings tab             │ │
│  └───────────────────────────┘    │   command palette          │ │
│                                   └────────┬───────────────────┘ │
└────────────────────────────────────────────│─────────────────────┘
                                             │
                                             ▼
                                ┌─────────────────────────────────┐
                                │ @fiftyone-folds/sdk (TS)        │
                                │   patchDrivers · waitUntilDone  │
                                │   submitEvidence (post-Q1)      │
                                │   revisions (post-Q5)           │
                                └─────────────────────────────────┘
                                             │
                                             ▼
                                       51Folds API
                                       (same as phase 1)
```

Hedgehog runs alongside, unchanged. The plugin doesn't know Hedgehog exists; it just sees a Hedgehog-formatted vault on disk and operates on the data contract.

---

## Open design questions for phase 2

### Q-P1. Where do the API credentials live?

Three options:

1. **Plugin settings** — user pastes `at_sk_…` into the plugin's settings tab. Independent of Hedgehog. Simplest, but the user now has the key in two places.
2. **Shared key store** — read from an OS keychain entry both Hedgehog and the plugin write to. Cleaner UX, but cross-process keychain coordination is fiddly.
3. **Delegate-to-Hedgehog** — the plugin calls a tiny local HTTP service Hedgehog exposes (`localhost:NNNN/merge-proxy`), which holds the key. Best UX, worst install story (Hedgehog must be running). Probably right *eventually*, wrong *initially*.

Recommendation: start with option 1 for sideload; revisit when the plugin graduates to the community browser.

### Q-P2. Does the plugin display probabilities, or just trigger merges?

The export already writes probability bars and Mermaid charts into markdown. Obsidian renders all of it. So the plugin doesn't *need* to display anything beyond what the vault already shows.

But the plugin could add value with:
- A right-pane Bayesian view (live probability bars that update on merge without requiring a file reload).
- A model-dashboard panel showing all models the user has vaults for, with "last merged" timestamps.

These are nice-to-haves. Phase 2a (first cut) ships without them; phase 2b adds them.

### Q-P3. Single vault per model, or multi-vault?

Phase 1 has one vault per model. Could a user have two vaults for the same model (e.g., "personal reasoning" and "team-shared")? The current schema doesn't preclude it — the snapshot carries `model_id`, so the server doesn't care.

The plugin should at minimum *detect* this (warn if two open vaults claim the same `model_id`) and ideally allow merging from any of them, with the audit note recording which vault originated the merge.

Phase 2a: explicit single-vault per model; warn on conflict.
Phase 2b: multi-vault with origin-tracking in the audit.

### Q-P4. Model creation — plugin or Hedgehog?

The plugin could grow a "Create new 51Folds model" command. The vault would then be created from inside Obsidian, with the API call happening in-plugin.

Pros: Obsidian-native workflow, no Hedgehog dependency for new users.
Cons: rebuilding Hedgehog's question-elicitation UI in Obsidian is non-trivial; we'd duplicate a lot of UX.

Recommendation: defer to phase 3. Phase 2 assumes Hedgehog created the vault and only handles merges. The plugin's "Open model" command can link out to Hedgehog if Hedgehog is installed.

### Q-P5. Plugin discovery of models — by vault scan, or by API listing?

Two ways to populate "your models" in the plugin:

1. **Vault scan** — recursively find any `Data/snapshots/` directory under the active Obsidian vault. Local-only; no network call needed for the list.
2. **API listing** — call `client.models.list()` and show every model the user owns, even ones not yet exported.

Phase 2a should ship (1) only — the plugin's value is on already-exported vaults. (2) becomes interesting once the plugin can also export (phase 3).

---

## Sequencing — six slices, parallels phase 1

1. **Snapshot reader/writer (TS).** Port `snapshot.rs`. Unit tests against the same fixture vault Hedgehog uses. No Obsidian dependency yet — plain `fs`.
2. **History reader/writer + section renderers (TS).** Port `history.rs`. Same fixture-driven tests; verifies the rendered `## History` markdown is byte-identical to Hedgehog's output for the same inputs.
3. **Vault diff reader (TS).** Port `merge.rs`. Tests use hand-crafted vault directories produced from Hedgehog fixtures.
4. **Selective rewriter (TS).** Port the user-zone preservation logic from `driver.rs` / `outcome.rs` / `overview.rs`. Tests: round-trip a vault with notes; assert byte-identical preservation.
5. **Obsidian plugin shell.** `manifest.json`, `main.ts`, settings tab with API key field, command palette entries that hook into the slices 1–4 modules. No SDK calls yet; "Merge from Vault" runs the diff and dumps it to console.
6. **SDK integration + preview modal.** Wire `@fiftyone-folds/sdk`. Port the preview dialog as an Obsidian Modal. End-to-end working: button click → diff → modal → confirm → PATCH → poll → selective re-export.

Slices 1–4 are pure module ports — no Obsidian API knowledge needed; can be done by anyone comfortable with TypeScript. Slices 5–6 require Obsidian plugin development familiarity but are mostly shell-and-glue work since the heavy lifting is in 1–4.

---

## What needs to happen on the 51Folds side first

These are gates *for the plugin to be more valuable than phase 1*, not blockers for shipping the plugin itself. The plugin can ship with the same "parked" caveats Hedgehog has today.

- **Q1 answered** — notes-as-evidence becomes the plugin's headline feature. Without Q1, the plugin's value over phase 1 is "more native UI"; with Q1, it's "your notes now reshape the model."
- **51F TypeScript SDK at parity with Rust.** Per `vendor/fiftyone-folds/CLAUDE.md`, the TS SDK exists and follows the same patterns. Confirm parity (especially the polling contract, error-shape parsing, and the response-envelope unwrap) before slice 6.
- **Q5 answered** — needed to build the revision picker. Phase 2a can ship without; phase 2b adds a "compare with revision X" UI.

Q2, Q3, Q4 are nice-to-haves that incrementally unlock the parked rows in the preview dialog. Each is a follow-up slice that just changes the "Sent to server?" column for one edit class.

---

## Verification

The phase-1 e2e tests in `src/obsidian/mod.rs::e2e_tests` become the cross-implementation contract:

- `rich_fixture_writes_complete_vault` — TS export of the same fixture must produce a byte-identical (or at least structurally identical) vault.
- `selective_re_export_preserves_user_notes_and_records_history` — TS post-merge rewrite must preserve notes byte-identically and produce equivalent `## History` / `Merges/<ts>.md` / `Data/snapshots/v002.json` content.
- `read_vault_diff_picks_up_state_change_note_and_rename` — TS diff reader must produce the same `VaultDiff` for the same hand-edited vault.
- `read_vault_diff_errors_when_snapshot_missing` — same failure mode.

Plus three new acceptance tests specific to the plugin shell:

- **Plugin loads cleanly in Obsidian** with a Hedgehog-exported fixture vault open; no console errors.
- **Settings round-trip** — API key entered, plugin reloaded, key persists.
- **End-to-end merge** — fixture vault → edit `current_state` → run "51Folds: Merge current vault" → see preview modal → click apply → mock-SDK confirms the PATCH payload matches Hedgehog's; rewritten vault is byte-identical to what Hedgehog would produce.

The plugin and Hedgehog should be interchangeable for the merge use case. Either tool can produce a merge against the same vault; the other tool can pick up the next merge against the resulting vault. This is the *real* validation that the data contract holds.

---

## Out of scope for phase 2

- **Plugin-driven model creation.** Stays in Hedgehog. See Q-P4.
- **Theme management and cross-model registry.** Stays in Hedgehog.
- **A Bayesian inference engine running inside the plugin.** Everything routes through the 51Folds server — same moat as phase 1.
- **A revisions/branching UI inside the plugin.** Gated on Q5; phase 2b at earliest.
- **An alternative merge mechanism (e.g. file-watch auto-merge).** Same explicit-action constraint as phase 1 — Brett's "press merge" framing.
- **Cross-vendor publishing.** No Roam, no Logseq, no Notion. The vault format is Obsidian-flavoured markdown.

---

## When phase 2 is "done"

- A Hedgehog-exported vault can be merged from inside Obsidian, by a user who has never run Hedgehog, with only the plugin installed and an API key configured.
- The post-merge vault is functionally identical (and ideally byte-identical) to what Hedgehog's merge would have produced for the same diff.
- Either Hedgehog *or* the plugin can be used on the same vault interchangeably across merges — the data contract is host-agnostic.
- Q1 has been answered and the plugin's preview dialog moves "Note additions" from the parked section to the applied section.

The point at which we'd write **ADR 0026: Obsidian Plugin Promotion** is when the first of those four bullets ships in `main` of a plugin repo, even as a sideload. The ADR records the decision that 51Folds-via-Obsidian is a first-class surface; this plan is the implementation guide for getting there.
