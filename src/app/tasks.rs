//! Background-task state structs used by `DashboardApp`. These are the
//! in-memory shapes that wrap the mpsc receivers for long-running work
//! (LLM inference, 51Folds model builds, the research-agent terminal,
//! and the startup splash timer). All fields are `pub(super)` so
//! `DashboardApp` in the parent module can read/write them directly
//! without accessor ceremony.
//!
//! Pure state + poll helpers — no egui, no rendering, no `DashboardApp`
//! coupling.
//!
//! See ADR 0020 for the foreground/backlog split implemented by
//! `FoldsTask` + `FoldsBacklog`.
use crate::folds::FoldsResult;
use crate::models::{AiEvent, AiInferenceResult};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};

// ---------------------------------------------------------------------------
// LLM task — AI inference over the VIX time series.
// ---------------------------------------------------------------------------

pub(super) struct LlmTask {
    pub(super) in_flight: bool,
    pub(super) rx: Option<Receiver<AiEvent>>,
    pub(super) error: Option<String>,
}

pub(super) enum LlmPoll {
    Response(AiInferenceResult),
    Failed,
    Pending,
    Idle,
}

impl LlmTask {
    pub(super) fn new() -> Self {
        Self { in_flight: false, rx: None, error: None }
    }

    pub(super) fn start(&mut self, rx: Receiver<AiEvent>) {
        self.in_flight = true;
        self.rx = Some(rx);
        self.error = None;
    }

