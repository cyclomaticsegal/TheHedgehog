# ADR 0023: 51Folds Refresh — Distinguish Server-Confirmed Failure, Wire Retry

**Date:** 2026-04-17
**Status:** Accepted
**Extends:** ADR 0008 (51Folds integration), ADR 0013 (SDK integration, model explorer), ADR 0020 (concurrent builds, foreground/backlog)

## Context

`refresh_model` (added for Loaded Inferences → Reload from 51Folds) had a single failure mode: if the SDK returned a model whose outcomes or drivers were empty, the UI banner read *"51Folds returned an incomplete model. Try again in a moment."* That message made sense when the reason for emptiness was "server is still building." It made no sense — and offered no recovery — when the reason was "server-side build failed and is never going to produce outcomes."

The concrete failure: the user picked a failed inference from the Loaded Inferences list, clicked **Reload from 51Folds**, got the "try again in a moment" stub, waited, clicked again, got the same stub. The model's `folds_models` row still read `pending` because nothing downstream had told it otherwise. The only way out was to rebuild the hypothesis from scratch, which lost the original inference thread and any downstream chart analysis tied to it.

The 51Folds SDK exposes two signals we weren't using:

- `ModelResponse::is_failed()` — returns true when the server's `status` field is `"failed"`. A refresh that lands on a failed build can be detected, not inferred from emptiness.
- `client.models().retry(&model_id)` — POSTs `/api/v1/models/{id}/retry`, re-enqueues the build server-side, and returns. The same `model_id` then re-enters the normal async lifecycle; polling it via `wait_until_complete` works exactly as it does for a fresh `create`.

Neither was wired in. The app had no way to tell the user "this one failed, but you can retry," and no way to actually retry.

## Decisions

### 1. `refresh_model` inspects `is_failed()` before the emptiness heuristic

```rs
if model.is_failed() {
    storage::update_folds_model_status_standalone(
        &db_path, &model_id, FOLDS_STATUS_FAIL, Some(Utc::now()),
    );
    let _ = tx.send(FoldsResult::RefreshFoundFailed { model_id });
    return;
}
if model.current.outcomes.is_empty() || model.drivers.is_empty() {
    // existing "try again in a moment" path
}
```

Server-confirmed failure takes precedence over emptiness. A failed model is *expected* to have empty outcomes/drivers, so the old path was firing on the right symptom for the wrong reason. Persisting `FOLDS_STATUS_FAIL` to the DB on this branch keeps the sidebar badge, Report empty-state, and resume sweep all in sync with what the server thinks.

### 2. New IPC variant `FoldsResult::RefreshFoundFailed { model_id }`

Distinct from `RefreshFailed(String)` (network/parse error — transient, retry the refresh itself) and `Failed(String)` (the original build-time catastrophic failure). `RefreshFoundFailed` carries the `model_id` so the UI can offer a one-click retry without re-deriving it from the DB.

The variant is handled in `poll_refresh` (clears the spinner, sets `refresh_found_failed_id`) and ignored in every other poll arm (`poll_main`, `FoldsBacklog::poll_pending_creates`, `FoldsBacklog::poll_background`) — refresh is a foreground-only operation, the backlog never originates it.

### 3. New `retry_build` function mirrors `create_and_poll`

```rs
pub fn retry_build(api_key, model_id, db_path, tx: Sender<FoldsResult>) {
    // POST /api/v1/models/{id}/retry
    client.models().retry(&model_id).await?;
    // flip DB row pending so resume-sweep would pick it up after restart
    storage::update_folds_model_status_standalone(
        &db_path, &model_id, FOLDS_STATUS_PENDING, None,
    );
    tx.send(FoldsResult::Created(model_id.clone()));
    // poll via wait_until_complete, same terminal-state branching as create_and_poll
}
```

The terminal-state branches are copied deliberately: `ModelBuildFailed` → `FOLDS_STATUS_FAIL`, `PollTimeout` → `FOLDS_STATUS_UNDISCLOSED_FAILURE`, `Ok` → `persist_completed`. A retry is structurally a rerun of the same build lifecycle, so it earns the same crash-recoverability guarantees as the original.

### 4. Report empty-state surfaces a Retry button

Inside the Report view's "model data is incomplete" branch (`src/app.rs:3988`), `self.folds_task.refresh_found_failed_id.is_some()` flips the copy from *"Reload from 51Folds"* to *"Build failed on 51Folds — Retry build"* and routes the button click to `start_folds_retry` rather than `start_folds_refresh`. The previous "refresh returned a stub" banner path is preserved for the genuinely-transient case.

### 5. `start_folds_retry` reuses the foreground slot via park-to-backlog

Rather than invent a new UI surface for retries, a retry routes through the normal foreground `FoldsTask` using the ADR 0020 park-before-reset pattern. If another build is already in the foreground it gets pushed to the backlog via `std::mem::replace`, then the retry takes the foreground slot with its fresh channel. The tray chip, elapsed-time display, Created/Completed/Failed event handling, and live sidebar badge updates all continue working with zero new code.

One subtlety the first pass got wrong: `load_historical_inference` populates `draft_hypothesis` but not `folds_task.question`. A retry issued immediately after loading a failed inference from the Loaded Inferences list therefore captured `question = None`, and the tray chip read `Kd — (untitled)`. The fix falls back to the draft hypothesis and to `last_inference_id`:

```rs
let question_label = self.folds_task.question.clone()
    .or_else(|| self.draft_hypothesis.as_ref().map(|h| h.question.clone()));
let inference_id = self.folds_task.inference_id.or(self.last_inference_id);
```

## Consequences

- Users can recover from server-confirmed failed builds without losing the inference thread. A single click reruns the build under the same `model_id`; the downstream inference rows and chart analyses stay linked.
- The Loaded Inferences → Reload path now surfaces accurate server state. Genuine transience still shows "Try again in a moment"; genuine failure shows "Retry build." The ambiguous middle has been eliminated.
- Reusing the foreground `FoldsTask` means retries get the tray, elapsed timer, and badge updates for free. The cost is one more `std::mem::replace` call site, which is fine — ADR 0020 already established this as the pattern.
- `FoldsResult::RefreshFoundFailed` must be added as an arm to every poll match. Four sites got updated (`poll_refresh`, `poll_main`, `poll_pending_creates`, `poll_background`); future poll sites need to remember it. Rustc exhaustiveness checking catches omissions at compile time, so the maintenance load is bounded.
- `FOLDS_STATUS_FAIL` is now written on two more paths (refresh-finds-failed, retry-build-fails) that previously left the row as `pending`. Status drift between the DB and server shrinks as a result, which matters for the resume-on-restart sweep.
- The retry itself is a network call with no local-only dry run. A user who retries an already-successful model by mistake would get whatever the server does in that case — currently an error, possibly a fresh build in the future. The UI only offers Retry on the `RefreshFoundFailed` path, so accidental retries of successful models are not a practical concern.
