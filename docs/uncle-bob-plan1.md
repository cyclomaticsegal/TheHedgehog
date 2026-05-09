# uncle-bob-plan1: Clean Code Fixes

## Context

Code review against SOLID/DRY/KISS/YAGNI identified 6 concrete problems in the Hedgehog codebase.
None require architectural overhaul — all are bounded, targeted fixes.
The `DashboardApp` God Object problem (35 fields, 700-line render method) is flagged but deferred:
splitting it cleanly requires understanding the full egui render lifecycle and is out of scope here.

---

## Fix 1 — Hoist `MAX_LOG_ENTRIES` to a module-level constant

**Problem:** The same literal `500` with the same comment appears at `src/app.rs:447` and `src/app.rs:558`.

**Fix:** Add one `const MAX_LOG_ENTRIES: usize = 500;` near the top of `app.rs` (alongside the palette constants) and remove both inline definitions.

**Files:** `src/app.rs`

---

## Fix 2 — Deduplicate the user-agent string

**Problem:** `"the-hedgehog/0.2.0"` appears as a string literal in both `src/ai.rs:50` and `src/providers.rs:20`.

**Fix:** Add `pub(crate) const USER_AGENT: &str = "the-hedgehog/0.2.0";` to `src/main.rs` and replace both literals with `crate::USER_AGENT`.

**Files:** `src/main.rs`, `src/ai.rs`, `src/providers.rs`

---

## Fix 3 — Remove pre-formatted strings from `LogEntry`

**Problem:** `LogEntry` stores `instrument_str: String` and `status_str: String` alongside the source values they are derived from. This is a DRY violation with a latent consistency risk. The `instrument_str` field is also used as a lookup key in `update_log_entry`, which is fragile.

**Fix:**
1. Remove `instrument_str` and `status_str` from the `LogEntry` struct.
2. In `update_log_entry`, change the lookup from `e.instrument_str == instrument_str` to `e.instrument == instrument && matches!(e.status, LogStatus::Fetching)`.
3. In the render code (inside `show_dashboard`), call `format!("{:<12}", entry.instrument.as_str())` and `format_log_status(&entry.status)` inline when rendering the log panel.

**Files:** `src/app.rs`

---

## Fix 4 — Remove the derived `ai_model` field from `AppSettings`

**Problem:** `AppSettings` has three related fields: `ai_model` (a runtime-sync'd copy), `ai_model_anthropic`, and `ai_model_openai`. The copy is derived from one of the source fields based on `ai_provider`. Storing a derived value alongside its sources is a consistency trap — the manual sync in `DashboardApp::new()` (line 264) and everywhere `ai_model` is written is error-prone.

**Fix:**
1. Delete the `ai_model: String` field from `AppSettings`.
2. Add an `#[serde(skip)]` note in the Default impl, or just remove the field (serde will silently skip missing fields due to `#[serde(default)]` on the struct).
3. Add a method to `AppSettings`:
   ```rust
   pub fn effective_model(&self) -> &str {
       match self.ai_provider {
           LlmProvider::Anthropic => &self.ai_model_anthropic,
           LlmProvider::OpenAI => &self.ai_model_openai,
       }
   }
   ```
4. Replace all reads of `self.settings.ai_model` with `self.settings.effective_model()`.
5. Replace all writes to `self.settings.ai_model` (settings UI inputs) with writes to the appropriate per-provider field (`ai_model_anthropic` or `ai_model_openai`).
6. Remove the sync code in `DashboardApp::new()` at lines 264–267.

**Files:** `src/models.rs`, `src/app.rs`

---

## Fix 5 — Make `evaluate_alert_transition` use the analysis cache

**Problem:** `evaluate_alert_transition()` calls `self.current_vix_status()` which always recomputes from scratch, bypassing the `cached_vix_status` that `refresh_analysis_cache()` maintains. This means two code paths can produce different answers for the same data.

**Fix:**
1. In `poll_refresh()`, after `self.reload_from_storage()` is called (line 478), also call `self.refresh_analysis_cache()` so the cache is fresh before alert evaluation.
2. Rewrite `evaluate_alert_transition()` to use `self.cached_vix_status.as_ref()` instead of `self.current_vix_status()`.
3. Delete the `current_vix_status()` method — it now has no callers.

**Files:** `src/app.rs`

---

## Fix 6 — Extract `LlmTask` to DRY the poll/start pattern

**Problem:** `poll_ai()` and `poll_report()` are structurally identical (channel take → try_recv → 4 match arms). `start_ai_analysis()` and `start_report_generation()` share the same pre-flight check + channel create + thread spawn boilerplate. The only divergence is in the success handler.

**Fix:** Add a small `LlmTask` struct inside `app.rs` (not a new file — YAGNI) to own the shared state and expose a `poll()` method that returns an enum the caller handles.

```rust
struct LlmTask {
    in_flight: bool,
    rx: Option<Receiver<AiEvent>>,
    error: Option<String>,
}

enum LlmPoll {
    Response(AiInferenceResult),
    Failed(String),
    Pending,
    Idle,
}

impl LlmTask {
    fn new() -> Self { Self { in_flight: false, rx: None, error: None } }

    fn start(&mut self, rx: Receiver<AiEvent>) {
        self.in_flight = true;
        self.rx = Some(rx);
        self.error = None;
    }

    fn poll(&mut self) -> LlmPoll {
        if !self.in_flight { return LlmPoll::Idle; }
        let Some(rx) = self.rx.take() else { return LlmPoll::Idle; };
        match rx.try_recv() {
            Ok(AiEvent::Response(r)) => { self.in_flight = false; LlmPoll::Response(r) }
            Ok(AiEvent::Failed(e))   => { self.in_flight = false; self.error = Some(e.clone()); LlmPoll::Failed(e) }
            Err(TryRecvError::Empty) => { self.rx = Some(rx); LlmPoll::Pending }
            Err(TryRecvError::Disconnected) => {
                self.in_flight = false;
                let e = "Analysis thread disconnected unexpectedly.".to_owned();
                self.error = Some(e.clone());
                LlmPoll::Failed(e)
            }
        }
    }
}
```

Then in `DashboardApp`:
- Replace `ai_in_flight`, `ai_rx`, `ai_error` with `ai_task: LlmTask`
- Replace `report_in_flight`, `report_rx`, `report_error` with `report_task: LlmTask`
- `poll_ai()` becomes: `match self.ai_task.poll() { LlmPoll::Response(r) => { /* persist, set ai_response */ } ... }`
- `poll_report()` becomes: `match self.report_task.poll() { ... }`
- Both `start_*` methods call `self.ai_task.start(rx)` / `self.report_task.start(rx)` instead of setting 3 fields each

**Files:** `src/app.rs`

---

## Execution Order

1. Fix 1 (const) — warmup, 2 min
2. Fix 2 (user-agent) — trivial, 2 min
3. Fix 3 (LogEntry) — moderate, touches struct + render
4. Fix 4 (ai_model) — moderate, touches models + multiple app.rs call sites
5. Fix 5 (cache) — small but requires understanding poll_refresh flow
6. Fix 6 (LlmTask) — largest; do last to minimize merge conflicts with Fix 4

## Verification

After all fixes:
- `cargo build` must succeed with no warnings
- `cargo clippy` should produce no new warnings
- The app should launch, auto-refresh market data, and run an AI analysis successfully
- Saving API keys and reloading the app should preserve settings (regression test for Fix 4)