    pub(super) fn poll(&mut self) -> LlmPoll {
        if !self.in_flight {
            return LlmPoll::Idle;
        }
        let Some(rx) = self.rx.take() else {
            return LlmPoll::Idle;
        };
        match rx.try_recv() {
            Ok(AiEvent::Response(r)) => {
                self.in_flight = false;
                LlmPoll::Response(r)
            }
            Ok(AiEvent::Failed(e)) => {
                self.in_flight = false;
                self.error = Some(e);
                LlmPoll::Failed
            }
            Err(TryRecvError::Empty) => {
                self.rx = Some(rx);
                LlmPoll::Pending
            }
            Err(TryRecvError::Disconnected) => {
                self.in_flight = false;
                self.error = Some("Analysis thread disconnected unexpectedly.".to_owned());
                LlmPoll::Failed
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 51Folds build task — foreground + backlog.
// ---------------------------------------------------------------------------

/// Mutable copy of a driver's state for the re-evaluate UI. The user
/// changes `selected_state` via the segmented selector; on "Re-evaluate"
/// we diff against `original_state` to build the patch request.
pub(super) struct DraftDriverState {
    pub(super) code: String,
    pub(super) name: String,
    pub(super) selected_state: String,
    pub(super) original_state: String,
    /// (state_name, description) pairs from the model's state_descriptors.
    pub(super) state_options: Vec<(String, String)>,
    pub(super) expanded: bool,
}

pub(super) struct FoldsTask {
    pub(super) in_flight: bool,
    pub(super) rx: Option<Receiver<FoldsResult>>,
    pub(super) model_id: Option<String>,
    pub(super) error: Option<String>,
    /// Full model response from the SDK, set when `Completed` arrives.
    pub(super) model: Option<Box<fiftyone_folds::ModelResponse>>,
    /// User-mutable driver states for the re-evaluate flow.
    pub(super) draft_drivers: Vec<DraftDriverState>,
    /// Snapshot of outcome probabilities BEFORE a re-evaluate, for
    /// rendering before/after deltas.
    pub(super) previous_outcomes: Option<Vec<(String, f64)>>,
    /// True when a driver re-evaluate (not initial creation) is in flight.
    pub(super) reevaluating: bool,
    /// True while a Refresh-Model call is in flight. Separate from
    /// `in_flight` so the Refresh affordance can run concurrently with
    /// (or after) a re-eval without clobbering its state.
    pub(super) refresh_in_flight: bool,
    pub(super) refresh_rx: Option<Receiver<FoldsResult>>,
    pub(super) refresh_error: Option<String>,
    /// Set when a Refresh reaches the server and the server reports the
    /// model is in the "Failed" state. Holds the `model_id` so the UI
    /// can offer a "Retry build" button that reuses it.
    pub(super) refresh_found_failed_id: Option<String>,
    /// Short hypothesis label for the tray / backlog popover. Set when a
    /// build is spawned or resumed; never used for anything other than
    /// display.
    pub(super) question: Option<String>,
    /// When the build was spawned (or resumed). Drives the tray elapsed
    /// time readout.
    pub(super) started_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Originating inference row. Used to update the sidebar/Report
    /// status badge map live as builds complete.
    pub(super) inference_id: Option<i64>,
}

impl FoldsTask {
    pub(super) fn new() -> Self {
        Self {
            in_flight: false,
            rx: None,
            model_id: None,
            error: None,
            model: None,
            draft_drivers: Vec::new(),
            previous_outcomes: None,
            reevaluating: false,
            refresh_in_flight: false,
            refresh_rx: None,
            refresh_error: None,
            refresh_found_failed_id: None,
            question: None,
            started_at: None,
            inference_id: None,
        }
    }

    pub(super) fn reset(&mut self) {
        self.in_flight = false;
        self.rx = None;
        self.model_id = None;
        self.error = None;
        self.model = None;
        self.draft_drivers.clear();
        self.previous_outcomes = None;
        self.reevaluating = false;
        self.refresh_in_flight = false;
        self.refresh_rx = None;
        self.refresh_error = None;
        self.refresh_found_failed_id = None;
        self.question = None;
        self.started_at = None;
        self.inference_id = None;
    }

    pub(super) fn start(&mut self, rx: Receiver<FoldsResult>) {
        self.reset();
        self.in_flight = true;
        self.rx = Some(rx);
    }

    /// Initialize draft driver states from the completed model response.
    /// Joins `model.drivers` (definitions) with `model.current.drivers`
    /// (current states) by code.
    pub(super) fn init_draft_drivers(&mut self) {
        let Some(ref model) = self.model else { return };
        self.draft_drivers = model
            .drivers
            .iter()
            .map(|def| {
                let raw_current_state = model
                    .current
                    .drivers
                    .iter()
                    .find(|ds| ds.code == def.code)
                    .map(|ds| ds.state.clone())
                    .unwrap_or_default();

                let state_options: Vec<(String, String)> = def
                    .state_descriptors
                    .iter()
                    .map(|sd| (sd.name.clone(), sd.description.clone()))
                    .collect();

                // See the long comment in the previous inline implementation:
                // two-step normalization (case-insensitive name match, then
                // ordinal fallback via the canonical Bayesian schema) lets
                // us match a server-reported "Negligible" even when the
                // LLM-generated descriptor is "Negligent".
                const CANONICAL_STATES: &[&str] =
                    &["Negligible", "Low", "Medium", "High", "Extreme"];
                let by_name = state_options
                    .iter()
                    .map(|(name, _)| name)
                    .find(|name| name.eq_ignore_ascii_case(&raw_current_state))
                    .cloned();
                let normalized_current = by_name.unwrap_or_else(|| {
                    if let Some(canonical_idx) = CANONICAL_STATES
                        .iter()
                        .position(|c| c.eq_ignore_ascii_case(&raw_current_state))
                        && let Some((descriptor_name, _)) =
                            state_options.get(canonical_idx)
                    {
                        return descriptor_name.clone();
                    }
                    raw_current_state
                });

                DraftDriverState {
                    code: def.code.clone(),
                    name: def.name.clone(),
                    selected_state: normalized_current.clone(),
                    original_state: normalized_current,
                    state_options,
                    expanded: false,
                }
            })
            .collect();
    }

    /// True when the model has completed successfully.
    pub(super) fn is_complete(&self) -> bool {
        self.model
            .as_ref()
            .is_some_and(|m| m.is_complete())
    }

    pub(super) fn poll(&mut self) {
        self.poll_main();
        self.poll_refresh();
    }

    fn poll_main(&mut self) {
        let Some(rx) = self.rx.take() else { return };
        loop {
            match rx.try_recv() {
                Ok(FoldsResult::Created(id)) => {
                    self.model_id = Some(id);
                }
                Ok(FoldsResult::Completed(model)) => {
                    self.model_id = Some(model.model_id.clone());
                    self.model = Some(model);
                    self.in_flight = false;
                    self.reevaluating = false;
                    self.init_draft_drivers();
                    return;
                }
                Ok(FoldsResult::Failed(e)) => {
                    self.error = Some(e);
                    self.in_flight = false;
                    self.reevaluating = false;
                    return;
                }
                Ok(FoldsResult::Refreshed(_))
                | Ok(FoldsResult::RefreshFailed(_))
                | Ok(FoldsResult::RefreshFoundFailed { .. }) => {}
                Err(TryRecvError::Empty) => {
                    self.rx = Some(rx);
                    return;
                }
                Err(TryRecvError::Disconnected) => {
                    if !self.is_complete() {
                        self.error = Some("51Folds task disconnected unexpectedly.".to_owned());
                    }
                    self.in_flight = false;
                    return;
                }
            }
        }
    }

    fn poll_refresh(&mut self) {
        let Some(rx) = self.refresh_rx.take() else { return };
        loop {
            match rx.try_recv() {
                Ok(FoldsResult::Refreshed(model)) => {
                    self.model_id = Some(model.model_id.clone());
                    self.model = Some(model);
                    self.refresh_in_flight = false;
                    self.refresh_error = None;
                    self.error = None;
                    self.init_draft_drivers();
                    return;
                }
                Ok(FoldsResult::RefreshFailed(e)) => {
                    self.refresh_in_flight = false;
                    self.refresh_error = Some(e);
                    return;
                }
                Ok(FoldsResult::RefreshFoundFailed { model_id }) => {
                    self.refresh_in_flight = false;
                    self.refresh_error = None;
                    self.refresh_found_failed_id = Some(model_id);
                    return;
                }
                Ok(_) => {}
                Err(TryRecvError::Empty) => {
                    self.refresh_rx = Some(rx);
                    return;
                }
                Err(TryRecvError::Disconnected) => {
                    self.refresh_in_flight = false;
                    return;
                }
            }
        }
    }

    /// Load a completed model response from a JSON blob (e.g. from the
    /// database). On success, initializes draft drivers. If the blob is a
    /// stub (empty outcomes / empty drivers from earlier buggy writes),
    /// keeps the `model_id` so the UI can still offer Refresh, but
    /// leaves `model` as `None`.
    pub(super) fn load_from_json(&mut self, json: &str) {
        match serde_json::from_str::<fiftyone_folds::ModelResponse>(json) {
            Ok(model) => {
                let is_stub =
                    model.current.outcomes.is_empty() || model.drivers.is_empty();
                if is_stub {
                    eprintln!(
                        "[folds] load_from_json: stub detected for model_id={:?} \
                         — keeping model_id for recovery, leaving model=None",
                        model.model_id,
                    );
                    if !model.model_id.is_empty() {
                        self.model_id = Some(model.model_id);
                    }
                    self.model = None;
                    self.draft_drivers.clear();
                    return;
                }
                self.model_id = Some(model.model_id.clone());
                self.model = Some(Box::new(model));
                self.init_draft_drivers();
            }
            Err(e) => {
                eprintln!("warn: failed to deserialize stored model response: {e}");
            }
        }
    }
}

/// Holds 51Folds builds that aren't currently in the foreground. See ADR
/// 0020 for the foreground/backlog split. `background` is keyed by model
/// ID once the server has returned one; `pending_creates` holds builds
/// in the brief window between a Create click and the first
/// `FoldsResult::Created` event.
pub(super) struct FoldsBacklog {
    pub(super) background: HashMap<String, FoldsTask>,
    pub(super) pending_creates: Vec<FoldsTask>,
}

impl FoldsBacklog {
    pub(super) fn new() -> Self {
        Self {
            background: HashMap::new(),
            pending_creates: Vec::new(),
        }
    }

    /// Count of builds that are still working. Foreground is counted
    /// separately by callers — this function only reports the backlog.
    pub(super) fn active_count(&self) -> usize {
        self.background
            .values()
            .filter(|t| t.in_flight)
            .count()
            + self.pending_creates.iter().filter(|t| t.in_flight).count()
    }

    /// Park a FoldsTask into the backlog. If it has a model_id, it lives
    /// in `background`; otherwise it waits in `pending_creates` until
    /// its model_id arrives via the Created event.
    pub(super) fn park(&mut self, task: FoldsTask) {
        let has_rx = task.rx.is_some();
        let in_flight = task.in_flight;
        if let Some(id) = task.model_id.clone() {
            tracing::info!(
                model_id = %id,
                in_flight,
                has_rx,
                inference_id = ?task.inference_id,
                "folds_backlog: park to background"
            );
            self.background.insert(id, task);
        } else {
            tracing::info!(
                in_flight,
                has_rx,
                inference_id = ?task.inference_id,
                "folds_backlog: park to pending_creates (no model_id yet)"
            );
            self.pending_creates.push(task);
        }
    }

    /// Take a parked task back into the foreground by model ID.
    pub(super) fn take(&mut self, model_id: &str) -> Option<FoldsTask> {
        self.background.remove(model_id)
    }

    /// Drain terminal events from every backlog channel.
    ///
    /// Completed/Failed builds stay in `background` with `in_flight =
    /// false` so the tray and sidebar can reflect them. Pending-creates
    /// promote into `background` the moment they receive a model ID
    /// (via either `Created` or `Completed`).
    ///
    /// Returns the list of inference IDs whose state changed, so the
    /// caller can refresh the sidebar/Report status badge map live.
    pub(super) fn poll_all(&mut self) -> Vec<i64> {
        let mut touched: Vec<i64> = Vec::new();

        let parked = std::mem::take(&mut self.pending_creates);
        for mut task in parked {
            let mut promote_id: Option<String> = None;
            if let Some(rx) = task.rx.take() {
                loop {
                    match rx.try_recv() {
                        Ok(FoldsResult::Created(id)) => {
                            task.model_id = Some(id.clone());
                            if let Some(iid) = task.inference_id {
                                touched.push(iid);
                            }
                            promote_id = Some(id);
                            continue;
                        }
                        Ok(FoldsResult::Completed(model)) => {
                            let id = model.model_id.clone();
                            task.in_flight = false;
                            task.model_id = Some(id.clone());
                            task.model = Some(model);
                            if let Some(iid) = task.inference_id {
                                touched.push(iid);
                            }
                            promote_id = Some(id);
                            break;
                        }
                        Ok(FoldsResult::Failed(e)) => {
                            task.in_flight = false;
                            task.error = Some(e);
                            if let Some(iid) = task.inference_id {
                                touched.push(iid);
                            }
                            break;
                        }
                        Ok(FoldsResult::Refreshed(_))
                        | Ok(FoldsResult::RefreshFailed(_))
                        | Ok(FoldsResult::RefreshFoundFailed { .. }) => continue,
                        Err(TryRecvError::Empty) => {
                            task.rx = Some(rx);
                            break;
                        }
                        Err(TryRecvError::Disconnected) => {
                            tracing::warn!(
                                inference_id = ?task.inference_id,
                                "folds_backlog: pending-create channel Disconnected \
                                 — in_flight flipped false without terminal event"
                            );
                            task.in_flight = false;
                            break;
                        }
                    }
                }
            }
            match promote_id {
                Some(id) => {
                    self.background.insert(id, task);
                }
                None => {
                    self.pending_creates.push(task);
                }
            }
        }

        for task in self.background.values_mut() {
            let Some(rx) = task.rx.take() else { continue };
            loop {
                match rx.try_recv() {
                    Ok(FoldsResult::Created(_)) => continue,
                    Ok(FoldsResult::Completed(model)) => {
                        task.in_flight = false;
                        task.model_id = Some(model.model_id.clone());
                        task.model = Some(model);
                        if let Some(iid) = task.inference_id {
                            touched.push(iid);
                        }
                        break;
                    }
                    Ok(FoldsResult::Failed(e)) => {
                        task.in_flight = false;
                        task.error = Some(e);
                        if let Some(iid) = task.inference_id {
                            touched.push(iid);
                        }
                        break;
                    }
                    Ok(FoldsResult::Refreshed(_))
                    | Ok(FoldsResult::RefreshFailed(_))
                    | Ok(FoldsResult::RefreshFoundFailed { .. }) => continue,
                    Err(TryRecvError::Empty) => {
                        task.rx = Some(rx);
                        break;
                    }
                    Err(TryRecvError::Disconnected) => {
                        tracing::warn!(
                            model_id = ?task.model_id,
                            inference_id = ?task.inference_id,
                            "folds_backlog: background channel Disconnected \
                             — in_flight flipped false without terminal event"
                        );
                        task.in_flight = false;
                        break;
                    }
                }
            }
        }

        touched
    }
}

/// One row in the toolbar tray popover. Built from a `FoldsTask`
/// snapshot without borrowing across the popup closure.
pub(super) struct TrayRow {
    pub(super) question: Option<String>,
    pub(super) model_id: Option<String>,
    pub(super) started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub(super) is_foreground: bool,
}

impl TrayRow {
    pub(super) fn from_task(task: &FoldsTask, is_foreground: bool) -> Self {
        Self {
            question: task.question.clone(),
            model_id: task.model_id.clone(),
            started_at: task.started_at,
            is_foreground,
        }
    }

    pub(super) fn elapsed_label(&self) -> String {
        let Some(started) = self.started_at else {
            return "just now".to_owned();
        };
        let elapsed_secs = (chrono::Utc::now() - started).num_seconds().max(0);
        if elapsed_secs < 60 {
            format!("{elapsed_secs}s")
        } else if elapsed_secs < 3600 {
            format!("{}m {}s", elapsed_secs / 60, elapsed_secs % 60)
        } else {
            format!("{}h {}m", elapsed_secs / 3600, (elapsed_secs % 3600) / 60)
        }
    }
}

// ---------------------------------------------------------------------------
// Embedded Dexter research-agent terminal.
// ---------------------------------------------------------------------------

/// State for the embedded Dexter research-agent terminal.
pub(super) struct ResearchTerminal {
    pub(super) backend: Option<egui_term::TerminalBackend>,
    pub(super) pty_rx: Option<std::sync::mpsc::Receiver<(u64, egui_term::PtyEvent)>>,
    /// Whether `bun` was found in PATH. `None` = not yet checked.
    pub(super) bun_available: Option<bool>,
    /// If the child process exited, stores the exit code.
    pub(super) child_exited: Option<i32>,
    /// Human-readable error (backend creation failure, missing key, etc.).
    pub(super) error: Option<String>,
    /// Monotonically increasing ID for terminal backend instances.
    pub(super) next_id: u64,
}

impl ResearchTerminal {
    pub(super) fn new() -> Self {
        Self {
            backend: None,
            pty_rx: None,
            bun_available: None,
            child_exited: None,
            error: None,
            next_id: 1,
        }
    }
}

// ---------------------------------------------------------------------------
// Startup splash timer.
// ---------------------------------------------------------------------------

/// Startup-splash overlay state. The actual mascot texture lives on
/// `DashboardApp::mascot_texture` so it can be shared with the Help
/// window; this struct only tracks the display timer and whether the
/// splash observed the startup auto-refresh.
pub(super) struct SplashState {
    /// When the splash became visible. `None` once dismissed.
    pub(super) shown_at: Option<std::time::Instant>,
    /// First frame we observed `refresh_in_flight == true` while the
    /// splash was visible. Used to detect "loading mode" and to know
    /// whether we need the post-load extension.
    pub(super) loading_start: Option<std::time::Instant>,
    /// First frame after `loading_start` at which `refresh_in_flight`
    /// flipped back to `false`. Used to record that we've already
    /// applied the post-load hold extension so it doesn't re-trigger.
    pub(super) loading_end: Option<std::time::Instant>,
}

impl SplashState {
    pub(super) fn new() -> Self {
        Self {
            shown_at: Some(std::time::Instant::now()),
            loading_start: None,
            loading_end: None,
        }
    }

    pub(super) fn is_active(&self) -> bool {
        self.shown_at.is_some()
    }

    pub(super) fn elapsed(&self) -> std::time::Duration {
        self.shown_at
            .map(|t| t.elapsed())
            .unwrap_or_default()
    }

    pub(super) fn dismiss(&mut self) {
        self.shown_at = None;
    }
}
