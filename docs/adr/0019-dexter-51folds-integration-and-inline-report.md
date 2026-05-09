# ADR 0019: Dexter /51folds Integration, Research-to-Model Pipeline, and Inline Report View

**Date:** 2026-04-14
**Status:** Accepted
**Extends:** ADR 0008 (hypothesis generation and model creation), ADR 0013 (SDK integration)

## Context

The Hedgehog had two separate workflows that didn't connect: the AI Analysis pipeline (built-in LLM analysis of VIX/commodity data → hypothesis → 51Folds model) and the Dexter research agent (embedded terminal for conversational financial research). A user doing deep research in Dexter had no path to turn that research into a 51Folds model without manually copy-pasting.

Additionally, the Summary Report was rendered as an `egui::Window` dialog, which created z-ordering issues with the terminal (egui_term renders at a level that covers egui windows) and felt disconnected from the central panel architecture.

## Decisions

### 1. `/51folds` slash command in Dexter

A new command added to the vendored Dexter codebase (`vendor/dexter/src/cli.ts`) that:

1. Gathers the full conversation history from `InMemoryChatHistory`
2. Builds a hypothesis-generation prompt with optional user-supplied directional text
3. Runs the agent to synthesize a structured hypothesis (question, outcomes, context) in the same format the AI Analysis produces
4. Writes the result directly to the Hedgehog's SQLite database (`ai_inferences` table) with `provider: "dexter:<provider>"`
5. Signals the Hedgehog via an OSC title-set escape sequence (`\x1b]0;51folds:ready:<id>\x07`)

The command accepts an optional directional argument: `/51folds` synthesizes from the full conversation; `/51folds focus on the bear case around margins` steers the hypothesis angle.

### 2. Shared database via environment variable

The Hedgehog passes `HEDGEHOG_DB_PATH` (absolute path to `data/regime_shift_dashboard.sqlite3`) as an environment variable when spawning the Dexter subprocess. Dexter opens this database with `bun:sqlite` for the insert. No schema changes were needed — Dexter writes to the existing `ai_inferences` table.

### 3. Event-based notification via OSC terminal title

Rather than polling the database or watching files, the integration uses the existing PTY event channel. When Dexter writes the hypothesis to the DB, it emits a standard OSC 0 "set window title" escape sequence containing the inference row ID. This is invisible to the user — it's a terminal control sequence intercepted by the alacritty terminal emulator before it reaches the display.

The `poll_research_terminal` method in the Hedgehog already drains all PTY events; it was ignoring `Event::Title` with a `_ => {}` catch-all. Adding a match arm for `Title("51folds:ready:<id>")` completes the event loop — zero new infrastructure, no polling, instant notification.

### 4. Unified hypothesis pipeline

`load_dexter_inference(id)` loads the row from SQLite and calls the existing `load_historical_inference()` — the same function used when clicking a sidebar history entry. This means the hypothesis populates the same editor, the same 51Folds section, and the same model-creation flow regardless of whether it came from AI Analysis or Dexter research.

The sidebar panel header is source-aware: it shows "Research Analysis" when the current inference has a `dexter:*` provider, and "AI Analysis" otherwise. The `ai_response_provider` field tracks this.

### 5. Slash command autocomplete with argument support

The existing Dexter slash command system (`CustomEditor` in `custom-editor.ts`) intercepts enter when suggestions are showing. For `/51folds`, tab autocomplete fills the command name and leaves the cursor in the editor with a trailing space, so the user can type their directional prompt. Once they type past the command name, `matchCommands` returns no matches, `slashActive` goes false, and enter submits through the normal `handleSubmit` path — which preserves the full text including the argument.

This required a small change to `custom-editor.ts`: `slashActive` is now set by the `onSlashChange` callback (based on whether suggestions exist) rather than unconditionally when text starts with `/`.

### 6. Summary Report moved to inline central panel view

The Summary Report is now `CentralView::Report` — a full central panel view alongside Charts, 51Folds, and Research Agent. The Report button in the toolbar toggles this view rather than opening a dialog window.

Benefits:
- No z-ordering conflicts with the terminal or DAG webview
- Larger rendering area with better typography (centered 720px column, 13-14pt fonts)
- Inference list has word-boundary truncation with ellipsis, hover highlight, pointing-hand cursor, full-row click targets, and tooltips for truncated text
- Consistent with the central panel architecture

The old `egui::Window` dialog was removed entirely.

### 7. Sidebar history paging

The inference history in the left sidebar now loads up to 100 items and pages 10 at a time with prev/next navigation. Previously it showed the most recent 20 with no paging, which crowded the sidebar.

### 8. Background model build indicator

When a 51Folds model is building in the background but the user is viewing a different (loaded) model, the 51Folds sidebar section shows a compact "New model building... [Show]" bar. Clicking "Show" clears the loaded model and reveals the build-in-progress spinner.

### 9. Model recovery for timed-out builds

When loading a historical inference whose linked model timed out locally (`undisclosed_failure` status, no `response_json`), the app now falls back to `load_folds_model_id_for_inference` to find the model ID, sets it on `folds_task`, and auto-triggers `start_folds_refresh()`. If the model completed on the 51Folds server after the app stopped polling, it recovers automatically.

## Consequences

- Dexter research flows seamlessly into the 51Folds modelling pipeline with `/51folds`
- The database is the single source of truth — both Rust and TypeScript write to the same `ai_inferences` table
- The OSC title approach is fragile in the sense that it depends on `egui_term` forwarding `Event::Title` — if the terminal emulator changes, the signal could break. But it uses standard terminal protocol and requires zero new dependencies.
- The inline Report view means one fewer floating window to manage, but the Report button now changes the central view (users who expect a dialog may need to adjust)
- The `/51folds` command modifies vendored Dexter code — future upstream pulls will need to merge these changes
