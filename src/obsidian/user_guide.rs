//! `How to Edit & Merge.md` at the vault root — the user-facing guide
//! to round-tripping changes from this vault back into the 51Folds
//! model via Hedgehog's **Merge from Vault** button.
//!
//! Static content (no model fields are interpolated). Regenerated on
//! every export and every selective re-export so the file is always
//! current with the merge contract — users shouldn't edit it because
//! their changes would be overwritten next merge. There's a banner at
//! the top saying so.

use anyhow::{Context, Result};
use std::path::Path;

pub(crate) const FILENAME: &str = "How to Edit & Merge.md";

pub(crate) fn write(vault_root: &Path) -> Result<()> {
    let path = vault_root.join(FILENAME);
    std::fs::write(&path, BODY)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

const BODY: &str = r#"---
tags: [user-guide]
---

# How to Edit & Merge

This vault is a working copy of a 51Folds Bayesian model. Edit it in Obsidian, then press **Merge from Vault** in Hedgehog to send your changes back to the server and re-elicit the probabilities.

> [!warning] This file is regenerated on every export — your edits to it will be lost. Put your own reasoning in `## Notes` sections on driver pages instead.

## What you can edit

### 1. Driver state — round-trips to the server

Open any file under `Drivers/`. The top of the page shows a **Properties** panel with `code`, `name`, `current_state`, `possible_states`, and so on. Click the `current_state` value and replace it with any value listed in `possible_states` (case-sensitive, exact match).

If you'd rather edit the raw YAML, press **Cmd+E** (macOS) or **Ctrl+E** (Windows/Linux) to switch to Source Mode — the Properties panel becomes a block of plain text between two `---` lines.

This is the only edit class that's actually sent to the server in v1.

### 2. Notes on a driver — detected, parked pending Q1

At the bottom of any driver page, add a section like:

```
## Notes
Yellen 14 May sounded dovish. Watch FOMC minutes 28 May.
```

The text under `## Notes` is preserved byte-identical across every merge — your accumulated reasoning stays in the vault.

The merge preview will show your notes count under **Detected, not applied (pending SDK support)** with a one-line example for the first three. They aren't sent to the server yet; once we know how `submit_evidence()` accepts text (Q1), they will.

### 3. Rename a driver — detected, parked pending Q2

In the driver's Properties panel, edit the `name:` field. The merge preview surfaces this as `Driver metadata edits × N` with a row `<code> — name: "old" → "new"`. The rename stays in the vault until the SDK exposes an endpoint to push it (Q2).

### 4. Edges via wiki-links — detected, parked pending Q3

Inside a driver's `## Notes` section (and only there — wiki-links elsewhere are noise from the export itself), add a wiki-link to another driver. The simplest way is to type `[[` and let Obsidian autocomplete from the vault:

```
## Notes
This shifts together with [[Drivers/MONP — Monetary Policy]] historically.
```

The merge preview shows this as `Edges × N`. The DAG isn't currently mutable post-build (Q3), so the link is recorded for your reasoning but not pushed to the server.

## What you can **not** edit

- Outcome probabilities — the server computes them. Reflected in `Outcomes/*.md` after each merge.
- The model's question or `## Background`.
- A driver's `possible_states` set.
- Edges drawn in `## Local Causal Map` or `Model.canvas` — those follow the server's DAG.

If you change `current_state` to a value that isn't in `possible_states`, the diff will pick it up but the server will reject the PATCH on merge.

## The merge round-trip

1. Click **Merge from Vault** in Hedgehog (next to **Export to Obsidian**). The folder picker defaults to where you exported.
2. The preview dialog shows:
   - **Will be applied** — one row per detected `current_state` change, each with a checkbox. Untick any you don't want to send.
   - **Detected, not applied (pending SDK support)** — counts and example rows for parked notes / renames / edges.
3. Click **Apply N state changes**. Hedgehog PATCHes the server, polls for re-inference, then selectively rewrites this vault:
   - Driver and outcome pages get fresh probability bars and structured sections.
   - Your `## Notes` are preserved verbatim.
   - A new `Merges/<timestamp>.md` audit note is created.
   - Per-driver and per-outcome `## History` sections get a new row.
   - `Overview.md`'s `## History` section gets a new row linking to the audit note.
   - A new `Data/snapshots/v{NNN}.json` is written — the baseline for *next* merge's diff.

## The five open questions (Q1–Q5)

The "parked" rows in the merge preview all point to open questions for the 51Folds team:

| # | Question | Unlocks |
|---|----------|---------|
| Q1 | What does `submit_evidence()` accept and return? | Notes as evidence (#2 above). |
| Q2 | Is there an endpoint to edit driver metadata? | Renames, descriptions (#3 above). |
| Q3 | Can edges mutate post-build? | Structural edits via wiki-links (#4 above). |
| Q4 | What does re-elicitation with new context need server-side? | Per-driver text-as-context. |
| Q5 | Are revisions linear or branching, and user-facing? | Revision tagging / comparison UI. |

As each is answered, the matching row in the preview dialog stops being "detected, not applied" and starts being sent to the server.

## Where the design lives

- `obsidian-merge-smoke-test-plan.md` (Hedgehog repo root) — full implementation plan with file-by-file changes.
- `docs/adr/0025-obsidian-merge-roundtrip-smoke-test.md` — ADR capturing the architectural decisions.
- `Data/snapshots/` and `Data/history.json` (here in the vault) — the merge engine's source of truth.
"#;
