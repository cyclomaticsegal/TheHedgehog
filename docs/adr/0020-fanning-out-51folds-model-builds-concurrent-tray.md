# ADR 0020: Fan-out 51Folds Model Builds — Concurrent Multi-Build State, Toolbar Tray, and File Logging

**Date:** 2026-04-16
**Status:** Accepted
**Extends:** ADR 0008 (51Folds integration), ADR 0013 (SDK integration, model explorer), ADR 0019 (Dexter integration, background build indicator)

## Context

From the introduction of 51Folds through ADR 0019, the Hedgehog allowed exactly one model build at a time. `FoldsTask` held a single `in_flight: bool`, a single `rx: Option<Receiver<FoldsResult>>`, a single `model_id`, and a single `model`. Every entry point — the Create button in the hypothesis editor, the resume-on-startup sweep, Dexter `/51folds` — funnelled through this one slot.

This was fine when 51Folds was new and one-build-at-a-time matched user pace. It stopped being fine once three origins could produce hypotheses (AI Analysis on chart view, chart+overlay "Analyze Current View", Dexter `/51folds`) and users began treating the app as a multi-thesis workbench.

The failure mode that prompted this ADR: a user ran `/51folds` in Dexter while an earlier model was still building. The new hypothesis loaded into the sidebar editor, but `load_historical_inference` deliberately preserved the prior build's `in_flight` + `rx` across its reset. The in-flight render gate early-returned with the old spinner, so the Create button for the new hypothesis was never drawn. When the first model completed, the completed-model UI replaced the editor entirely, and the second hypothesis was silently discarded — never submitted to the 51Folds API.

ADR 0019 §8 ("Background Model Build Indicator") was a partial mitigation: it surfaced a small "New model building… [Show]" bar when a build was in flight but the user was viewing a different *already-loaded* model. It did nothing for the case where a build was in flight and a *new hypothesis* arrived needing its own build.

The deeper observation: the `folds_models` SQLite table has always been multi-valued — one row per build, each with its own `model_id`, `status`, `created_at`, and `inference_id`. The persistence layer was already a queue. The in-memory singleton was a UI fiction layered on top.

## Decisions

### 1. Foreground `FoldsTask` plus a `FoldsBacklog` of others

`FoldsTask` stays — it represents the build currently visible in the central Model view, with all its existing fields (`in_flight`, `rx`, `model_id`, `model`, `draft_drivers`, `previous_outcomes`, `reevaluating`, `refresh_*`, `error`). This is the "foreground" build; UI code that reads and mutates `self.folds_task.*` is unchanged.

A new `FoldsBacklog` sits next to it:

```rs
struct FoldsBacklog {
    /// Backgrounded builds keyed by model ID. Each retains its own rx,
    /// in_flight flag, and eventual model response.
    background: HashMap<String, FoldsTask>,
    /// Newly-spawned builds whose model_id hasn't arrived yet (between
    /// the Create click and the first `FoldsResult::Created`).
    pending_creates: Vec<FoldsTask>,
}
```

The foreground/backlog split matches the mental model of the UI: there's the one we're looking at, and there's everything else that happens to be running. It also minimises the diff — the ~80 existing `self.folds_task.*` call sites stay as-is. Only the spawn, poll, tray, and selection paths need awareness of the backlog.

### 2. Spawn rules

When `start_folds_create` fires and `self.folds_task` already has an in-flight or completed build:

- If it has a `model_id`, move it into `backlog.background[model_id]`.
- If it has no `model_id` (still in the pending-create window), move it into `backlog.pending_creates`.
- Reset `self.folds_task` and start the new build in it.

When the user clicks a backlog entry (via the tray), the foreground and the chosen backlog entry swap: the current foreground moves into the backlog, the chosen entry becomes the foreground. The central Model view always renders the foreground, so the rest of the rendering code is unaffected.

### 3. `load_historical_inference` no longer stashes in-flight state

Previously, loading a different inference while a build was in flight preserved the prior build's `rx` across the reset. That was the proximate cause of the silent-discard bug. The new behaviour: if `self.folds_task.in_flight` is true when a new inference loads, the current foreground is pushed to the backlog; the folds task is then reset cleanly and loaded with whatever model is linked to the new inference.

### 4. Toolbar tray chip

A persistent compact indicator lives right-aligned in the `rs_toolbar` row: `◌ N` where N is the total count of in-flight builds (foreground + backlog + pending-creates). Hidden when zero. Clicking opens a popover listing each active build — short hypothesis label, model ID, elapsed time. Clicking an entry swaps it into the foreground and switches to `CentralView::Model`.

The tray is the single non-modal affordance that says "things are happening." It replaces the ADR 0019 §8 inline bar in the sidebar, which is now removed.

### 5. Resume sweep routes all pending builds

`resume_pending_folds_models` previously picked a single "UI candidate" — the most recent non-expired pending row — and routed it through `FoldsTask` with a channel, while every other pending row got a fire-and-forget `resume_poll` that wrote to the DB silently. The new behaviour: the most recent becomes the foreground, every other pending row becomes a backlog entry with its own `FoldsTask` and channel. All of them show in the tray.

### 6. Live badge updates

The `folds_status_by_inference` map driving the sidebar list (`src/app.rs:6099-6118`) and Report view (`src/app.rs:3740`) badges now refreshes on every terminal event — `Created` flips the row from □ to ◌, `Completed` to ●, `Failed` to ⚠ — not just on inference-history reload. Users see state change as it happens, for both foreground and backlog builds.

### 7. File logging via `tracing` + rolling file appender

`main.rs` initializes a `tracing_subscriber` with two layers: stderr (unchanged dev behaviour) and a daily-rotating file under the platform data directory (`~/Library/Application Support/TheHedgehog/logs/hedgehog.log` on macOS). Critical `eprintln!` sites in `folds.rs` — build created, completed, failed, poll timeout, patch errors — become `tracing::info!` / `tracing::warn!` calls with `model_id` as a structured field. Less-critical `eprintln!` sites remain as-is.

This is insurance, not a direct fix. The ADR 0019 incident was unreconstructable because the app was launched from Finder (stderr went nowhere). A forensic log trail gives the next incident something to read.

## Consequences

- Users can submit builds from any origin while others are running. Dexter `/51folds` no longer needs to time its output around existing builds; the chart-view and overlay analyses are equivalent.
- The silent-discard bug that prompted this ADR is eliminated at the source: loading a new hypothesis never clobbers an in-flight build's channel; it goes to the backlog.
- Keeping `FoldsTask` as the foreground minimises the change to rendering code (driver pills, re-eval, refresh, revert, DAG view, all unchanged). The cost is a slightly less pure model: the "current build" is special-cased rather than a peer entry in a set. The pragmatic payoff is worth it.
- Re-evaluate / Refresh / Revert-to-original flows operate on the foreground build as they always have. Switching via the tray swaps foreground and backlog, so a re-eval always targets whichever build is visible — no risk of the wrong build being mutated.
- The tray is a new UI surface. It's minimal (a pill + popover) and lives in existing layout real estate (right-aligned in `rs_toolbar`), so no window-level restructuring.
- The ADR 0019 §8 "New model building… [Show]" bar is removed as redundant. Users who relied on it get the tray instead.
- File logging introduces a new on-disk artefact users may need to know about for support triage. One line in `src/help.rs` points at the path.
- The tracing migration is deliberately partial. The long tail of `eprintln!` calls in `app.rs` / `storage.rs` / `dag.rs` stay as-is until there's a reason to change them. This ADR is about the fan-out; logging is a rider.
