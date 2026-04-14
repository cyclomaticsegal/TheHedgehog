use crate::ai;
use crate::analysis;
use crate::eval;
use crate::help;
use crate::knowledge;
use crate::models::{
    AiEvent, AiInferenceResult, AiPanelDock, AlertEvent, AlertLevel, ApiKeys, AppSettings,
    AssetGroup, ChartWindow, Instrument, LlmProvider, Observation,
    ParsedHypothesis, RefreshEvent, SavedInference, ThresholdConfig, ThresholdMode,
    ThresholdSnapshot, VixStatus,
};
use crate::providers;
use crate::storage::Storage;
use chrono::Utc;
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Vec2,
};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Design-system palette
// ---------------------------------------------------------------------------
// Backgrounds
const APP_BG: Color32 = Color32::from_rgb(10, 14, 26);
const PANEL_BG: Color32 = Color32::from_rgb(17, 24, 39);
const SURFACE: Color32 = Color32::from_rgb(26, 34, 54);
const SURFACE_HOVER: Color32 = Color32::from_rgb(34, 45, 66);
// Borders
const BORDER: Color32 = Color32::from_rgb(45, 55, 72);
// Text
const TEXT_PRIMARY: Color32 = Color32::from_rgb(226, 232, 240);
const TEXT_SECONDARY: Color32 = Color32::from_rgb(148, 163, 184);
const TEXT_MUTED: Color32 = Color32::from_rgb(74, 85, 104);
// Alert levels
const ALERT_NORMAL_FG: Color32 = Color32::from_rgb(56, 161, 105);
const ALERT_APPROACHING_FG: Color32 = Color32::from_rgb(214, 158, 46);
const ALERT_EXTREME_FG: Color32 = Color32::from_rgb(229, 62, 62);
// Accent colors (51Folds model explorer)
const ACCENT_BLUE: Color32 = Color32::from_rgb(96, 165, 250);
const ACCENT_BLUE_DIM: Color32 = Color32::from_rgb(59, 130, 246);

const MAX_LOG_ENTRIES: usize = 500;

/// Which view the central panel is showing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CentralView {
    Charts,
    Model,
    ResearchAgent,
    Report,
}

/// Navigation stack within the 51Folds model explorer. Each variant is
/// a "page" in the central panel. The back button pops to the previous
/// level rather than needing explicit tab management.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModelView {
    /// Outcome probabilities + take away summary.
    Outcome,
    /// Clean list of all drivers with pill selectors.
    DriverList,
    /// Interactive DAG visualization of the causal network.
    VisualMap,
    /// Full-page detail for one driver (by index in draft_drivers).
    DriverDetail(usize),
    /// Full-page content for one driver section.
    DriverSection(usize, DriverDetailSection),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DriverDetailSection {
    WhySelected,
    WhyMatters,
    WhatShift,
    WhatMonitor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusKind {
    Info,
    Success,
    Error,
}

#[derive(Clone)]
struct LogEntry {
    timestamp_str: String,
    instrument: Instrument,
    source: String,
    status: LogStatus,
}

#[derive(Clone)]
enum LogStatus {
    Fetching,
    Ok(usize),
    Cached(String),
    Failed(String),
}

struct LlmTask {
    in_flight: bool,
    rx: Option<Receiver<AiEvent>>,
    error: Option<String>,
}

enum LlmPoll {
    Response(AiInferenceResult),
    Failed,
    Pending,
    Idle,
}

impl LlmTask {
    fn new() -> Self {
        Self { in_flight: false, rx: None, error: None }
    }

    fn start(&mut self, rx: Receiver<AiEvent>) {
        self.in_flight = true;
        self.rx = Some(rx);
        self.error = None;
    }

    fn poll(&mut self) -> LlmPoll {
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

/// Model tier we send to 51Folds for every create request. Advanced is the
/// richest tier (~25-30 min build time) and provisions the most drivers
/// with full causal analysis. We don't expose a tier selector in the UI;
/// the constant lives here so it's easy to find if we ever need to change it.
const FOLDS_MODEL_TYPE: &str = "Advanced";

use crate::folds::FoldsResult;

/// Mutable copy of a driver's state for the re-evaluate UI. The user
/// changes `selected_state` via the segmented selector; on "Re-evaluate"
/// we diff against `original_state` to build the patch request.
struct DraftDriverState {
    code: String,
    name: String,
    selected_state: String,
    original_state: String,
    /// (state_name, description) pairs from the model's state_descriptors.
    state_options: Vec<(String, String)>,
    expanded: bool,
}

struct FoldsTask {
    in_flight: bool,
    rx: Option<Receiver<FoldsResult>>,
    model_id: Option<String>,
    error: Option<String>,
    /// Full model response from the SDK, set when `Completed` arrives.
    model: Option<Box<fiftyone_folds::ModelResponse>>,
    /// User-mutable driver states for the re-evaluate flow.
    draft_drivers: Vec<DraftDriverState>,
    /// Snapshot of outcome probabilities BEFORE a re-evaluate, for
    /// rendering before/after deltas.
    previous_outcomes: Option<Vec<(String, f64)>>,
    /// True when a driver re-evaluate (not initial creation) is in flight.
    reevaluating: bool,
    /// True while a Refresh-Model call is in flight. Separate from
    /// `in_flight` so the Refresh affordance can run concurrently with
    /// (or after) a re-eval without clobbering its state.
    refresh_in_flight: bool,
    refresh_rx: Option<Receiver<FoldsResult>>,
    refresh_error: Option<String>,
}

impl FoldsTask {
    fn new() -> Self {
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
        }
    }

    fn reset(&mut self) {
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
    }

    fn start(&mut self, rx: Receiver<FoldsResult>) {
        self.reset();
        self.in_flight = true;
        self.rx = Some(rx);
    }

    /// Initialize draft driver states from the completed model response.
    /// Joins `model.drivers` (definitions) with `model.current.drivers`
    /// (current states) by code.
    fn init_draft_drivers(&mut self) {
        let Some(ref model) = self.model else { return };
        self.draft_drivers = model
            .drivers
            .iter()
            .map(|def| {
                // Raw current state from the model response (this is
                // what the server considers authoritative).
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

                // Normalize the current state to match the case/
                // whitespace of the corresponding state_descriptor
                // name. Two layers of defence:
                //
                // 1. **Case-insensitive name match** — handles the
                //    trivial case where the server returned "high"
                //    while descriptors contain "High".
                //
                // 2. **Ordinal fallback via the canonical Bayesian
                //    schema states** `[Negligible, Low, Medium, High,
                //    Extreme]`. The 51Folds LLM-generated
                //    `stateDescriptors[].name` sometimes uses
                //    "Negligent" (a real word but the wrong one) in
                //    place of the schema's canonical "Negligible".
                //    When the server returns `current.drivers[].state
                //    = "Negligible"` we can't find "Negligible" in
                //    `["Negligent", "Low", …]` by name, but both
                //    arrays have the same ordinal layout, so we look
                //    up the canonical state's index and use the
                //    descriptor at the same index. Without this step
                //    a driver whose server-canonical state is
                //    "Negligible" would leave `original_state` un-
                //    matchable against any pill and every re-eval
                //    would accidentally flag it as dirty.
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
                    {
                        if let Some((descriptor_name, _)) =
                            state_options.get(canonical_idx)
                        {
                            return descriptor_name.clone();
                        }
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
    fn is_complete(&self) -> bool {
        self.model
            .as_ref()
            .is_some_and(|m| m.is_complete())
    }

    fn poll(&mut self) {
        self.poll_main();
        self.poll_refresh();
    }

    /// Poll the main channel — used by create_and_poll and
    /// patch_drivers. Handles build/completion/reeval results.
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
                // These variants never arrive on the main channel —
                // they come in via refresh_rx. Ignore them here to
                // keep poll_main exhaustive but simple.
                Ok(FoldsResult::Refreshed(_))
                | Ok(FoldsResult::RefreshFailed(_)) => {}
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

    /// Poll the refresh channel — used by the Refresh Model button.
    fn poll_refresh(&mut self) {
        let Some(rx) = self.refresh_rx.take() else { return };
        loop {
            match rx.try_recv() {
                Ok(FoldsResult::Refreshed(model)) => {
                    self.model_id = Some(model.model_id.clone());
                    self.model = Some(model);
                    self.refresh_in_flight = false;
                    self.refresh_error = None;
                    // Clear any stale re-evaluation error — the fresh
                    // model state from the server supersedes it.
                    self.error = None;
                    self.init_draft_drivers();
                    // Refresh replaces in-memory state with server
                    // state, but does NOT create a history entry —
                    // rehydration isn't a distinct user action.
                    return;
                }
                Ok(FoldsResult::RefreshFailed(e)) => {
                    self.refresh_in_flight = false;
                    self.refresh_error = Some(e);
                    return;
                }
                Ok(_) => {} // other variants ignored on this channel
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
    /// database). On success, initializes draft drivers and pushes an
    /// "Original" snapshot onto the session history. If the blob is a
    /// stub (empty outcomes / empty drivers from earlier buggy writes),
    /// keeps the `model_id` so the UI can still offer Refresh, but
    /// leaves `model` as `None`.
    fn load_from_json(&mut self, json: &str) {
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

/// State for the embedded Dexter research-agent terminal.
struct ResearchTerminal {
    backend: Option<egui_term::TerminalBackend>,
    pty_rx: Option<std::sync::mpsc::Receiver<(u64, egui_term::PtyEvent)>>,
    /// Whether `bun` was found in PATH. `None` = not yet checked.
    bun_available: Option<bool>,
    /// If the child process exited, stores the exit code.
    child_exited: Option<i32>,
    /// Human-readable error (backend creation failure, missing key, etc.).
    error: Option<String>,
    /// Monotonically increasing ID for terminal backend instances.
    next_id: u64,
}

impl ResearchTerminal {
    fn new() -> Self {
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

pub struct DashboardApp {
    storage: Storage,
    settings: AppSettings,
    api_keys: ApiKeys,
    env_path: std::path::PathBuf,
    /// Set when the app cannot open its database. If Some, `update()` shows an
    /// error screen and skips all normal rendering.
    init_error: Option<String>,
    data: HashMap<Instrument, Vec<Observation>>,
    status_line: String,
    status_kind: StatusKind,
    refresh_in_flight: bool,
    refresh_rx: Option<Receiver<RefreshEvent>>,
    last_vix_level: Option<AlertLevel>,
    last_refresh_completed: Option<std::time::Instant>,
    activity_log: Vec<LogEntry>,
    highlighted_spike: Option<(chrono::NaiveDate, chrono::NaiveDate)>,
    synced_hover_x: Option<f64>,
    zoom_drag_start: Option<f64>,
    custom_zoom: Option<(chrono::NaiveDate, chrono::NaiveDate)>,
    show_help: bool,
    help_cache: egui_commonmark::CommonMarkCache,
    // Analysis cache — avoids recomputing expensive analysis every frame.
    data_generation: u64,
    cached_vix_status: Option<VixStatus>,
    cached_vix_summary: String,
    cached_spike_episodes: Vec<analysis::SpikeEpisode>,
    cache_data_gen: u64,
    cache_threshold_config: ThresholdConfig,
    cached_chart_end_date: Option<chrono::NaiveDate>,
    // Price panel: [P] opens a picker, selected instrument shown as a raw price chart.
    price_panel_instrument: Option<Instrument>,
    show_price_picker: bool,
    price_picker_just_opened: bool,
    price_picker_filter: String,
    price_picker_filter_prev: String,
    price_picker_cursor: usize,
    price_picker_candidates: Vec<Instrument>,
    show_activity_log: bool,
    // Per-chart collapse state (session-only, not persisted).
    vix_collapsed: bool,
    correlation_collapsed: bool,
    price_panel_collapsed: bool,
    // AI analysis state
    ai_task: LlmTask,
    ai_response: Option<String>,
    /// Source of the current inference ("anthropic", "openai", "dexter:anthropic", etc.)
    ai_response_provider: Option<String>,
    ai_panel_open: bool,
    ai_panel_content_height: f32,
    ai_markdown_cache: egui_commonmark::CommonMarkCache,
    inference_history: Vec<SavedInference>,
    history_page: usize,
    // Report generation state (Phase 2)
    #[allow(dead_code)]
    show_report_window: bool, // retained for settings persistence compatibility
    report_from: String,
    report_to: String,
    report_inferences: Vec<SavedInference>,
    report_task: LlmTask,
    report_result: Option<String>,
    report_markdown_cache: egui_commonmark::CommonMarkCache,
    // Central panel view mode + navigation stack
    central_view: CentralView,
    model_view: ModelView,
    /// Where to navigate back to from DriverDetail (DriverList or VisualMap).
    model_view_back: ModelView,
    // 51Folds state (session-only, not persisted)
    parsed_hypothesis: Option<ParsedHypothesis>,
    draft_hypothesis: Option<ParsedHypothesis>,
    folds_task: FoldsTask,
    /// The database row ID of the most recent AI inference. Used to link
    /// a 51Folds model back to the analysis that spawned it.
    last_inference_id: Option<i64>,
    /// Background task for re-rolling outcomes (a fresh set of outcomes for
    /// the current draft hypothesis). Independent from `ai_task` so it does
    /// not save to the inference history or touch the main analysis state.
    outcomes_task: LlmTask,
    /// Startup splash screen state — overlays the dashboard until the
    /// auto-dismiss timer elapses or the user clicks/presses a key.
    splash: SplashState,
    /// Shared mascot texture — decoded once from the embedded PNG and
    /// kept alive for the lifetime of the app. Used by both the startup
    /// splash and the Help window header.
    mascot_texture: Option<egui::TextureHandle>,
    /// When set, the Outcome tab renders a fading success toast
    /// announcing that probabilities were just updated from driver
    /// edits. Cleared once `now() >= reeval_toast_until`.
    reeval_toast_until: Option<std::time::Instant>,
    /// If set, a modal confirmation is asking the user to approve
    /// reverting to the original (from the DB baseline).
    revert_to_original_confirm: bool,
    /// Maps inference_id → (model_id, status) for showing 51Folds badges
    /// in the history list and report window.
    folds_status_by_inference: std::collections::HashMap<i64, (String, String)>,
    // -- DAG visualization --------------------------------------------------
    /// Stored raw window handle from CreationContext for wry child webview.
    stored_window_handle: Option<crate::dag::StoredWindowHandle>,
    /// The embedded D3.js DAG webview (lazy-initialized on first Visual Map visit).
    dag_webview: crate::dag::DagWebView,
    /// Generation counter for detecting when model data needs re-sending to JS.
    dag_model_generation: u64,
    /// Embedded Dexter terminal state (lazy-initialized on first tab switch).
    research_terminal: ResearchTerminal,
    /// Cached egui Context for TerminalBackend creation outside update().
    egui_ctx: egui::Context,
    // -- Commodity-bias validation ------------------------------------------
    /// Deterministic response validation results (populated instantly after
    /// each analysis completes).
    bias_deterministic: Vec<eval::ResponseValidation>,
    /// LLM judge results (populated asynchronously after the judge call
    /// completes; `None` while in flight or if no analysis has run).
    bias_judge_results: Option<Vec<eval::ResponseValidation>>,
    /// Which provider is running the judge (for display).
    bias_judge_provider: Option<LlmProvider>,
    /// Background LLM task for the bias judge.
    bias_judge_task: LlmTask,
}

/// Mascot PNG embedded at compile time (transparent background — chosen
/// so the character's dark navy outlines blend into our PANEL_BG instead
/// of sitting inside a jarring white rectangle).
const MASCOT_PNG: &[u8] =
    include_bytes!("../artwork/hedgehog-mascot-transparent.png");

/// Startup-splash overlay state. The actual mascot texture lives on
/// `DashboardApp::mascot_texture` so it can be shared with the Help
/// window; this struct only tracks the display timer and whether the
/// splash observed the startup auto-refresh.
struct SplashState {
    /// When the splash became visible. `None` once dismissed.
    shown_at: Option<std::time::Instant>,
    /// First frame we observed `refresh_in_flight == true` while the
    /// splash was visible. Used to detect "loading mode" and to know
    /// whether we need the post-load extension.
    loading_start: Option<std::time::Instant>,
    /// First frame after `loading_start` at which `refresh_in_flight`
    /// flipped back to `false`. Used to record that we've already
    /// applied the post-load hold extension so it doesn't re-trigger.
    loading_end: Option<std::time::Instant>,
}

impl SplashState {
    fn new() -> Self {
        Self {
            shown_at: Some(std::time::Instant::now()),
            loading_start: None,
            loading_end: None,
        }
    }

    fn is_active(&self) -> bool {
        self.shown_at.is_some()
    }

    fn elapsed(&self) -> std::time::Duration {
        self.shown_at
            .map(|t| t.elapsed())
            .unwrap_or_default()
    }

    fn dismiss(&mut self) {
        self.shown_at = None;
    }
}

/// Render the compact header at the top of the Help window — mascot on
/// the left, app name / tagline / version chip stacked on the right,
/// followed by a thin separator. Kept as a free function (not a
/// `&mut self` method) so it can be called from inside the
/// `egui::Window::open(&mut self.show_help)` closure without colliding
/// with that outer mutable borrow.
fn render_help_header(
    ui: &mut egui::Ui,
    mascot: Option<&egui::TextureHandle>,
) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if let Some(tex) = mascot {
            let orig = tex.size_vec2();
            let target_h = 96.0;
            let scale = target_h / orig.y;
            let size = orig * scale;
            ui.add(egui::Image::new(egui::load::SizedTexture::new(
                tex.id(),
                size,
            )));
        } else {
            ui.add_space(96.0);
        }

        ui.add_space(18.0);

        ui.vertical(|ui| {
            ui.add_space(8.0);
            ui.label(
                RichText::new("The Hedgehog")
                    .size(28.0)
                    .strong()
                    .color(Color32::WHITE),
            );
            ui.add_space(2.0);
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Causal, probabilistic modelling of capital-markets regimes",
                    )
                    .size(13.0)
                    .color(TEXT_SECONDARY),
                )
                .wrap(),
            );
            ui.add_space(10.0);
            egui::Frame::default()
                .fill(PANEL_BG)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .corner_radius(16.0)
                .inner_margin(egui::Margin {
                    left: 40,
                    right: 40,
                    top: 14,
                    bottom: 14,
                })
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("PREVIEW 0.1")
                            .size(13.0)
                            .strong()
                            .color(ACCENT_BLUE),
                    );
                });
        });
    });
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);
}

/// Decode the embedded mascot PNG and upload it as an egui texture. Only
/// called once, the first frame the splash is active. Returns `None` if
/// decoding fails — the splash will then render text-only (which is
/// acceptable; we shouldn't crash startup over a missing mascot).
fn load_mascot_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let img = image::load_from_memory(MASCOT_PNG).ok()?;
    let mut rgba = img.to_rgba8();
    // The "transparent" PNG actually has its editor's grey-and-white
    // checker pattern baked in as opaque pixels instead of real alpha.
    // Flood-fill it out before uploading so the mascot sits cleanly on
    // the splash card's dark SURFACE.
    strip_checker_background(&mut rgba);
    let size = [rgba.width() as usize, rgba.height() as usize];
    let pixels = rgba.as_flat_samples();
    let color_image =
        egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_slice());
    Some(ctx.load_texture(
        "hedgehog-mascot",
        color_image,
        egui::TextureOptions::LINEAR,
    ))
}

/// Rewrite the alpha channel of an RGBA image so that the editor's
/// transparency-checker pattern becomes truly transparent. Works in two
/// passes:
///
/// 1. **Flood-fill from every edge pixel** — any pixel that looks like
///    checker background (light and near-neutral grey) and is connected
///    to the image border gets alpha 0. The mascot's dark navy outline
///    is the natural stopping boundary for the fill, so interior whites
///    (face, belly, gloves) are preserved.
///
/// 2. **Edge-feather pass** — any pixel still opaque but adjacent to a
///    newly-transparent pixel has its alpha reduced if it's lightish.
///    This softens the anti-aliased band where the mascot's outline
///    originally blended with the checker, eliminating a visible halo.
fn strip_checker_background(rgba: &mut image::RgbaImage) {
    use std::collections::VecDeque;

    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return;
    }

    let idx_of = |x: u32, y: u32| (y * w + x) as usize;
    let mut visited = vec![false; (w * h) as usize];
    let mut queue: VecDeque<(u32, u32)> = VecDeque::new();

    // Seed from the entire 4-pixel border.
    for x in 0..w {
        queue.push_back((x, 0));
        queue.push_back((x, h - 1));
    }
    for y in 0..h {
        queue.push_back((0, y));
        queue.push_back((w - 1, y));
    }

    // Pass 1: connected-component flood fill from the border.
    while let Some((x, y)) = queue.pop_front() {
        let i = idx_of(x, y);
        if visited[i] {
            continue;
        }
        visited[i] = true;

        let [r, g, b, _] = rgba.get_pixel(x, y).0;
        if !is_checker_pixel(r, g, b) {
            continue;
        }
        rgba.put_pixel(x, y, image::Rgba([r, g, b, 0]));

        if x > 0 {
            queue.push_back((x - 1, y));
        }
        if x + 1 < w {
            queue.push_back((x + 1, y));
        }
        if y > 0 {
            queue.push_back((x, y - 1));
        }
        if y + 1 < h {
            queue.push_back((x, y + 1));
        }
    }

    // Pass 2: feather anti-aliased edge pixels. Snapshot the pass-1 state
    // so reads see original pixels, writes go to the live buffer.
    let snapshot = rgba.clone();
    for y in 0..h {
        for x in 0..w {
            let p = snapshot.get_pixel(x, y);
            if p.0[3] == 0 {
                continue;
            }
            // Count 4-connected transparent neighbours.
            let has_transparent_neighbor = (x > 0
                && snapshot.get_pixel(x - 1, y).0[3] == 0)
                || (x + 1 < w && snapshot.get_pixel(x + 1, y).0[3] == 0)
                || (y > 0 && snapshot.get_pixel(x, y - 1).0[3] == 0)
                || (y + 1 < h && snapshot.get_pixel(x, y + 1).0[3] == 0);
            if !has_transparent_neighbor {
                continue;
            }
            let [r, g, b, _] = p.0;
            let avg = (r as u16 + g as u16 + b as u16) / 3;
            // Very light edge pixel — almost certainly residual checker.
            if avg > 210 {
                rgba.put_pixel(x, y, image::Rgba([r, g, b, 0]));
            } else if avg > 170 {
                // Mid-tone edge pixel — reduce alpha to soften the outline.
                let t = (avg - 170) as f32 / 40.0;
                let new_alpha = ((1.0 - t) * 255.0) as u8;
                rgba.put_pixel(x, y, image::Rgba([r, g, b, new_alpha]));
            }
        }
    }
}

/// A pixel is "checker background" if it is both light and close to
/// neutral grey — ie. one of the two checker tiles (#FFF or ~#CCC) or a
/// lightly anti-aliased pixel between them.
fn is_checker_pixel(r: u8, g: u8, b: u8) -> bool {
    let avg = (r as u16 + g as u16 + b as u16) / 3;
    if avg < 170 {
        return false;
    }
    let max_c = r.max(g).max(b);
    let min_c = r.min(g).min(b);
    (max_c - min_c) < 30
}

impl DashboardApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // Capture the native window handle for the wry DAG webview.
        use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
        let stored_wh = _cc
            .window_handle()
            .ok()
            .and_then(|wh| {
                _cc.display_handle().ok().map(|dh| {
                    crate::dag::StoredWindowHandle::new(wh.as_raw(), dh.as_raw())
                })
            });

        // Apply global dark theme with design-system palette.
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = PANEL_BG;
        visuals.window_fill = PANEL_BG;
        visuals.extreme_bg_color = APP_BG;
        // Keep faint_bg_color close to PANEL_BG so it doesn't create a
        // visible lighter "stripe" in scroll areas or striped lists.
        visuals.faint_bg_color = Color32::from_rgb(20, 28, 45);
        visuals.widgets.noninteractive.bg_fill = SURFACE;
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
        // `widgets.noninteractive.fg_stroke.color` is what egui uses as the
        // default text color (see Visuals::text_color). The dark preset ships
        // with ~gray(140), which makes egui_commonmark body text render with
        // very low contrast on our PANEL_BG. Force it to TEXT_PRIMARY so
        // markdown-rendered content (report window, help, AI analysis) is
        // legible out of the box, without every label needing `.color(...)`.
        visuals.widgets.noninteractive.fg_stroke =
            egui::Stroke::new(1.0, TEXT_PRIMARY);
        visuals.override_text_color = Some(TEXT_PRIMARY);
        // `.weak()` text (blockquotes in egui_commonmark, etc.) resolves via
        // `weak_text_color() = gray_out(text_color())`, which tints towards
        // `widgets.noninteractive.weak_bg_fill`. The dark preset leaves that
        // at gray(27) so weak text blends towards near-black — completely
        // unreadable on our navy background. Point the fade-out target at a
        // mid-light grey so weak text stays legible (dimmer than primary,
        // but not invisible).
        visuals.widgets.noninteractive.weak_bg_fill = TEXT_SECONDARY;
        // `.strong()` text (bold, headings) resolves via `strong_text_color()`
        // which reads `widgets.active.fg_stroke.color`. Default dark is
        // already WHITE — pin it explicitly so nothing can drift it.
        visuals.widgets.active.fg_stroke =
            egui::Stroke::new(1.0, Color32::WHITE);
        visuals.widgets.inactive.bg_fill = SURFACE;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
        visuals.widgets.hovered.bg_fill = SURFACE_HOVER;
        visuals.widgets.active.bg_fill = SURFACE_HOVER;
        // Selection (e.g. selectable_value active state): use a visible accent
        // blue so selected items are clearly distinct from unselected.
        visuals.selection.bg_fill = Color32::from_rgb(37, 65, 130);
        visuals.selection.stroke = egui::Stroke::new(1.0, TEXT_SECONDARY);
        // Pin the theme preference to Dark and install our palette under BOTH
        // theme slots — otherwise a macOS system in Light mode can fall back
        // to egui's default light visuals mid-session (central panel glitch).
        _cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
        _cc.egui_ctx
            .set_visuals_of(egui::Theme::Dark, visuals.clone());
        _cc.egui_ctx.set_visuals_of(egui::Theme::Light, visuals);

        let db_path = database_path();
        if let Some(parent) = db_path.parent() {
            let _ = fs::create_dir_all(parent);
            // Restrict the data directory to the current user only.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
        }

        let (storage, init_error) = match Storage::open(&db_path) {
            Ok(s) => {
                // Restrict DB file to current user only (no-op if it fails).
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(&db_path, fs::Permissions::from_mode(0o600));
                }
                (s, None)
            }
            Err(e) => {
                let msg = format!(
                    "Could not open the database at:\n  {}\n\n{:#}\n\nPossible causes: another instance is already running, the file is corrupted, or the directory is not writable.",
                    db_path.display(),
                    e
                );
                let fallback = Storage::open_memory()
                    .expect("cannot open in-memory fallback database");
                (fallback, Some(msg))
            }
        };
        let settings = storage.load_settings().unwrap_or_else(|e| { eprintln!("warn: {e}"); AppSettings::default() });

        // Keys live in .env only — never in the database. We prefer .env
        // sitting next to the binary (the layout shipped in release
        // archives, where double-clicking from Finder/Explorer sets the
        // cwd to something unrelated to the binary's folder), and fall
        // back to dotenvy's cwd walk for `cargo run` dev workflows where
        // the binary is in target/release/ but .env is at the project
        // root. The remembered path is what the Save button writes back to.
        let exe_neighbour = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.join(".env")));
        let env_path = if let Some(path) = exe_neighbour.as_ref().filter(|p| p.exists()) {
            if let Err(e) = dotenvy::from_path(path) {
                eprintln!("warn: failed to load .env from {}: {e}", path.display());
            }
            path.clone()
        } else {
            match dotenvy::dotenv() {
                Ok(path) => path,
                Err(_) => exe_neighbour
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join(".env")),
            }
        };
        let api_keys = ApiKeys::from_env();

        let mut app = Self {
            storage,
            settings,
            api_keys,
            env_path,
            init_error,
            data: HashMap::new(),
            status_line: String::new(),
            status_kind: StatusKind::Info,
            refresh_in_flight: false,
            refresh_rx: None,
            last_vix_level: None,
            last_refresh_completed: None,
            activity_log: Vec::new(),
            highlighted_spike: None,
            synced_hover_x: None,
            zoom_drag_start: None,
            custom_zoom: None,
            show_help: false,
            help_cache: egui_commonmark::CommonMarkCache::default(),
            data_generation: 0,
            cached_vix_status: None,
            cached_vix_summary: String::new(),
            cached_spike_episodes: Vec::new(),
            cache_data_gen: 0,
            cache_threshold_config: ThresholdConfig::default(),
            cached_chart_end_date: None,
            price_panel_instrument: None,
            show_price_picker: false,
            price_picker_just_opened: false,
            price_picker_filter: String::new(),
            price_picker_filter_prev: String::new(),
            price_picker_cursor: 0,
            price_picker_candidates: Vec::new(),
            vix_collapsed: false,
            correlation_collapsed: false,
            price_panel_collapsed: false,
            show_activity_log: true,
            ai_task: LlmTask::new(),
            ai_response: None,
            ai_response_provider: None,
            ai_panel_open: false,
            ai_panel_content_height: 200.0,
            ai_markdown_cache: egui_commonmark::CommonMarkCache::default(),
            inference_history: Vec::new(),
            history_page: 0,
            show_report_window: false,
            report_from: String::new(),
            report_to: String::new(),
            report_inferences: Vec::new(),
            report_task: LlmTask::new(),
            report_result: None,
            report_markdown_cache: egui_commonmark::CommonMarkCache::default(),
            central_view: CentralView::Charts,
            model_view: ModelView::Outcome,
            model_view_back: ModelView::DriverList,
            parsed_hypothesis: None,
            draft_hypothesis: None,
            folds_task: FoldsTask::new(),
            last_inference_id: None,
            outcomes_task: LlmTask::new(),
            splash: SplashState::new(),
            mascot_texture: None,
            reeval_toast_until: None,
            revert_to_original_confirm: false,
            folds_status_by_inference: std::collections::HashMap::new(),
            stored_window_handle: stored_wh,
            dag_webview: crate::dag::DagWebView::new(),
            dag_model_generation: 0,
            research_terminal: ResearchTerminal::new(),
            egui_ctx: _cc.egui_ctx.clone(),
            bias_deterministic: Vec::new(),
            bias_judge_results: None,
            bias_judge_provider: None,
            bias_judge_task: LlmTask::new(),
        };

        // Skip data loading and auto-refresh if the database couldn't be opened;
        // the error screen in update() will handle it from here.
        if app.init_error.is_some() {
            return app;
        }

        app.reload_from_storage();
        app.refresh_analysis_cache();
        let _ = app.storage.seed_knowledge_chunks(
            &knowledge::KNOWLEDGE_BASE
                .iter()
                .map(|c| (c.title, c.tags, c.body))
                .collect::<Vec<_>>(),
        );
        app.reload_inference_history();
        app.last_vix_level = app.cached_vix_status.as_ref().map(|s| s.level);

        // Resume polling for any 51Folds models that were `pending` when
        // the app last shut down. Anything older than 35 min gets marked
        // `undisclosed_failure` immediately; anything fresher gets a
        // background polling thread that will update the DB independently
        // of the live UI session.
        app.resume_pending_folds_models();

        // Auto-refresh on startup when both keys are present and the user has
        // not opted out. The cache check inside start_refresh() makes this a
        // no-op for instruments whose latest stored close is already today's,
        // so opening the app multiple times in a day is free.
        let has_fred = app.api_keys.has_fred();
        let has_commodity = app.api_keys.has_alpha_vantage();

        if !has_fred || !has_commodity {
            app.set_status(
                "API keys missing. Enter FRED and Alpha Vantage keys in the sidebar under Data Source, then click Refresh.",
                StatusKind::Error,
            );
        } else if app.settings.auto_refresh_on_startup {
            app.start_refresh();
        } else {
            app.set_status(
                "Auto-refresh disabled. Click Refresh to fetch latest data.",
                StatusKind::Info,
            );
        }

        app
    }

    fn series(&self, instrument: Instrument) -> &[Observation] {
        self.data.get(&instrument).map(Vec::as_slice).unwrap_or(&[])
    }

    fn reload_from_storage(&mut self) {
        self.data.clear();
        for instrument in Instrument::ALL {
            let mut observations = self
                .storage
                .load_observations(instrument)
                .unwrap_or_else(|e| { eprintln!("warn: {e}"); Vec::new() });

            // Filter by source (VIX from FRED, all commodities from Alpha Vantage)
            if instrument == Instrument::Vix {
                observations.retain(|o| o.source == "FRED VIXCLS");
            } else {
                observations.retain(|o| o.source.starts_with("Alpha Vantage"));
            }

            self.data.insert(instrument, observations);
        }
        // Compute reference end date for chart alignment: latest date across all data
        self.cached_chart_end_date = Instrument::ALL
            .iter()
            .filter_map(|&inst| self.series(inst).last().map(|o| o.date))
            .max();
        self.data_generation = self.data_generation.wrapping_add(1);
    }

    fn refresh_analysis_cache(&mut self) {
        if self.data_generation == self.cache_data_gen
            && self.settings.threshold_config == self.cache_threshold_config
        {
            return;
        }

        self.cached_vix_status = analysis::compute_vix_status(
            self.series(Instrument::Vix),
            &self.settings.threshold_config,
        );
        self.cached_vix_summary = self.cached_vix_status.as_ref()
            .map(|s| format!("{:.2} - {}", s.latest.close, s.level.label()))
            .unwrap_or_default();
        self.cached_spike_episodes = analysis::recent_spike_episodes(
            self.series(Instrument::Vix),
            &self.settings.threshold_config,
            5,
        );
        self.cache_threshold_config = self.settings.threshold_config.clone();
        self.cache_data_gen = self.data_generation;
    }

    fn set_status(&mut self, msg: &str, kind: StatusKind) {
        self.status_line = msg.to_owned();
        self.status_kind = kind;
    }

    /// Write API keys back to the `.env` file via atomic write (tmp + rename),
    /// preserving all other lines (comments, other variables).
    fn save_keys_to_env(&self) -> Result<(), String> {
        let existing = fs::read_to_string(&self.env_path).unwrap_or_default();
        let updated = update_env_content(&existing, &[
            ("FRED_API_KEY", self.api_keys.fred.trim()),
            ("ALPHA_VANTAGE_API_KEY", self.api_keys.alpha_vantage.trim()),
            ("ANTHROPIC_API_KEY", self.api_keys.anthropic.trim()),
            ("OPENAI_API_KEY", self.api_keys.openai.trim()),
            ("FOLDS_API_KEY", self.api_keys.folds.trim()),
        ]);
        let tmp_path = self.env_path.with_extension("tmp");
        fs::write(&tmp_path, &updated)
            .map_err(|e| format!("Failed to write temp file: {e}"))?;
        fs::rename(&tmp_path, &self.env_path)
            .map_err(|e| format!("Failed to rename .env.tmp to .env: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.env_path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// Spawn (or re-spawn) the Dexter research agent in an embedded PTY terminal.
    fn spawn_research_terminal(&mut self) {
        self.research_terminal.child_exited = None;
        self.research_terminal.error = None;
        self.research_terminal.backend = None;
        self.research_terminal.pty_rx = None;

        let provider = self.settings.research_provider;
        let api_key = self.api_keys.ai_key_for(provider).trim().to_owned();

        if api_key.is_empty() {
            self.research_terminal.error = Some(format!(
                "No API key for {}. Configure it in Settings → Research Agent.",
                provider.label(),
            ));
            return;
        }

        let dexter_dir = self
            .env_path
            .parent()
            .map(|p| p.join("vendor").join("dexter"))
            .unwrap_or_else(|| PathBuf::from("vendor/dexter"));

        let env_key_name = match provider {
            LlmProvider::Anthropic => "ANTHROPIC_API_KEY",
            LlmProvider::OpenAI => "OPENAI_API_KEY",
        };

        // Build env exports: LLM key + any Dexter-specific data source keys
        // present in the environment.
        let mut exports = format!("export {env_key_name}='{api_key}'");
        let passthrough_keys = [
            "FINANCIAL_DATASETS_API_KEY",
            "EXASEARCH_API_KEY",
            "PERPLEXITY_API_KEY",
            "TAVILY_API_KEY",
            "X_BEARER_TOKEN",
        ];
        for key in passthrough_keys {
            if let Ok(val) = std::env::var(key) {
                if !val.is_empty() {
                    exports.push_str(&format!(" && export {key}='{val}'"));
                }
            }
        }

        // Pass the Hedgehog database path so /51folds can write hypotheses.
        let db_path = database_path();
        let db_path_str = db_path.display().to_string();
        exports.push_str(&format!(" && export HEDGEHOG_DB_PATH='{db_path_str}'"));

        let settings = egui_term::BackendSettings {
            shell: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                format!("{exports} && exec bun run start"),
            ],
            working_directory: Some(dexter_dir),
        };

        let (tx, rx) = mpsc::channel();
        let id = self.research_terminal.next_id;
        self.research_terminal.next_id += 1;

        match egui_term::TerminalBackend::new(id, self.egui_ctx.clone(), tx, settings) {
            Ok(backend) => {
                self.research_terminal.backend = Some(backend);
                self.research_terminal.pty_rx = Some(rx);
            }
            Err(e) => {
                self.research_terminal.error =
                    Some(format!("Failed to start terminal: {e}"));
            }
        }
    }

    /// Drain PTY events for the research terminal.
    fn poll_research_terminal(&mut self) {
        let mut dexter_inference_id: Option<i64> = None;

        let Some(ref rx) = self.research_terminal.pty_rx else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok((_id, event)) => match event {
                    egui_term::PtyEvent::ChildExit(code) => {
                        self.research_terminal.child_exited = Some(code);
                        self.research_terminal.backend = None;
                        self.research_terminal.pty_rx = None;
                        break;
                    }
                    egui_term::PtyEvent::Exit => {
                        self.research_terminal.child_exited = Some(0);
                        self.research_terminal.backend = None;
                        self.research_terminal.pty_rx = None;
                        break;
                    }
                    egui_term::PtyEvent::Title(ref title) => {
                        // /51folds signals hypothesis ready via OSC title.
                        if let Some(id_str) = title.strip_prefix("51folds:ready:") {
                            if let Ok(id) = id_str.parse::<i64>() {
                                dexter_inference_id = Some(id);
                            }
                        }
                    }
                    _ => {}
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.research_terminal.child_exited.is_none() {
                        self.research_terminal.child_exited = Some(-1);
                    }
                    self.research_terminal.backend = None;
                    self.research_terminal.pty_rx = None;
                    break;
                }
            }
        }

        // Handle dexter inference signal after the borrow is released.
        if let Some(id) = dexter_inference_id {
            self.load_dexter_inference(id);
        }
    }

    fn start_refresh(&mut self) {
        if self.refresh_in_flight {
            return;
        }

        // Cooldown check: don't allow refresh within 60 seconds of the last one.
        if let Some(t) = self.last_refresh_completed {
            if t.elapsed() < Duration::from_secs(60) {
                self.set_status(
                    "Refresh on cooldown. Please wait before refreshing again.",
                    StatusKind::Info,
                );
                return;
            }
        }

        let fred_key = self.api_keys.fred.trim().to_owned();
        let alpha_key = self.api_keys.alpha_vantage.trim().to_owned();

        if fred_key.is_empty() || alpha_key.is_empty() {
            self.set_status(
                "API keys missing. Enter FRED and Alpha Vantage keys in the sidebar, then click Refresh.",
                StatusKind::Error,
            );
            return;
        }

        // Build per-provider cache maps so providers::refresh_market_data can
        // skip API calls for instruments whose latest stored close already
        // matches today (Alpha Vantage publishes one daily close per spot
        // commodity, so a same-day re-fetch adds nothing).
        let mut cached_dates_alpha = std::collections::HashMap::new();
        for &instrument in &Instrument::ALL {
            if instrument == Instrument::Vix {
                continue; // FRED-only instrument
            }
            if let Ok(Some(date)) = self
                .storage
                .last_observation_date_for_provider(instrument, "Alpha Vantage")
            {
                cached_dates_alpha.insert(instrument, date);
            }
        }
        let mut cached_dates_fred = std::collections::HashMap::new();
        if let Ok(Some(date)) = self
            .storage
            .last_observation_date_for_provider(Instrument::Vix, "FRED VIXCLS")
        {
            cached_dates_fred.insert(Instrument::Vix, date);
        }

        let (tx, rx) = mpsc::channel();
        self.refresh_in_flight = true;
        self.refresh_rx = Some(rx);
        self.activity_log.clear();
        self.show_activity_log = true;
        self.set_status(
            "Refreshing from FRED and Alpha Vantage...",
            StatusKind::Info,
        );

        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                providers::refresh_market_data(
                    &fred_key,
                    &alpha_key,
                    tx.clone(),
                    cached_dates_fred,
                    cached_dates_alpha,
                );
            }));
            if result.is_err() {
                let _ = tx.send(RefreshEvent::FetchFailed {
                    instrument: Instrument::Vix,
                    source: "Unknown".to_string(),
                    error: "Refresh thread panicked unexpectedly.".to_owned(),
                });
                let _ = tx.send(RefreshEvent::Done);
            }
        });
    }

    fn poll_refresh(&mut self) {
        if !self.refresh_in_flight {
            return;
        }

        let Some(rx) = self.refresh_rx.take() else {
            return;
        };

        let mut any_saved = false;
        let mut done = false;
        loop {
            match rx.try_recv() {
                Ok(RefreshEvent::Fetching { instrument, source }) => {
                    self.activity_log.push(LogEntry {
                        timestamp_str: chrono::Utc::now().format("%H:%M:%S").to_string(),
                        instrument,
                        source,
                        status: LogStatus::Fetching,
                    });
                    if self.activity_log.len() > MAX_LOG_ENTRIES {
                        self.activity_log.drain(..self.activity_log.len() - MAX_LOG_ENTRIES);
                    }
                }
                Ok(RefreshEvent::Fetched(batch)) => {
                    let instrument = batch.instrument;
                    let source = batch.source.to_string();
                    match self
                        .storage
                        .replace_observations(instrument, &batch.observations)
                    {
                        Ok(count) => {
                            self.update_log_entry(instrument, source, LogStatus::Ok(count));
                            any_saved = true;
                        }
                        Err(err) => {
                            self.update_log_entry(
                                instrument,
                                source,
                                LogStatus::Failed(format!("save: {err:#}")),
                            );
                        }
                    }
                }
                Ok(RefreshEvent::Cached { instrument, source, date }) => {
                    self.update_log_entry(instrument, source, LogStatus::Cached(date));
                }
                Ok(RefreshEvent::FetchFailed { instrument, source, error }) => {
                    self.update_log_entry(instrument, source, LogStatus::Failed(error));
                }
                Ok(RefreshEvent::Done) => {
                    self.refresh_in_flight = false;
                    self.last_refresh_completed = Some(std::time::Instant::now());
                    done = true;
                    if any_saved {
                        self.reload_from_storage();
                        self.refresh_analysis_cache();
                        self.evaluate_alert_transition();
                    }
                    let ok_count = self
                        .activity_log
                        .iter()
                        .filter(|e| matches!(e.status, LogStatus::Ok(_)))
                        .count();
                    let cache_count = self
                        .activity_log
                        .iter()
                        .filter(|e| matches!(e.status, LogStatus::Cached(_)))
                        .count();
                    let fail_count = self
                        .activity_log
                        .iter()
                        .filter(|e| matches!(e.status, LogStatus::Failed(_)))
                        .count();
                    let total_pts: usize = self
                        .activity_log
                        .iter()
                        .filter_map(|e| {
                            if let LogStatus::Ok(n) = e.status {
                                Some(n)
                            } else {
                                None
                            }
                        })
                        .sum();
                    if fail_count == 0 && (ok_count > 0 || cache_count > 0) {
                        let mut msg = format!("Refreshed {ok_count} instruments ({total_pts} points).");
                        if cache_count > 0 {
                            msg = format!("Updated {ok_count}, cached {cache_count} ({total_pts} points).");
                        }
                        self.set_status(&msg, StatusKind::Success);
                    } else if ok_count > 0 {
                        self.set_status(
                            &format!("Partial refresh: {ok_count} OK, {fail_count} failed."),
                            StatusKind::Error,
                        );
                    } else {
                        self.set_status(
                            "Refresh failed for all instruments.",
                            StatusKind::Error,
                        );
                    }
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.refresh_in_flight = false;
                    done = true;
                    if any_saved {
                        self.reload_from_storage();
                    }
                    self.set_status("Refresh interrupted.", StatusKind::Error);
                    break;
                }
            }
        }
        if !done {
            self.refresh_rx = Some(rx);
        }
    }

    fn update_log_entry(&mut self, instrument: Instrument, source: String, status: LogStatus) {
        if let Some(entry) = self
            .activity_log
            .iter_mut()
            .rev()
            .find(|e| e.instrument == instrument && e.source == source && matches!(e.status, LogStatus::Fetching))
        {
            entry.status = status;
            entry.timestamp_str = chrono::Utc::now().format("%H:%M:%S").to_string();
        } else {
            self.activity_log.push(LogEntry {
                timestamp_str: chrono::Utc::now().format("%H:%M:%S").to_string(),
                instrument,
                source,
                status,
            });
            if self.activity_log.len() > MAX_LOG_ENTRIES {
                self.activity_log.drain(..self.activity_log.len() - MAX_LOG_ENTRIES);
            }
        }
    }

    fn evaluate_alert_transition(&mut self) {
        let Some(status) = self.cached_vix_status.as_ref() else {
            return;
        };

        if self.last_vix_level == Some(status.level) {
            return;
        }

        let level = status.level;
        let note = format!(
            "VIX moved to {} at {:.2} (approaching {:.2}, extreme {:.2})",
            status.level.label(),
            status.latest.close,
            status.thresholds.approaching,
            status.thresholds.extreme
        );
        let event = AlertEvent {
            timestamp_utc: Utc::now(),
            instrument: Instrument::Vix,
            level,
            note,
        };
        self.last_vix_level = Some(level);
        let _ = self.storage.insert_alert_event(&event);
    }

    fn start_ai_analysis(&mut self) {
        if self.ai_task.in_flight {
            return;
        }

        let provider = self.settings.ai_provider;
        let api_key = self.api_keys.ai_key_for(provider).trim().to_owned();

        if api_key.is_empty() {
            self.ai_task.error = Some(format!(
                "No API key configured for {}. Add it in the AI Analysis sidebar section.",
                provider.label()
            ));
            self.ai_panel_open = true;
            return;
        }

        if let Err(e) = validate_model_name(self.settings.effective_model()) {
            self.ai_task.error = Some(e);
            self.ai_panel_open = true;
            return;
        }

        // Retrieve knowledge BEFORE spawning (Storage is not Send).
        let instrument_tags: Vec<&str> = self
            .settings
            .overlay_instruments
            .iter()
            .map(|i| i.storage_key())
            .collect();
        let knowledge_chunks = knowledge::retrieve_for_context(&self.storage, &instrument_tags);

        // Assemble context from cached analysis state. Snapshots carry the
        // absolute close + date so the LLM has authoritative ground truth and
        // does not fall back on training-data price priors.
        let instrument_snapshots: Vec<ai::InstrumentSnapshot> = self
            .settings
            .overlay_instruments
            .iter()
            .map(|&inst| ai::InstrumentSnapshot::from_series(inst, self.series(inst)))
            .collect();

        // Instruments not on the chart — still sent for cross-instrument context.
        let unselected_snapshots: Vec<ai::InstrumentSnapshot> = Instrument::ALL
            .iter()
            .filter(|&&inst| {
                inst != Instrument::Vix
                    && !self.settings.overlay_instruments.contains(&inst)
            })
            .map(|&inst| ai::InstrumentSnapshot::from_series(inst, self.series(inst)))
            .collect();

        let user_message = ai::assemble_user_message(
            self.cached_vix_status.as_ref(),
            &self.settings.overlay_instruments,
            &instrument_snapshots,
            &unselected_snapshots,
            &self.cached_spike_episodes,
        );
        let system_prompt = ai::assemble_system_prompt(&knowledge_chunks);

        let request = ai::AiRequest {
            provider,
            api_key,
            model: self.settings.effective_model().to_owned(),
            // Headroom: the template asks for ~300 words in the Hypothesis
            // Context alone (~400 tokens) plus six other narrative
            // sections. 512 was truncating Context mid-sentence; 2048
            // gives the model room without inviting runaway responses.
            max_tokens: 2048,
            system_prompt,
            user_message,
        };

        let (tx, rx) = mpsc::channel();
        self.ai_task.start(rx);
        self.ai_panel_open = true;

        thread::spawn(move || {
            ai::run_analysis(request, tx);
        });
    }

    /// Send the current draft hypothesis (question + context + previous
    /// outcomes) back to the LLM and ask for a fresh set of outcomes only.
    /// Used by the "Different Outcomes" button in the 51Folds section so the
    /// user can iterate on outcome framing without regenerating the whole
    /// regime analysis or burning a row in the inference history.
    fn start_outcomes_reroll(&mut self) {
        if self.outcomes_task.in_flight {
            return;
        }
        let Some(ref draft) = self.draft_hypothesis else { return };

        let provider = self.settings.ai_provider;
        let api_key = self.api_keys.ai_key_for(provider).trim().to_owned();
        if api_key.is_empty() {
            self.outcomes_task.error = Some(format!(
                "No API key configured for {}.",
                provider.label()
            ));
            return;
        }
        if let Err(e) = validate_model_name(self.settings.effective_model()) {
            self.outcomes_task.error = Some(e);
            return;
        }

        let (system_prompt, user_message) = ai::assemble_outcomes_reroll_prompt(
            &draft.question,
            &draft.context,
            &draft.outcomes,
        );

        let request = ai::AiRequest {
            provider,
            api_key,
            model: self.settings.effective_model().to_owned(),
            // Outcomes are short — a small budget is plenty and keeps the
            // round-trip snappy.
            max_tokens: 256,
            system_prompt,
            user_message,
        };

        let (tx, rx) = mpsc::channel();
        self.outcomes_task.start(rx);

        thread::spawn(move || {
            ai::run_analysis(request, tx);
        });
    }

    /// Poll the outcomes-reroll task. On success, replace the outcomes in the
    /// draft hypothesis (question + context untouched) so the user can keep
    /// iterating from the same hypothesis statement.
    fn poll_outcomes_reroll(&mut self) {
        match self.outcomes_task.poll() {
            LlmPoll::Response(result) => {
                match ai::parse_outcomes_reroll(&result.response) {
                    Some(new_outcomes) => {
                        if let Some(ref mut draft) = self.draft_hypothesis {
                            draft.outcomes = new_outcomes;
                        }
                    }
                    None => {
                        self.outcomes_task.error = Some(
                            "Could not parse outcomes from LLM response.".to_owned(),
                        );
                    }
                }
            }
            LlmPoll::Failed | LlmPoll::Pending | LlmPoll::Idle => {}
        }
    }

    fn poll_ai(&mut self) {
        match self.ai_task.poll() {
            LlmPoll::Response(result) => {
                let vix_close = self.cached_vix_status.as_ref().map(|s| s.latest.close);
                let vix_level_str = self.cached_vix_status.as_ref().map(|s| match s.level {
                    AlertLevel::Normal => "normal",
                    AlertLevel::ApproachingExtreme => "approaching_extreme",
                    AlertLevel::Extreme => "extreme",
                });

                // Parse the hypothesis BEFORE persisting so we can store
                // the structured fields alongside the raw response. The
                // overlay snapshot is also captured here so the report
                // window can label past analyses by their instrument
                // selection (Gold vs Silver vs etc).
                let hypothesis = ai::parse_hypothesis(&result.response);
                let overlay_keys: Vec<String> = self
                    .settings
                    .overlay_instruments
                    .iter()
                    .map(|i| i.storage_key().to_owned())
                    .collect();

                let inference_id = self.storage.save_inference(
                    &result.provider,
                    &result.model,
                    &result.system_prompt,
                    &result.user_message,
                    &result.response,
                    vix_close,
                    vix_level_str,
                    hypothesis.as_ref().map(|h| h.question.as_str()),
                    hypothesis.as_ref().map(|h| h.outcomes.as_slice()),
                    hypothesis.as_ref().map(|h| h.context.as_str()),
                    Some(overlay_keys.as_slice()),
                );
                self.last_inference_id = inference_id.ok();

                // -- Commodity-bias validation (Layer 1: deterministic) --------
                self.bias_deterministic.clear();
                self.bias_judge_results = None;

                if let Some(ref hyp) = hypothesis {
                    let snapshots: Vec<ai::InstrumentSnapshot> = self
                        .settings
                        .overlay_instruments
                        .iter()
                        .map(|&inst| ai::InstrumentSnapshot::from_series(inst, self.series(inst)))
                        .collect();

                    let response_scenario = eval::ResponseScenario::from_analysis(
                        &self.settings.overlay_instruments,
                        &snapshots,
                        &hyp.question,
                        &hyp.outcomes,
                        &hyp.context,
                    );
                    self.bias_deterministic = eval::validate_response(&response_scenario);

                    // -- Layer 2: LLM judge (async, cross-model when possible)
                    // Prefer the OTHER provider for independent validation.
                    // Fall back to the analysis provider if only one key exists.
                    let analysis_provider = self.settings.ai_provider;
                    let other_provider = match analysis_provider {
                        LlmProvider::Anthropic => LlmProvider::OpenAI,
                        LlmProvider::OpenAI => LlmProvider::Anthropic,
                    };
                    let other_key = self.api_keys.ai_key_for(other_provider).trim().to_owned();
                    let (judge_provider, judge_key) = if !other_key.is_empty() {
                        (other_provider, other_key)
                    } else {
                        let same_key = self.api_keys.ai_key_for(analysis_provider).trim().to_owned();
                        (analysis_provider, same_key)
                    };

                    if !judge_key.is_empty() {
                        let judge_model = judge_provider.default_model().to_owned();
                        self.bias_judge_provider = Some(judge_provider);

                        let (judge_sys, judge_usr) = eval::assemble_bias_judge_prompt(
                            &self.settings.overlay_instruments,
                            &snapshots,
                            &result.response,
                        );
                        let judge_request = ai::AiRequest {
                            provider: judge_provider,
                            api_key: judge_key,
                            model: judge_model,
                            max_tokens: 512,
                            system_prompt: judge_sys,
                            user_message: judge_usr,
                        };
                        let (jtx, jrx) = mpsc::channel();
                        self.bias_judge_task.start(jrx);
                        thread::spawn(move || {
                            ai::run_analysis(judge_request, jtx);
                        });
                    }
                }

                self.parsed_hypothesis = hypothesis.clone();
                self.draft_hypothesis = hypothesis;
                self.folds_task.reset();
                self.ai_response = Some(result.response);
                self.ai_response_provider = Some(result.provider.clone());
                self.reload_inference_history();
            }
            LlmPoll::Failed | LlmPoll::Pending | LlmPoll::Idle => {}
        }
    }

    /// Poll the background LLM bias-judge task. When it completes,
    /// parse the structured output into `ResponseValidation` results.
    fn poll_bias_judge(&mut self) {
        match self.bias_judge_task.poll() {
            LlmPoll::Response(result) => {
                self.bias_judge_results =
                    Some(eval::parse_bias_judge(&result.response));
            }
            LlmPoll::Failed => {
                // Judge failure is non-critical — deterministic checks
                // still stand. Surface the error as a single failed result
                // so the UI shows something.
                let err_msg = self
                    .bias_judge_task
                    .error
                    .clone()
                    .unwrap_or_else(|| "Judge call failed".to_owned());
                self.bias_judge_results = Some(vec![eval::ResponseValidation {
                    rule: "JUDGE_ERROR".to_owned(),
                    pass: false,
                    reason: err_msg,
                }]);
            }
            LlmPoll::Pending | LlmPoll::Idle => {}
        }
    }

    /// Restore the AI panel + 51Folds editor state from a previously
    /// saved inference. Used when the user clicks a row in the sidebar
    /// History list or the Report window inference list. Without this
    /// helper the click handlers would only set `ai_response` and the
    /// 51Folds section would render "No hypothesis in this analysis"
    /// because `draft_hypothesis` was still `None`.
    ///
    /// Hypothesis fields come from the persisted columns when present
    /// (post-migration rows); falls back to re-parsing the raw response
    /// markdown for older rows that pre-date the migration.
    /// Load a Dexter-generated hypothesis from the database (triggered
    /// by the OSC title signal from `/51folds`).
    fn load_dexter_inference(&mut self, id: i64) {
        match self.storage.load_inference_by_id(id) {
            Ok(Some(inf)) => {
                self.load_historical_inference(inf);
                self.reload_inference_history();
                self.set_status(
                    &format!("Research hypothesis loaded (inference #{id})"),
                    StatusKind::Success,
                );
            }
            Ok(None) => {
                eprintln!("warn: dexter signalled inference #{id} but row not found");
            }
            Err(e) => {
                eprintln!("warn: failed to load dexter inference #{id}: {e:#}");
            }
        }
    }

    fn load_historical_inference(&mut self, inf: SavedInference) {
        let hypothesis = match (
            inf.hypothesis_question.clone(),
            inf.hypothesis_outcomes.clone(),
            inf.hypothesis_context.clone(),
        ) {
            (Some(question), Some(outcomes), Some(context))
                if !question.is_empty() && !outcomes.is_empty() =>
            {
                Some(ParsedHypothesis { question, outcomes, context })
            }
            _ => ai::parse_hypothesis(&inf.response),
        };

        self.parsed_hypothesis = hypothesis.clone();
        self.draft_hypothesis = hypothesis;
        self.last_inference_id = Some(inf.id);

        // Preserve in-flight build state across the reset — loading a
        // historical inference should not kill a running model build.
        // The toolbar spinner and central-view progress depend on these
        // fields staying alive; when the build completes, poll_folds
        // will auto-switch to the new model regardless of what's
        // currently displayed.
        let stashed_in_flight = self.folds_task.in_flight;
        let stashed_rx = self.folds_task.rx.take();
        let stashed_reevaluating = self.folds_task.reevaluating;

        // Clear previous folds_task, then try to load the linked model
        // from the database so completed models appear immediately.
        self.folds_task.reset();
        if let Ok(Some(json)) = self.storage.load_folds_response_for_inference(inf.id) {
            self.folds_task.load_from_json(&json);
            if self.folds_task.model_id.is_some() {
                self.central_view = CentralView::Model;
                self.model_view = ModelView::Outcome;
            }
            // Stubbed/incomplete model + API key → auto-refresh from server.
            if !self.folds_task.is_complete()
                && self.folds_task.model_id.is_some()
                && !self.api_keys.folds.trim().is_empty()
            {
                self.start_folds_refresh();
            }
        } else if let Ok(Some(model_id)) =
            self.storage.load_folds_model_id_for_inference(inf.id)
        {
            // Model exists but has no response_json (e.g. timed out
            // locally). Set the model_id and auto-refresh from the
            // server — the model may have completed after the app
            // stopped polling.
            self.folds_task.model_id = Some(model_id);
            self.central_view = CentralView::Model;
            self.model_view = ModelView::Outcome;
            if !self.api_keys.folds.trim().is_empty() {
                self.start_folds_refresh();
            }
        }
        // Restore in-flight build state so the toolbar spinner stays
        // alive and poll_main can still receive the completion event.
        if stashed_in_flight {
            self.folds_task.in_flight = true;
            self.folds_task.rx = stashed_rx;
            self.folds_task.reevaluating = stashed_reevaluating;
        }

        self.outcomes_task = LlmTask::new();
        self.ai_response = Some(inf.response);
        self.ai_response_provider = Some(inf.provider);
        self.ai_task.error = None;
        self.ai_panel_open = true;
    }

    fn start_folds_create(&mut self) {
        let Some(ref draft) = self.draft_hypothesis else { return };
        let api_key = self.api_keys.folds.trim().to_owned();
        if api_key.is_empty() { return; }

        let req = crate::folds::FoldsCreateRequest {
            question: draft.question.clone(),
            outcomes: draft.outcomes.clone(),
            additional_context: draft.context.clone(),
            model_type: FOLDS_MODEL_TYPE.to_owned(),
        };

        let db_path = database_path();
        let inference_id = self.last_inference_id;
        let created_at = chrono::Utc::now();

        let (tx, rx) = mpsc::channel();
        self.folds_task.start(rx);

        thread::spawn(move || {
            crate::folds::create_and_poll(
                api_key,
                req,
                db_path,
                inference_id,
                created_at,
                tx,
            );
        });
    }

    fn start_folds_reevaluate(&mut self) {
        let api_key = self.api_keys.folds.trim().to_owned();
        let Some(ref model) = self.folds_task.model else { return };
        let model_id = model.model_id.clone();

        // Snapshot current outcomes for before/after comparison (only
        // set after we've confirmed we're actually going to hit the
        // API — otherwise a bailed-out attempt would clobber the
        // deltas from a previous successful re-eval).
        let current_outcomes: Vec<(String, f64)> = model
            .current
            .outcomes
            .iter()
            .map(|o| (o.label.clone(), o.probability.unwrap_or(0.0)))
            .collect();

        // Build driver state inputs from modified drafts only.
        let changed: Vec<fiftyone_folds::DriverStateInput> = self
            .folds_task
            .draft_drivers
            .iter()
            .filter(|d| d.selected_state != d.original_state)
            .map(|d| fiftyone_folds::DriverStateInput {
                code: d.code.clone(),
                state: d.selected_state.clone(),
            })
            .collect();

        eprintln!(
            "[folds] reeval: model_id={} changed_drivers={}",
            model_id,
            changed.len()
        );

        // Client-side safety net. If the filter produced no diffs we
        // must NOT hit the API — the 51Folds server rejects empty
        // payloads with "Validation failed: No driver states were
        // changed", which ends up in the user's face as a confusing
        // error even though the real issue is that the UI thought
        // something was dirty but the filter disagreed. Bail out with
        // a clear in-app message instead.
        if changed.is_empty() {
            eprintln!(
                "[folds] aborting re-eval: no driver state diffs after filtering \
                 (check for case/whitespace mismatch between state_descriptors \
                 and current.drivers[].state)"
            );
            self.folds_task.error = Some(
                "No driver changes to re-evaluate. Click a different state on at least one driver before clicking Re-evaluate.".to_owned(),
            );
            self.set_status(
                "Re-evaluate ignored: no driver changes detected.",
                StatusKind::Error,
            );
            return;
        }

        // Commit the previous-outcomes snapshot only now that we know
        // we're actually sending a request.
        self.folds_task.previous_outcomes = Some(current_outcomes);

        let (tx, rx) = mpsc::channel();
        self.folds_task.rx = Some(rx);
        self.folds_task.reevaluating = true;
        self.folds_task.in_flight = true;
        self.folds_task.error = None;

        thread::spawn(move || {
            crate::folds::patch_drivers(api_key, model_id, changed, tx);
        });
    }

    /// Kick off a Refresh-Model background call. Re-fetches the full
    /// `ModelResponse` from the server and persists it, so the local
    /// view matches the authoritative server state. User-triggered via
    /// the Refresh button in the 51Folds sidebar summary.
    fn start_folds_refresh(&mut self) {
        let api_key = self.api_keys.folds.trim().to_owned();
        if api_key.is_empty() {
            self.folds_task.refresh_error =
                Some("No 51Folds API key set.".to_owned());
            return;
        }
        let Some(ref model_id) = self.folds_task.model_id.clone() else {
            self.folds_task.refresh_error =
                Some("No model loaded to refresh.".to_owned());
            return;
        };
        // Prevent a second refresh on top of one that's still running.
        if self.folds_task.refresh_in_flight {
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.folds_task.refresh_rx = Some(rx);
        self.folds_task.refresh_in_flight = true;
        self.folds_task.refresh_error = None;

        let db_path = database_path();
        let model_id = model_id.clone();
        thread::spawn(move || {
            crate::folds::refresh_model(api_key, model_id, db_path, tx);
        });
    }

    /// Revert-to-original: read the immutable DB baseline for the
    /// current model, extract its driver states, and PATCH the server
    /// with them. The server re-infers from the pristine input and
    /// returns outcomes matching the build-time snapshot.
    fn start_folds_revert_to_original(&mut self) {
        let Some(inf_id) = self.last_inference_id else {
            self.folds_task.error =
                Some("Can't find the original — no inference linked to this model.".to_owned());
            return;
        };
        let baseline_json = match self.storage.load_folds_response_for_inference(inf_id) {
            Ok(Some(json)) => json,
            Ok(None) => {
                self.folds_task.error = Some(
                    "No baseline snapshot stored locally for this model."
                        .to_owned(),
                );
                return;
            }
            Err(e) => {
                self.folds_task.error =
                    Some(format!("Couldn't read baseline: {e}"));
                return;
            }
        };
        let baseline: fiftyone_folds::ModelResponse =
            match serde_json::from_str(&baseline_json) {
                Ok(m) => m,
                Err(e) => {
                    self.folds_task.error =
                        Some(format!("Stored baseline is unreadable: {e}"));
                    return;
                }
            };
        if baseline.current.drivers.is_empty() {
            self.folds_task.error = Some(
                "Stored baseline has no driver data. Try building a new model."
                    .to_owned(),
            );
            return;
        }

        let api_key = self.api_keys.folds.trim().to_owned();
        if api_key.is_empty() {
            self.folds_task.error = Some("No 51Folds API key set.".to_owned());
            return;
        }
        let Some(model_id) = self.folds_task.model_id.clone() else {
            return;
        };

        // Snapshot current outcomes for deltas.
        let current_outcomes: Vec<(String, f64)> = self
            .folds_task
            .model
            .as_ref()
            .map(|m| {
                m.current
                    .outcomes
                    .iter()
                    .map(|o| (o.label.clone(), o.probability.unwrap_or(0.0)))
                    .collect()
            })
            .unwrap_or_default();
        self.folds_task.previous_outcomes = Some(current_outcomes);

        let drivers: Vec<fiftyone_folds::DriverStateInput> = baseline
            .current
            .drivers
            .iter()
            .map(|d| fiftyone_folds::DriverStateInput {
                code: d.code.clone(),
                state: d.state.clone(),
            })
            .collect();

        let (tx, rx) = mpsc::channel();
        self.folds_task.rx = Some(rx);
        self.folds_task.reevaluating = true;
        self.folds_task.in_flight = true;
        self.folds_task.error = None;

        // Revert uses PUT (update_drivers) rather than PATCH so the
        // server atomically replaces all driver states in one shot.
        // PATCH is partial-merge and has been observed to leave the
        // post-revert inference subtly different from the pristine
        // original; PUT eliminates that drift.
        thread::spawn(move || {
            crate::folds::put_drivers(api_key, model_id, drivers, tx);
        });
    }

    fn poll_folds(&mut self) {
        let was_complete = self.folds_task.is_complete();
        let was_reevaluating = self.folds_task.reevaluating;
        self.folds_task.poll();

        // Auto-switch to the model view when the initial build just
        // completed.
        if !was_complete && self.folds_task.is_complete() {
            self.central_view = CentralView::Model;
            self.model_view = ModelView::Outcome;
        }

        // Re-evaluate just finished successfully — auto-navigate to
        // the Outcome tab so the user sees the updated probabilities.
        if was_reevaluating
            && !self.folds_task.reevaluating
            && self.folds_task.error.is_none()
        {
            self.central_view = CentralView::Model;
            self.model_view = ModelView::Outcome;
            self.reeval_toast_until = Some(
                std::time::Instant::now() + std::time::Duration::from_millis(5000),
            );
            self.set_status(
                "51Folds model re-evaluated with your driver edits.",
                StatusKind::Success,
            );
        }
    }

    /// Resume polling for any 51Folds models that were still `pending` when
    /// the app last shut down. Called once from `App::new`.
    ///
    /// - elapsed > 35 min → mark `undisclosed_failure` immediately
    /// - elapsed > 25 min → still poll, but count as "suspect"
    /// - otherwise → spawn a background polling thread
    ///
    /// The **most recent** pending model (by `created_at`) is routed through
    /// `FoldsTask` so the toolbar spinner and central model view show
    /// build-in-progress. All other pending models use the fire-and-forget
    /// DB-only path.
    fn resume_pending_folds_models(&mut self) {
        let api_key = self.api_keys.folds.trim().to_owned();
        if api_key.is_empty() {
            return;
        }

        let pending = match self.storage.load_pending_folds_models() {
            Ok(rows) => rows,
            Err(e) => {
                eprintln!("warn: failed to load pending folds models: {e:#}");
                return;
            }
        };
        if pending.is_empty() {
            return;
        }

        let now = chrono::Utc::now();
        let mut resumed = 0usize;
        let mut suspect = 0usize;
        let mut marked_failure = 0usize;
        let db_path = database_path();

        // Find the most recent pending model that isn't expired —
        // this one gets routed through FoldsTask for UI visibility.
        let ui_candidate_idx = pending
            .iter()
            .enumerate()
            .rev()
            .find(|(_, r)| {
                (now - r.created_at).num_seconds() < crate::models::FOLDS_UNDISCLOSED_AFTER_SECS
            })
            .map(|(i, _)| i);

        for (i, record) in pending.iter().enumerate() {
            let elapsed_secs = (now - record.created_at).num_seconds();
            if elapsed_secs >= crate::models::FOLDS_UNDISCLOSED_AFTER_SECS {
                if let Err(e) = self.storage.update_folds_model_status(
                    &record.model_id,
                    crate::models::FOLDS_STATUS_UNDISCLOSED_FAILURE,
                    Some(now),
                ) {
                    eprintln!(
                        "warn: failed to mark {} as undisclosed_failure: {e:#}",
                        record.model_id
                    );
                }
                marked_failure += 1;
                continue;
            }

            if record.is_suspect(now) {
                suspect += 1;
            }

            let api_key_c = api_key.clone();
            let db_path_c = db_path.clone();
            let model_id_c = record.model_id.clone();
            let created_at_c = record.created_at;

            if Some(i) == ui_candidate_idx {
                // Route through FoldsTask so the UI shows progress.
                let (tx, rx) = mpsc::channel();
                self.folds_task.reset();
                self.folds_task.model_id = Some(model_id_c.clone());
                self.folds_task.in_flight = true;
                self.folds_task.rx = Some(rx);
                thread::spawn(move || {
                    crate::folds::resume_poll_ui(
                        api_key_c,
                        model_id_c,
                        db_path_c,
                        created_at_c,
                        tx,
                    );
                });
            } else {
                // Background DB-only poll.
                thread::spawn(move || {
                    crate::folds::resume_poll(
                        api_key_c,
                        model_id_c,
                        db_path_c,
                        created_at_c,
                    );
                });
            }
            resumed += 1;
        }

        if resumed > 0 || marked_failure > 0 {
            let mut msg = format!("Resumed polling for {resumed} pending 51Folds model(s)");
            if suspect > 0 {
                msg.push_str(&format!(
                    " ({suspect} suspect, >25 min pending)"
                ));
            }
            if marked_failure > 0 {
                msg.push_str(&format!(
                    "; marked {marked_failure} as undisclosed_failure (>35 min)"
                ));
            }
            msg.push('.');
            self.set_status(&msg, StatusKind::Info);
            eprintln!("{msg}");
        }
    }

    fn reload_inference_history(&mut self) {
        self.inference_history = self
            .storage
            .load_recent_inferences(100)
            .unwrap_or_default();
        self.history_page = 0;
        self.reload_folds_status_map();
    }

    fn reload_folds_status_map(&mut self) {
        self.folds_status_by_inference = self
            .storage
            .load_folds_status_by_inference()
            .unwrap_or_default();
    }

    // -- Phase 2: Report generation --

    fn load_report_inferences(&mut self) {
        let Ok(from) = chrono::NaiveDate::parse_from_str(&self.report_from, "%Y-%m-%d") else {
            self.report_task.error = Some("Invalid 'from' date. Use YYYY-MM-DD.".to_owned());
            return;
        };
        let Ok(to) = chrono::NaiveDate::parse_from_str(&self.report_to, "%Y-%m-%d") else {
            self.report_task.error = Some("Invalid 'to' date. Use YYYY-MM-DD.".to_owned());
            return;
        };
        if from > to {
            self.report_task.error = Some("'From' date must be before or equal to 'To' date.".to_owned());
            return;
        }
        if (to - from).num_days() > 365 * 5 {
            self.report_task.error = Some("Date range cannot exceed 5 years.".to_owned());
            return;
        }
        match self.storage.load_inferences_in_range(from, to) {
            Ok(inferences) => {
                if inferences.is_empty() {
                    self.report_task.error = Some("No inferences found in this date range.".to_owned());
                } else {
                    self.report_task.error = None;
                }
                self.report_inferences = inferences;
                self.reload_folds_status_map();
            }
            Err(err) => {
                self.report_task.error = Some(format!("Failed to load inferences: {err:#}"));
            }
        }
    }

    fn start_report_generation(&mut self) {
        if self.report_task.in_flight {
            return;
        }
        if self.report_inferences.is_empty() {
            self.report_task.error = Some("No inferences found in selected range.".to_owned());
            return;
        }

        let provider = self.settings.ai_provider;
        let api_key = self.api_keys.ai_key_for(provider).trim().to_owned();
        if api_key.is_empty() {
            self.report_task.error = Some(format!(
                "No API key configured for {}.",
                provider.label()
            ));
            return;
        }
        if let Err(e) = validate_model_name(self.settings.effective_model()) {
            self.report_task.error = Some(e);
            return;
        }

        let (system_prompt, user_message) = ai::assemble_report_prompt(
            &self.report_inferences,
            &self.report_from,
            &self.report_to,
        );

        let request = ai::AiRequest {
            provider,
            api_key,
            model: self.settings.effective_model().to_owned(),
            max_tokens: 1024,
            system_prompt,
            user_message,
        };

        let (tx, rx) = mpsc::channel();
        self.report_task.start(rx);
        self.report_result = None;

        thread::spawn(move || {
            ai::run_analysis(request, tx);
        });
    }

    fn poll_report(&mut self) {
        match self.report_task.poll() {
            LlmPoll::Response(result) => {
                // Save the report itself as an inference for historical record.
                // Reports are syntheses across many analyses, so they have no
                // hypothesis fields and no overlay snapshot of their own.
                let _ = self.storage.save_inference(
                    &format!("report:{}", result.provider),
                    &result.model,
                    &result.system_prompt,
                    &result.user_message,
                    &result.response,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                );
                self.report_result = Some(result.response);
                self.reload_inference_history();
            }
            LlmPoll::Failed | LlmPoll::Pending | LlmPoll::Idle => {}
        }
    }

    /// Renders the shared body of the AI analysis panel.
    /// Returns `(close_requested, reanalyze_requested)`.
    fn render_ai_panel_contents(
        &mut self,
        ui: &mut egui::Ui,
        close: &mut bool,
        reanalyze: &mut bool,
    ) {
        ui.horizontal(|ui| {
            let panel_title = if self
                .ai_response_provider
                .as_deref()
                .is_some_and(|p| p.starts_with("dexter"))
            {
                "Research Analysis"
            } else {
                "AI Analysis"
            };
            ui.label(
                RichText::new(panel_title)
                    .strong()
                    .size(11.0)
                    .color(TEXT_SECONDARY),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("\u{2715}").clicked() {
                    *close = true;
                }
                if !self.ai_task.in_flight && ui.small_button("Re-analyze").clicked() {
                    *reanalyze = true;
                }
                let dock_label = match self.settings.ai_panel_dock {
                    AiPanelDock::Bottom => "\u{25B6}",  // ▶ dock right
                    AiPanelDock::Right  => "\u{25BC}",  // ▼ dock bottom
                };
                let dock_tip = match self.settings.ai_panel_dock {
                    AiPanelDock::Bottom => "Dock to right sidebar",
                    AiPanelDock::Right  => "Dock to bottom panel",
                };
                if ui.small_button(dock_label).on_hover_text(dock_tip).clicked() {
                    self.settings.ai_panel_dock = match self.settings.ai_panel_dock {
                        AiPanelDock::Bottom => AiPanelDock::Right,
                        AiPanelDock::Right  => AiPanelDock::Bottom,
                    };
                }
            });
        });
        ui.separator();
        let scroll_out = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.ai_task.in_flight {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new("Calling LLM...")
                                .size(12.0)
                                .color(TEXT_SECONDARY),
                        );
                    });
                } else if let Some(ref err) = self.ai_task.error {
                    ui.label(
                        RichText::new(format!("Error: {err}"))
                            .color(ALERT_EXTREME_FG)
                            .size(12.0),
                    );
                } else if let Some(ref response) = self.ai_response {
                    // Render only the regime portion of the response. The
                    // Hypothesis / Hypothesis Outcomes / Hypothesis Context
                    // sections are intentionally suppressed here because the
                    // 51Folds editor below shows the same content as
                    // editable fields — rendering both would duplicate the
                    // information for the user.
                    let display_text = split_off_hypothesis(response);
                    egui_commonmark::CommonMarkViewer::new()
                        .show(ui, &mut self.ai_markdown_cache, display_text);
                } else {
                    ui.label(
                        RichText::new("Click 'Analyze' to get AI analysis of the current view.")
                            .color(TEXT_MUTED)
                            .size(12.0),
                    );
                }

                // Commodity-bias validation — shown after analysis completes
                if self.ai_response.is_some() && !self.ai_task.in_flight {
                    self.render_bias_validation(ui);
                }

                // 51Folds section — shown after a successful analysis
                if self.ai_response.is_some() && !self.ai_task.in_flight {
                    self.render_folds_section(ui);
                }
            });
        self.ai_panel_content_height = scroll_out.content_size.y;
    }

    /// Render the commodity-bias validation summary in the AI panel.
    fn render_bias_validation(&self, ui: &mut egui::Ui) {
        if self.bias_deterministic.is_empty() {
            return;
        }
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);

        // Combine deterministic + judge results for display.
        let det_pass = self.bias_deterministic.iter().filter(|r| r.pass).count();
        let det_total = self.bias_deterministic.len();

        let judge_provider_label = self
            .bias_judge_provider
            .map(|p| match p {
                LlmProvider::Anthropic => "Claude",
                LlmProvider::OpenAI => "GPT",
            })
            .unwrap_or("");

        let (judge_label, judge_results) = match &self.bias_judge_results {
            None if self.bias_judge_task.in_flight => ("running...", None),
            None => ("not run", None),
            Some(results) => {
                let pass = results.iter().filter(|r| r.pass).count();
                let total = results.len();
                ("", Some((pass, total, results)))
            }
        };

        // Header line: "Bias Check: 3/3 deterministic | 5/5 judge"
        ui.horizontal(|ui| {
            let det_color = if det_pass == det_total {
                ALERT_NORMAL_FG
            } else {
                ALERT_EXTREME_FG
            };
            ui.label(
                RichText::new("Bias Check")
                    .strong()
                    .size(11.0)
                    .color(TEXT_SECONDARY),
            );
            ui.label(
                RichText::new(format!("{det_pass}/{det_total} structural"))
                    .size(11.0)
                    .color(det_color),
            ).on_hover_text(
                "Instant checks on the AI response:\n\
                 \u{2022} Hypothesis names a selected instrument\n\
                 \u{2022} No unselected instrument is the subject\n\
                 \u{2022} Price levels match the latest closes"
            );
            ui.label(
                RichText::new("|")
                    .size(11.0)
                    .color(TEXT_MUTED),
            );
            match judge_results {
                Some((pass, total, _)) => {
                    let judge_color = if pass == total {
                        ALERT_NORMAL_FG
                    } else {
                        ALERT_EXTREME_FG
                    };
                    let label = if judge_provider_label.is_empty() {
                        format!("{pass}/{total} semantic")
                    } else {
                        format!("{pass}/{total} semantic ({judge_provider_label})")
                    };
                    ui.label(
                        RichText::new(label)
                            .size(11.0)
                            .color(judge_color),
                    ).on_hover_text(
                        "LLM judge validates the analysis semantically:\n\
                         \u{2022} Subject match \u{2014} hypothesis is about the right instruments\n\
                         \u{2022} Price anchoring \u{2014} prices from latest data, not training priors\n\
                         \u{2022} Mechanism relevance \u{2014} causal logic specific to selected instruments\n\
                         \u{2022} Tertiary boundary \u{2014} non-selected instruments stay as background\n\
                         \u{2022} Outcome alignment \u{2014} outcomes have distinct causal paths"
                    );
                }
                None => {
                    let label = if self.bias_judge_task.in_flight && !judge_provider_label.is_empty() {
                        format!("semantic via {judge_provider_label}...")
                    } else {
                        format!("semantic {judge_label}")
                    };
                    ui.label(
                        RichText::new(label)
                            .size(11.0)
                            .color(TEXT_MUTED),
                    );
                    if self.bias_judge_task.in_flight {
                        ui.spinner();
                    }
                }
            }
        });

        // Show failures — truncated with ellipsis, full text in tooltip.
        let all_results: Vec<&eval::ResponseValidation> = self
            .bias_deterministic
            .iter()
            .chain(
                self.bias_judge_results
                    .as_ref()
                    .map(|v| v.as_slice())
                    .unwrap_or(&[])
                    .iter(),
            )
            .collect();

        let failures: Vec<&&eval::ResponseValidation> =
            all_results.iter().filter(|r| !r.pass).collect();

        if !failures.is_empty() {
            ui.add_space(2.0);
            for r in failures {
                let full_text = if r.reason.is_empty() {
                    r.rule.clone()
                } else {
                    format!("{}: {}", r.rule, r.reason)
                };
                let truncated = if full_text.len() > 80 {
                    format!("{}...", &full_text[..full_text.floor_char_boundary(77)])
                } else {
                    full_text.clone()
                };
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("x")
                            .size(10.0)
                            .color(ALERT_EXTREME_FG),
                    );
                    ui.label(
                        RichText::new(&truncated)
                            .size(10.0)
                            .color(TEXT_SECONDARY),
                    ).on_hover_text(&full_text);
                });
            }
        }
    }

    fn render_folds_section(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        let has_key = self.api_keys.has_folds();
        let has_hypothesis = self.draft_hypothesis.is_some();

        ui.label(
            RichText::new("51Folds")
                .strong()
                .size(11.0)
                .color(TEXT_SECONDARY),
        );
        ui.add_space(4.0);

        if !has_key {
            ui.label(
                RichText::new("Configure 51Folds key in settings to create a model from this analysis.")
                    .size(11.0)
                    .color(TEXT_MUTED),
            );
            return;
        }

        if !has_hypothesis {
            ui.label(
                RichText::new("No hypothesis in this analysis. Re-analyze to generate one.")
                    .size(11.0)
                    .color(TEXT_MUTED),
            );
            return;
        }

        // ── Background build indicator ────────────────────────────
        // When a model is building in the background but the user is
        // viewing a different (loaded) model, show a small bar to
        // let them switch back to the in-progress view.
        if self.folds_task.in_flight
            && !self.folds_task.reevaluating
            && self.folds_task.is_complete()
        {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new("New model building...")
                        .size(10.0)
                        .color(ACCENT_BLUE),
                );
                if ui.small_button("Show").clicked() {
                    // Clear the loaded model so the central view
                    // reverts to the build-in-progress spinner.
                    self.folds_task.model = None;
                    self.central_view = CentralView::Model;
                    self.model_view = ModelView::Outcome;
                }
            });
            ui.add_space(4.0);
        }

        // ── Post-creation: completed model → compact summary ──────
        if self.folds_task.is_complete() {
            let model_id = self.folds_task.model_id.clone().unwrap_or_default();
            ui.label(
                RichText::new(format!("Model ID: {model_id}   complete"))
                    .size(13.0)
                    .strong()
                    .color(ALERT_NORMAL_FG),
            );
            ui.add_space(8.0);
            // Brief outcome listing — probability on its own line above the
            // label so wrapped outcome text stays cleanly left-aligned.
            if let Some(ref model) = self.folds_task.model {
                for o in &model.current.outcomes {
                    ui.label(
                        RichText::new(format!("{:.1}%", o.probability.unwrap_or(0.0) * 100.0))
                            .size(12.0)
                            .strong()
                            .color(TEXT_PRIMARY),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(&o.label)
                                .size(12.0)
                                .color(TEXT_SECONDARY),
                        )
                        .wrap(),
                    );
                    ui.add_space(6.0);
                }
            }
            ui.add_space(6.0);
            if ui.button("View in 51Folds tab").clicked() {
                self.central_view = CentralView::Model;
                self.model_view = ModelView::Outcome;
            }

            // ── Refresh affordance ─────────────────────────────────
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let label = if self.folds_task.refresh_in_flight {
                    "Refreshing…"
                } else {
                    "Refresh"
                };
                let btn = ui.add_enabled(
                    !self.folds_task.refresh_in_flight,
                    egui::Button::new(
                        RichText::new(label).size(11.0).color(TEXT_SECONDARY),
                    )
                    .fill(Color32::TRANSPARENT)
                    .stroke(egui::Stroke::new(1.0, BORDER))
                    .corner_radius(4.0),
                );
                if btn.on_hover_text(
                    "Pull the latest model state from 51Folds and refresh the local copy."
                ).clicked() {
                    self.start_folds_refresh();
                }
                if self.folds_task.refresh_in_flight {
                    ui.spinner();
                }
            });
            if let Some(ref err) = self.folds_task.refresh_error.clone() {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("Couldn't refresh: {err}"))
                            .size(10.0)
                            .color(ALERT_EXTREME_FG),
                    )
                    .wrap(),
                );
            }

            return;
        }

        // ── Error state ────────────────────────────────────────────
        if let Some(ref err) = self.folds_task.error.clone() {
            ui.label(
                RichText::new(format!("Error: {err}"))
                    .size(12.0)
                    .color(ALERT_EXTREME_FG),
            );
            return;
        }

        // ── In-flight spinner ──────────────────────────────────────
        if self.folds_task.in_flight {
            let model_id = self.folds_task.model_id.clone().unwrap_or_default();
            let label = if self.folds_task.reevaluating {
                "Re-evaluating…".to_owned()
            } else if model_id.is_empty() {
                "Submitting to 51Folds…".to_owned()
            } else {
                format!("Model ID: {model_id}   building…")
            };
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(label)
                        .size(12.0)
                        .color(TEXT_SECONDARY),
                );
            });
            return;
        }

        // ── Pre-creation: editable hypothesis ──────────────────────
        self.render_folds_hypothesis_editor(ui);
    }

    /// Render the editable hypothesis / outcomes / context fields and
    /// the "Create 51Folds Model" button. Shown before model creation.
    fn render_folds_hypothesis_editor(&mut self, ui: &mut egui::Ui) {
        let row_width = ui.available_width();
        let draft = self.draft_hypothesis.as_mut().unwrap();

        ui.label(RichText::new("Hypothesis Statement").size(11.0).color(TEXT_MUTED));
        ui.label(RichText::new("Substantive claim (not a question), time-bounded 7-90 days, explaining mechanism.").size(9.0).color(Color32::from_gray(100)));
        ui.add(
            egui::TextEdit::multiline(&mut draft.question)
                .desired_rows(2)
                .desired_width(row_width),
        );
        ui.add_space(4.0);

        ui.label(RichText::new("Outcomes").size(11.0).color(TEXT_MUTED));
        ui.label(
            RichText::new(
                "Mutually exclusive outcomes representing distinct causal \
paths. Click \"Different outcomes\" below to ask the LLM for a fresh set.",
            )
            .size(9.0)
            .color(Color32::from_gray(100)),
        );
        ui.add_space(4.0);

        let block_indent: f32 = 8.0;
        let block_width = (row_width - block_indent * 2.0).max(80.0);
        let frame_inner_h_margin: f32 = 10.0;
        let inner_text_width = (block_width - frame_inner_h_margin * 2.0).max(60.0);

        for outcome in &draft.outcomes {
            ui.horizontal(|ui| {
                ui.add_space(block_indent);
                egui::Frame::default()
                    .fill(SURFACE)
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin::symmetric(
                        frame_inner_h_margin as i8,
                        6,
                    ))
                    .show(ui, |ui| {
                        ui.set_min_width(inner_text_width);
                        ui.set_max_width(inner_text_width);
                        ui.label(
                            RichText::new(outcome.trim())
                                .size(12.0)
                                .strong()
                                .color(Color32::WHITE),
                        );
                    });
            });
            ui.add_space(3.0);
        }
        ui.add_space(6.0);

        ui.label(
            RichText::new("Context (Narrative ~300 words)")
                .size(11.0)
                .color(TEXT_MUTED),
        );
        ui.label(
            RichText::new(
                "Historical background, mechanism of change, \
confirming/contradicting signals, and why this 7-90 day timeframe matters.",
            )
            .size(9.0)
            .color(Color32::from_gray(100)),
        );
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.add_space(block_indent);
            egui::Frame::default()
                .fill(SURFACE)
                .corner_radius(4.0)
                .inner_margin(egui::Margin::symmetric(frame_inner_h_margin as i8, 8))
                .show(ui, |ui| {
                    ui.set_min_width(inner_text_width);
                    ui.set_max_width(inner_text_width);
                    ui.add(
                        egui::Label::new(
                            RichText::new(draft.context.trim())
                                .size(12.0)
                                .color(TEXT_PRIMARY),
                        )
                        .wrap(),
                    );
                });
        });
        ui.add_space(8.0);

        let mut reroll_clicked = false;
        let mut create_clicked = false;
        ui.horizontal(|ui| {
            if self.outcomes_task.in_flight {
                ui.spinner();
                ui.label(
                    RichText::new("Getting new outcomes…")
                        .size(11.0)
                        .color(TEXT_SECONDARY),
                );
            } else if ui
                .button("↻ Different outcomes")
                .on_hover_text(
                    "Ask the LLM for a fresh set of mutually exclusive \
outcomes for this hypothesis. The statement and context stay unchanged.",
                )
                .clicked()
            {
                reroll_clicked = true;
            }
            ui.add_space(8.0);
            if ui
                .button("→ Create 51Folds Model")
                .on_hover_text(
                    "Submit the hypothesis, outcomes, and context to \
51Folds to generate an Advanced-tier Bayesian model.",
                )
                .clicked()
            {
                create_clicked = true;
            }
        });
        if let Some(err) = self.outcomes_task.error.clone() {
            ui.label(
                RichText::new(format!("⚠ {err}"))
                    .size(10.0)
                    .color(ALERT_EXTREME_FG),
            );
        }
        if reroll_clicked {
            self.start_outcomes_reroll();
        }
        if create_clicked {
            self.start_folds_create();
        }
        ui.add_space(6.0);
    }

    /// Legacy side-panel model results renderer. Now superseded by the
    /// central panel model explorer (render_central_model_view). Kept as
    /// a thin redirect in case any code path still calls it.
    #[allow(dead_code)]
    fn render_folds_model_results(&mut self, ui: &mut egui::Ui) {
        let model_id = self.folds_task.model_id.clone().unwrap_or_default();
        ui.label(
            RichText::new(format!("Model {model_id}"))
                .size(11.0)
                .color(ALERT_NORMAL_FG),
        );
        ui.add_space(6.0);

        // ── Outcome probability bars ───────────────────────────────
        let row_width = ui.available_width();
        // We need the outcomes from the model. Take a clone to avoid
        // holding a borrow on self across the mutable driver rendering.
        let outcomes: Vec<(String, f64)> = self
            .folds_task
            .model
            .as_ref()
            .map(|m| {
                m.current
                    .outcomes
                    .iter()
                    .map(|o| (o.label.clone(), o.probability.unwrap_or(0.0)))
                    .collect()
            })
            .unwrap_or_default();
        let previous = self.folds_task.previous_outcomes.clone();

        for (label, prob) in &outcomes {
            // Outcome label
            ui.label(
                RichText::new(label)
                    .size(11.0)
                    .color(TEXT_PRIMARY),
            );
            // Probability bar
            let bar_height = 20.0;
            let desired = Vec2::new(row_width, bar_height);
            let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
            let painter = ui.painter();

            // Background track
            painter.rect_filled(rect, 4.0, SURFACE);

            // Filled portion — accent blue
            let bar_width = (*prob as f32 * rect.width()).max(0.0);
            let fill_rect = Rect::from_min_size(rect.min, Vec2::new(bar_width, bar_height));
            let bar_color = Color32::from_rgb(59, 130, 246); // blue-500
            painter.rect_filled(fill_rect, 4.0, bar_color);

            // Percentage text right-aligned inside the bar area
            painter.text(
                Pos2::new(rect.right() - 4.0, rect.center().y),
                Align2::RIGHT_CENTER,
                format!("{:.1}%", prob * 100.0),
                FontId::proportional(11.0),
                TEXT_PRIMARY,
            );

            // Before/after delta annotation
            if let Some(ref prev) = previous {
                if let Some((_, prev_prob)) = prev.iter().find(|(l, _)| l == label) {
                    let delta = prob - prev_prob;
                    if delta.abs() > 0.001 {
                        let (text, color) = if delta > 0.0 {
                            (format!("Previously: {:.2}% ↑", prev_prob * 100.0), ALERT_NORMAL_FG)
                        } else {
                            (format!("Previously: {:.2}% ↓", prev_prob * 100.0), ALERT_EXTREME_FG)
                        };
                        ui.label(
                            RichText::new(text)
                                .size(9.0)
                                .color(color),
                        );
                    }
                }
            }
            ui.add_space(4.0);
        }

        // ── Take Away summary ──────────────────────────────────────
        let summary = self
            .folds_task
            .model
            .as_ref()
            .map(|m| m.short_summary.clone())
            .unwrap_or_default();
        if !summary.is_empty() {
            ui.add_space(6.0);
            ui.label(
                RichText::new("Take Away")
                    .size(11.0)
                    .strong()
                    .color(TEXT_SECONDARY),
            );
            ui.add_space(4.0);
            let block_indent: f32 = 8.0;
            let frame_inner_h_margin: f32 = 10.0;
            let inner_text_width = (row_width - block_indent * 2.0 - frame_inner_h_margin * 2.0).max(60.0);
            ui.horizontal(|ui| {
                ui.add_space(block_indent);
                egui::Frame::default()
                    .fill(SURFACE)
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin::symmetric(frame_inner_h_margin as i8, 8))
                    .show(ui, |ui| {
                        ui.set_min_width(inner_text_width);
                        ui.set_max_width(inner_text_width);
                        ui.add(
                            egui::Label::new(
                                RichText::new(&summary)
                                    .size(12.0)
                                    .color(TEXT_PRIMARY),
                            )
                            .wrap(),
                        );
                    });
            });
        }

        // ── Driver list with state selectors ───────────────────────
        ui.add_space(8.0);
        let driver_count = self.folds_task.draft_drivers.len();
        ui.label(
            RichText::new(format!("Drivers ({driver_count})"))
                .size(11.0)
                .strong()
                .color(TEXT_SECONDARY),
        );
        ui.add_space(4.0);

        // Collect driver context/justification data we need while
        // rendering, to avoid holding a borrow on self.folds_task.model
        // across the mutable iteration of draft_drivers.
        let driver_contexts: Vec<Option<fiftyone_folds::DriverContext>> = self
            .folds_task
            .model
            .as_ref()
            .map(|m| {
                self.folds_task
                    .draft_drivers
                    .iter()
                    .map(|d| {
                        m.drivers
                            .iter()
                            .find(|def| def.code == d.code)
                            .and_then(|def| def.context.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();

        let driver_justifications: Vec<Option<fiftyone_folds::DriverJustification>> = self
            .folds_task
            .model
            .as_ref()
            .map(|m| {
                self.folds_task
                    .draft_drivers
                    .iter()
                    .map(|d| {
                        m.current
                            .drivers
                            .iter()
                            .find(|ds| ds.code == d.code)
                            .and_then(|ds| ds.justification.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();

        for (i, draft) in self.folds_task.draft_drivers.iter_mut().enumerate() {
            let is_modified = draft.selected_state != draft.original_state;

            egui::Frame::default()
                .fill(SURFACE)
                .corner_radius(4.0)
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    // Header: expand chevron + driver name + code badge
                    ui.horizontal(|ui| {
                        let icon = if draft.expanded { "▼" } else { "▶" };
                        if ui.small_button(icon).clicked() {
                            draft.expanded = !draft.expanded;
                        }
                        let name_color = if is_modified { ALERT_APPROACHING_FG } else { TEXT_PRIMARY };
                        ui.label(
                            RichText::new(&draft.name)
                                .size(10.0)
                                .strong()
                                .color(name_color),
                        );
                        ui.label(
                            RichText::new(format!("({})", draft.code))
                                .size(9.0)
                                .color(TEXT_MUTED),
                        );
                    });

                    // Segmented state selector
                    ui.horizontal_wrapped(|ui| {
                        for (state_name, _desc) in &draft.state_options {
                            let selected = *state_name == draft.selected_state;
                            let text = RichText::new(state_name).size(10.0);
                            let btn = if selected {
                                egui::Button::new(text.color(Color32::WHITE))
                                    .fill(Color32::from_rgb(59, 130, 246))
                            } else {
                                egui::Button::new(text.color(TEXT_SECONDARY))
                                    .fill(SURFACE_HOVER)
                            };
                            if ui.add(btn).clicked() {
                                draft.selected_state = state_name.clone();
                            }
                        }
                    });

                    // Expandable details
                    if draft.expanded {
                        ui.add_space(4.0);

                        // State descriptors
                        ui.label(
                            RichText::new("State descriptions")
                                .size(10.0)
                                .strong()
                                .color(TEXT_SECONDARY),
                        );
                        for (sn, sd) in &draft.state_options {
                            let highlight = *sn == draft.selected_state;
                            let color = if highlight { TEXT_PRIMARY } else { TEXT_MUTED };
                            ui.label(
                                RichText::new(format!("  {sn}: {sd}"))
                                    .size(10.0)
                                    .color(color),
                            );
                        }

                        // Justification ("Why was X selected?")
                        if let Some(Some(just)) = driver_justifications.get(i) {
                            if !just.content.is_empty() {
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(format!(
                                        "Why was {} selected?",
                                        draft.original_state
                                    ))
                                    .size(10.0)
                                    .strong()
                                    .color(TEXT_SECONDARY),
                                );
                                for line in &just.content {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(line)
                                                .size(10.0)
                                                .color(TEXT_SECONDARY),
                                        )
                                        .wrap(),
                                    );
                                }
                                if !just.citations.is_empty() {
                                    ui.add_space(2.0);
                                    for cite in &just.citations {
                                        ui.label(
                                            RichText::new(format!("[{}] {}", cite.num, cite.source))
                                                .size(9.0)
                                                .color(TEXT_MUTED),
                                        );
                                    }
                                }
                            }
                        }

                        // Context sections (importance, shifts, monitor)
                        if let Some(Some(ctx)) = driver_contexts.get(i) {
                            if !ctx.importance.is_empty() {
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new("Why does this matter?")
                                        .size(10.0)
                                        .strong()
                                        .color(TEXT_SECONDARY),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&ctx.importance)
                                            .size(10.0)
                                            .color(TEXT_SECONDARY),
                                    )
                                    .wrap(),
                                );
                            }
                            if !ctx.shifts.is_empty() {
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new("What could shift?")
                                        .size(10.0)
                                        .strong()
                                        .color(TEXT_SECONDARY),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&ctx.shifts)
                                            .size(10.0)
                                            .color(TEXT_SECONDARY),
                                    )
                                    .wrap(),
                                );
                            }
                            if !ctx.monitor.is_empty() {
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new("What should we monitor?")
                                        .size(10.0)
                                        .strong()
                                        .color(TEXT_SECONDARY),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&ctx.monitor)
                                            .size(10.0)
                                            .color(TEXT_SECONDARY),
                                    )
                                    .wrap(),
                                );
                            }
                        }
                    }
                });
            ui.add_space(3.0);
        }

        // ── Re-evaluate & Reset buttons ────────────────────────────
        ui.add_space(4.0);
        let any_modified = self
            .folds_task
            .draft_drivers
            .iter()
            .any(|d| d.selected_state != d.original_state);
        let mut reevaluate_clicked = false;
        let mut reset_clicked = false;

        ui.horizontal(|ui| {
            let btn = ui.add_enabled(any_modified, egui::Button::new("Re-evaluate"));
            if btn.clicked() {
                reevaluate_clicked = true;
            }
            ui.add_space(8.0);
            let reset_btn = ui.add_enabled(any_modified, egui::Button::new("Reset"));
            if reset_btn.clicked() {
                reset_clicked = true;
            }
        });

        if reset_clicked {
            for d in &mut self.folds_task.draft_drivers {
                d.selected_state = d.original_state.clone();
            }
            self.folds_task.previous_outcomes = None;
        }
        if reevaluate_clicked {
            self.start_folds_reevaluate();
        }
        ui.add_space(6.0);
    }

    /// Render the 51Folds model explorer in the central panel.
    fn render_research_agent_panel(&mut self, ui: &mut egui::Ui) {
        // Lazy bun check (once).
        if self.research_terminal.bun_available.is_none() {
            let has_bun = std::process::Command::new("bun")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            self.research_terminal.bun_available = Some(has_bun);
        }

        if self.research_terminal.bun_available == Some(false) {
            ui.vertical_centered(|ui| {
                ui.add_space(80.0);
                ui.label(
                    RichText::new("Bun runtime not found")
                        .size(18.0)
                        .color(ALERT_APPROACHING_FG)
                        .strong(),
                );
                ui.add_space(12.0);
                ui.label(
                    RichText::new(
                        "The Research Agent requires Bun to run.\n\
                         Install it with: curl -fsSL https://bun.com/install | bash",
                    )
                    .size(13.0)
                    .color(TEXT_SECONDARY),
                );
                ui.add_space(12.0);
                if ui.button("Re-check").clicked() {
                    self.research_terminal.bun_available = None;
                }
            });
            return;
        }

        // Lazy terminal spawn on first visit.
        if self.research_terminal.backend.is_none()
            && self.research_terminal.child_exited.is_none()
            && self.research_terminal.error.is_none()
        {
            self.spawn_research_terminal();
        }

        // Error state.
        if let Some(ref err) = self.research_terminal.error {
            let err = err.clone();
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(
                    RichText::new("Research Agent Error")
                        .size(18.0)
                        .color(ALERT_EXTREME_FG)
                        .strong(),
                );
                ui.add_space(8.0);
                ui.label(RichText::new(&err).size(13.0).color(TEXT_SECONDARY));
                ui.add_space(12.0);
                if ui.button("Retry").clicked() {
                    self.spawn_research_terminal();
                }
            });
            return;
        }

        // Child exited state.
        if let Some(code) = self.research_terminal.child_exited {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                let msg = if code == 0 {
                    "Research Agent exited normally."
                } else {
                    "Research Agent exited unexpectedly."
                };
                ui.label(
                    RichText::new(msg)
                        .size(16.0)
                        .color(if code == 0 {
                            ALERT_NORMAL_FG
                        } else {
                            ALERT_APPROACHING_FG
                        })
                        .strong(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("Exit code: {code}"))
                        .size(12.0)
                        .color(TEXT_MUTED),
                );
                ui.add_space(12.0);
                if ui.button("Restart").clicked() {
                    self.spawn_research_terminal();
                }
            });
            return;
        }

        // Render terminal widget. Skip when the Report window is open
        // because egui_term renders at a z-level that covers egui::Window.
        if self.show_help {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Terminal paused while dialog is open")
                        .size(12.0)
                        .color(TEXT_MUTED),
                );
            });
            return;
        }

        // Render the terminal. The central panel already has 16px left
        // inner_margin, but the first-column circle glyphs extend slightly
        // left of their character cell. We can't add more padding without
        // breaking the terminal's height allocation, so we accept the
        // minor left-edge clip — it's a limitation of egui_term rendering
        // character cells flush to the allocated rect boundary.
        if let Some(ref mut backend) = self.research_terminal.backend {
            let available = ui.available_size();
            let terminal_view = egui_term::TerminalView::new(ui, backend)
                .set_focus(true)
                .set_size(available)
                .set_theme(hedgehog_terminal_theme());
            ui.add(terminal_view);
        }
    }

    /// Summary Report rendered inline in the central panel.
    fn render_central_report_view(&mut self, ui: &mut egui::Ui) {
        let max_width = 720.0_f32;
        let available_w = ui.available_width();
        let side_pad = ((available_w - max_width) / 2.0).max(0.0);

        let mut start_report = false;
        let mut load_inferences = false;
        let mut load_history: Option<SavedInference> = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(20.0);

                // Centered column
                ui.horizontal(|ui| {
                    ui.add_space(side_pad);
                    ui.vertical(|ui| {
                        ui.set_max_width(max_width);

                        ui.label(
                            RichText::new("Summary Report")
                                .size(22.0)
                                .strong()
                                .color(Color32::WHITE),
                        );
                        ui.add_space(14.0);

                        // Date range controls
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("From:")
                                    .size(13.0)
                                    .color(TEXT_SECONDARY),
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut self.report_from)
                                    .desired_width(110.0)
                                    .hint_text("YYYY-MM-DD"),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("To:")
                                    .size(13.0)
                                    .color(TEXT_SECONDARY),
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut self.report_to)
                                    .desired_width(110.0)
                                    .hint_text("YYYY-MM-DD"),
                            );
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let today = chrono::Utc::now().date_naive();
                            if ui.small_button("Last 7 days").clicked() {
                                self.report_from =
                                    (today - chrono::Duration::days(7)).format("%Y-%m-%d").to_string();
                                self.report_to = today.format("%Y-%m-%d").to_string();
                            }
                            if ui.small_button("Last 30 days").clicked() {
                                self.report_from =
                                    (today - chrono::Duration::days(30)).format("%Y-%m-%d").to_string();
                                self.report_to = today.format("%Y-%m-%d").to_string();
                            }
                            if ui.small_button("Last 90 days").clicked() {
                                self.report_from =
                                    (today - chrono::Duration::days(90)).format("%Y-%m-%d").to_string();
                                self.report_to = today.format("%Y-%m-%d").to_string();
                            }
                            if ui.small_button("All").clicked() {
                                self.report_from = "2020-01-01".to_owned();
                                self.report_to = today.format("%Y-%m-%d").to_string();
                            }
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("Load Inferences").clicked() {
                                load_inferences = true;
                            }
                            if !self.report_inferences.is_empty() {
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(format!(
                                        "{} inferences loaded",
                                        self.report_inferences.len()
                                    ))
                                    .size(13.0)
                                    .color(TEXT_SECONDARY),
                                );
                                if self.report_task.in_flight {
                                    ui.spinner();
                                } else {
                                    ui.add_space(8.0);
                                    if ui.button("Generate Report").clicked() {
                                        start_report = true;
                                    }
                                }
                            }
                        });

                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);

                        // Error
                        if let Some(ref err) = self.report_task.error {
                            ui.label(
                                RichText::new(err)
                                    .color(ALERT_EXTREME_FG)
                                    .size(14.0),
                            );
                            ui.add_space(8.0);
                        }

                        // In-flight
                        if self.report_task.in_flight {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(
                                    RichText::new("Generating report...")
                                        .size(14.0)
                                        .color(TEXT_SECONDARY),
                                );
                            });
                        } else if let Some(ref report) = self.report_result {
                            // Render the generated report as markdown
                            let v = ui.visuals_mut();
                            v.override_text_color = Some(TEXT_PRIMARY);
                            v.widgets.noninteractive.fg_stroke =
                                egui::Stroke::new(1.0, TEXT_PRIMARY);
                            v.widgets.active.fg_stroke =
                                egui::Stroke::new(1.0, Color32::WHITE);
                            egui_commonmark::CommonMarkViewer::new()
                                .show(ui, &mut self.report_markdown_cache, report);
                        } else if !self.report_inferences.is_empty() {
                            // Browsable inference list
                            ui.label(
                                RichText::new("Loaded Inferences")
                                    .strong()
                                    .size(14.0)
                                    .color(TEXT_SECONDARY),
                            );
                            ui.add_space(6.0);
                            // Truncation target: fit within the max column width
                            // minus the dot + badge. Use word-boundary truncation.
                            let label_max_chars = 90;

                            for inf in &self.report_inferences {
                                let level_color = match inf.vix_level.as_deref() {
                                    Some("extreme") => ALERT_EXTREME_FG,
                                    Some("approaching_extreme") => ALERT_APPROACHING_FG,
                                    _ => ALERT_NORMAL_FG,
                                };
                                let full_label = inference_label(inf);
                                let folds_badge = self.folds_status_by_inference.get(&inf.id);

                                // Word-boundary truncation with ellipsis
                                let display_label = if full_label.chars().count() > label_max_chars {
                                    let truncated: String = full_label.chars().take(label_max_chars).collect();
                                    // Find last space for word boundary
                                    if let Some(last_space) = truncated.rfind(' ') {
                                        format!("{}...", &truncated[..last_space])
                                    } else {
                                        format!("{truncated}...")
                                    }
                                } else {
                                    full_label.clone()
                                };

                                // Full row as a clickable area with hover highlight
                                let row_resp = ui.horizontal(|ui| {
                                    let (dot_rect, _) = ui.allocate_exact_size(
                                        Vec2::new(12.0, 12.0),
                                        Sense::hover(),
                                    );
                                    ui.painter().circle_filled(
                                        dot_rect.center(),
                                        6.0,
                                        level_color,
                                    );
                                    ui.add_space(6.0);
                                    // Fixed-width 51Folds model column
                                    let (badge_text, badge_color, badge_tip) = match folds_badge {
                                        Some((mid, status)) => {
                                            let tip = format!("51Folds model {mid} — {status}");
                                            match status.as_str() {
                                                "success" => (mid.clone(), ALERT_NORMAL_FG, tip),
                                                "pending" => (format!("{mid}.."), ACCENT_BLUE, tip),
                                                _ => (format!("{mid}!"), ALERT_EXTREME_FG, tip),
                                            }
                                        }
                                        None => ("\u{2014}".to_owned(), TEXT_MUTED, "No 51Folds model".to_owned()),
                                    };
                                    let badge_resp = ui.allocate_ui_with_layout(
                                        Vec2::new(40.0, 18.0),
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(&badge_text)
                                                    .size(13.0)
                                                    .color(badge_color),
                                            );
                                        },
                                    );
                                    badge_resp.response.on_hover_text(&badge_tip);
                                    ui.label(
                                        RichText::new(&display_label)
                                            .size(13.0)
                                            .color(TEXT_PRIMARY),
                                    );
                                });
                                // Make the entire row clickable with hover feedback
                                let row_rect = row_resp.response.rect;
                                let row_sense = ui.allocate_rect(row_rect, Sense::click());
                                if row_sense.hovered() {
                                    ui.painter().rect_filled(
                                        row_rect,
                                        4.0,
                                        Color32::from_rgba_unmultiplied(255, 255, 255, 8),
                                    );
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                if row_sense.clicked() {
                                    load_history = Some(inf.clone());
                                }
                                if display_label != full_label {
                                    row_sense.on_hover_text(&full_label);
                                }
                            }
                        }
                    });
                });
            });

        if load_inferences {
            self.load_report_inferences();
        }
        if start_report {
            self.start_report_generation();
        }
        if let Some(inf) = load_history {
            self.load_historical_inference(inf);
        }
    }

    fn render_central_model_view(&mut self, ui: &mut egui::Ui) {
        if !self.folds_task.is_complete() {
            // ── Build in progress — show spinner + model ID ───────
            if self.folds_task.in_flight {
                ui.add_space(80.0);
                ui.vertical_centered(|ui| {
                    ui.spinner();
                    ui.add_space(10.0);
                    let model_id = self.folds_task.model_id.clone().unwrap_or_default();
                    let label = if self.folds_task.reevaluating {
                        "Re-evaluating model\u{2026}".to_owned()
                    } else if model_id.is_empty() {
                        "Submitting to 51Folds\u{2026}".to_owned()
                    } else {
                        format!("Building model {model_id}\u{2026}")
                    };
                    ui.label(
                        RichText::new(label)
                            .size(16.0)
                            .color(Color32::WHITE),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            "This typically takes 25\u{2013}30 minutes. \
                             You can navigate away \u{2014} the build continues in the background.",
                        )
                        .size(13.0)
                        .color(TEXT_SECONDARY),
                    );
                });
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }

            // Two empty states: genuinely no model loaded, or we have
            // a `model_id` in memory but the model itself is stub /
            // unloadable. The second case is a recovery scenario —
            // show a Refresh button so the user can pull a fresh copy
            // from the server without needing to rebuild.
            let recoverable_id =
                self.folds_task.model_id.clone().filter(|s| !s.is_empty());
            ui.add_space(80.0);
            ui.vertical_centered(|ui| {
                if let Some(id) = recoverable_id {
                    if self.folds_task.refresh_in_flight {
                        // Auto-refresh is running — show a clear "fetching"
                        // state so the user knows the app isn't stuck.
                        ui.spinner();
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("Reloading model from 51Folds…")
                                .size(16.0)
                                .color(Color32::WHITE),
                        );
                        ui.add_space(6.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!(
                                    "Pulling the latest state for model {id}.",
                                ))
                                .size(13.0)
                                .color(TEXT_SECONDARY),
                            )
                            .wrap(),
                        );
                        ui.ctx().request_repaint_after(
                            std::time::Duration::from_millis(100),
                        );
                    } else {
                        ui.label(
                            RichText::new("Model data is incomplete")
                                .size(18.0)
                                .color(ALERT_APPROACHING_FG),
                        );
                        ui.add_space(8.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!(
                                    "Model {id} is tracked locally but the stored copy looks corrupt. Pull a fresh copy from 51Folds to recover.",
                                ))
                                .size(13.0)
                                .color(TEXT_SECONDARY),
                            )
                            .wrap(),
                        );
                        ui.add_space(14.0);
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Reload model from 51Folds")
                                        .size(13.0)
                                        .strong()
                                        .color(Color32::WHITE),
                                )
                                .fill(ACCENT_BLUE_DIM)
                                .corner_radius(6.0),
                            )
                            .clicked()
                        {
                            self.start_folds_refresh();
                        }
                        if let Some(err) = self.folds_task.refresh_error.clone() {
                            ui.add_space(10.0);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!("Couldn't refresh: {err}"))
                                        .size(11.0)
                                        .color(ALERT_EXTREME_FG),
                                )
                                .wrap(),
                            );
                        }
                    }
                } else {
                    ui.label(
                        RichText::new("No model loaded")
                            .size(18.0)
                            .color(TEXT_MUTED),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Run an AI analysis and create a 51Folds model to see results here.")
                            .size(14.0)
                            .color(TEXT_MUTED),
                    );
                }
            });
            return;
        }

        ui.add_space(12.0);

        // ── Re-evaluation status & error banners ───────────────────
        // Rendered above the question so they're the first thing the
        // user sees regardless of which sub-view they're on. The
        // reevaluating banner is the primary "system is working" cue;
        // the error banner is the retry cue on failure.
        if self.folds_task.reevaluating {
            render_reeval_in_flight_banner(ui);
            // Keep the spinner animation alive.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(80));
            ui.add_space(14.0);
        } else if let Some(ref err) = self.folds_task.error.clone() {
            render_reeval_error_banner(ui, err);
            ui.add_space(14.0);
        }

        // Question as the primary heading — large, bold, white.
        let question = self
            .folds_task
            .model
            .as_ref()
            .map(|m| m.question.clone())
            .unwrap_or_default();
        if !question.is_empty() {
            ui.add(
                egui::Label::new(
                    RichText::new(&question)
                        .size(22.0)
                        .strong()
                        .color(Color32::WHITE),
                )
                .wrap(),
            );
            ui.add_space(6.0);
        }

        // Timestamps + Refresh-from-server affordance, on one row so
        // the user can always see when the model was last updated and
        // pull a fresh copy without hunting for buttons in other panels.
        ui.horizontal(|ui| {
            if let Some(ref model) = self.folds_task.model {
                ui.label(
                    RichText::new(format!(
                        "Created {} \u{00B7} Last updated {}",
                        &model.created_at.get(..16).unwrap_or(&model.created_at),
                        &model.updated_at.get(..16).unwrap_or(&model.updated_at),
                    ))
                    .size(12.0)
                    .color(TEXT_SECONDARY),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if self.folds_task.refresh_in_flight {
                    "Refreshing…"
                } else {
                    "\u{21BB} Refresh from 51Folds"
                };
                let resp = ui.add_enabled(
                    !self.folds_task.refresh_in_flight && !self.folds_task.reevaluating,
                    egui::Button::new(
                        RichText::new(label).size(11.0).color(ACCENT_BLUE),
                    )
                    .fill(Color32::TRANSPARENT)
                    .stroke(egui::Stroke::new(1.0, BORDER))
                    .corner_radius(4.0),
                );
                if resp
                    .on_hover_text(
                        "Pull the latest model state from 51Folds and overwrite the local copy. Use this if the pills don't match what the server has.",
                    )
                    .clicked()
                {
                    self.start_folds_refresh();
                }
                if self.folds_task.refresh_in_flight {
                    ui.spinner();
                }
            });
        });

        if let Some(ref err) = self.folds_task.refresh_error.clone() {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("Couldn't refresh: {err}"))
                        .size(11.0)
                        .color(ALERT_EXTREME_FG),
                )
                .wrap(),
            );
        }

        ui.add_space(18.0);

        // The Visual Map (DAG webview) fills the panel directly — it must
        // NOT be inside a ScrollArea because the webview handles its own
        // pan/zoom. All other views scroll normally.
        if self.model_view == ModelView::VisualMap {
            self.render_dag_view(ui);
        } else {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    match self.model_view {
                        ModelView::Outcome => {
                            self.render_central_outcome_tab(ui);
                        }
                        ModelView::DriverList => {
                            self.render_central_drivers_tab(ui);
                        }
                        ModelView::DriverDetail(idx) => {
                            self.render_driver_detail_page(ui, idx);
                        }
                        ModelView::DriverSection(idx, section) => {
                            self.render_driver_section_page(ui, idx, section);
                        }
                        ModelView::VisualMap => {} // handled above
                    }
                });
        }
    }

    /// Outcome view: probability bars + take away, rendered as dark
    /// cards with high-contrast typography.
    /// Render the DAG visualization via the embedded wry WebView.
    fn render_dag_view(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_rect_before_wrap();
        // Allocate the space in egui so it knows the area is used.
        ui.allocate_rect(available, egui::Sense::hover());

        // Convert egui logical rect to wry bounds (logical coordinates).
        let bounds = wry::Rect {
            position: wry::dpi::Position::Logical(wry::dpi::LogicalPosition::new(
                available.left() as f64,
                available.top() as f64,
            )),
            size: wry::dpi::Size::Logical(wry::dpi::LogicalSize::new(
                available.width() as f64,
                available.height() as f64,
            )),
        };

        // Lazy-create the webview on first visit.
        if !self.dag_webview.is_created() {
            if let Some(ref handle) = self.stored_window_handle {
                self.dag_webview.create(handle, bounds);
            } else {
                ui.label(
                    RichText::new("Cannot create Visual Map: no window handle available.")
                        .color(ALERT_EXTREME_FG)
                        .size(12.0),
                );
                return;
            }
        }

        if let Some(ref err) = self.dag_webview.error {
            ui.label(
                RichText::new(format!("Visual Map error: {err}"))
                    .color(ALERT_EXTREME_FG)
                    .size(12.0),
            );
            return;
        }

        // Reposition every frame.
        self.dag_webview.set_bounds(bounds);
        self.dag_webview.set_visible(true);

        // Send model data if we have a model and the data hasn't been sent yet
        // (or has changed due to re-evaluation).
        if let Some(ref model) = self.folds_task.model {
            // Build a generation from the draft driver states so we detect
            // changes (pill edits, re-eval completion).
            let mut hasher = 0u64;
            for d in &self.folds_task.draft_drivers {
                for b in d.selected_state.bytes() {
                    hasher = hasher.wrapping_mul(31).wrapping_add(b as u64);
                }
            }
            let generation = hasher;

            if generation != self.dag_model_generation {
                let driver_data: Vec<(String, String, String)> = self
                    .folds_task
                    .draft_drivers
                    .iter()
                    .map(|d| (d.code.clone(), d.name.clone(), d.selected_state.clone()))
                    .collect();
                self.dag_webview
                    .send_model_data(model, &driver_data, generation);
                self.dag_model_generation = generation;
            }
        }

        // Poll IPC — handle node clicks.
        while let Some(msg) = self.dag_webview.poll_ipc() {
            match msg {
                crate::dag::DagIpcMessage::NodeClicked { code } => {
                    if let Some(idx) = self
                        .folds_task
                        .draft_drivers
                        .iter()
                        .position(|d| d.code == code)
                    {
                        self.model_view_back = ModelView::VisualMap;
                        self.model_view = ModelView::DriverDetail(idx);
                    }
                }
                crate::dag::DagIpcMessage::Ready => {
                    self.dag_webview.mark_ready();
                }
            }
        }
    }

    fn render_central_outcome_tab(&mut self, ui: &mut egui::Ui) {
        // ── Success toast (if a re-eval just completed) ────────────
        // Fades over the last 800 ms of its display window. Clickable
        // to dismiss early. Auto-clears once expired.
        if let Some(until) = self.reeval_toast_until {
            let now = std::time::Instant::now();
            if now >= until {
                self.reeval_toast_until = None;
            } else {
                const FADE_OUT_WINDOW_MS: u128 = 800;
                let remaining = until.saturating_duration_since(now).as_millis();
                let fade_out = if remaining < FADE_OUT_WINDOW_MS {
                    1.0 - (remaining as f32 / FADE_OUT_WINDOW_MS as f32)
                } else {
                    0.0
                };
                render_reeval_success_toast(ui, fade_out);
                // Click-to-dismiss: any pointer click while the toast
                // is visible jumps straight to the end of its window.
                if ui.ctx().input(|i| i.pointer.any_click()) {
                    self.reeval_toast_until = None;
                }
                // Keep repainting so the fade animates.
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(32));
                ui.add_space(14.0);
            }
        }

        let outcomes: Vec<(String, f64)> = self
            .folds_task
            .model
            .as_ref()
            .map(|m| {
                m.current
                    .outcomes
                    .iter()
                    .map(|o| (o.label.clone(), o.probability.unwrap_or(0.0)))
                    .collect()
            })
            .unwrap_or_default();
        let previous = self.folds_task.previous_outcomes.clone();

        // ── Outcomes card ────────────────────────────────────────────
        section_card(ui, |ui| {
            ui.label(
                RichText::new("OUTCOME PROBABILITIES")
                    .size(12.0)
                    .strong()
                    .color(TEXT_SECONDARY),
            );
            ui.add_space(14.0);

            // Layout constants. Every row is painted by hand into a
            // single full-width allocated rect — no nested horizontal
            // layouts, no column-width arithmetic. This is the only way
            // to guarantee that the percentage text lands exactly at
            // the card's inner right edge regardless of how egui's
            // `available_width` resolves for a wrapping label column.
            let bar_max_width = 260.0_f32;
            let pct_width = 62.0_f32;
            let bar_to_pct_gap = 12.0_f32;
            let label_to_bar_gap = 16.0_f32;
            // Track colour — visibly lighter than the card's SURFACE
            // so the unfilled portion reads as "here's how much further
            // this bar could go" instead of blending into the card
            // background. Was `rgb(31,41,55)`, which was barely
            // distinguishable from `SURFACE rgb(26,34,54)`.
            let track_color = Color32::from_rgb(55, 67, 92);
            let font = FontId::new(16.0, egui::FontFamily::Proportional);

            for (i, (label, prob)) in outcomes.iter().enumerate() {
                let avail_w = ui.available_width();
                let label_max_w = (avail_w
                    - pct_width
                    - bar_to_pct_gap
                    - bar_max_width
                    - label_to_bar_gap)
                    .max(120.0);

                // Pre-layout the label galley so we know how tall the
                // row needs to be (wrapped labels take two lines).
                let label_galley = ui.fonts(|f| {
                    f.layout(
                        label.clone(),
                        font.clone(),
                        Color32::WHITE,
                        label_max_w,
                    )
                });
                let row_height = label_galley.size().y.max(28.0);

                // Allocate the full row in one shot. `row_rect.right()`
                // is guaranteed to be the section_card's inner right
                // edge — that's what the outer `ui.available_width()`
                // call resolves to — so everything we anchor from it
                // lines up to the same pixel across rows.
                let (row_rect, _) =
                    ui.allocate_exact_size(Vec2::new(avail_w, row_height), Sense::hover());

                // ── Child positions, right-anchored ────────────
                // Everything is computed from the card's inner right
                // edge (`row_rect.right()`). `pct_width` reserves room
                // for the widest possible percentage ("100.0%") so the
                // bar's right edge sits cleanly left of the digits
                // with `bar_to_pct_gap` of breathing room.
                let pct_right = row_rect.right();
                let bar_right = pct_right - pct_width - bar_to_pct_gap;
                let bar_left = bar_right - bar_max_width;
                let bar_y_center = row_rect.center().y;
                let bar_top = bar_y_center - 7.0;

                // Label at the left edge, vertically centered in the row.
                let label_y = bar_y_center - label_galley.size().y / 2.0;
                ui.painter().galley(
                    Pos2::new(row_rect.left(), label_y),
                    label_galley,
                    Color32::WHITE,
                );

                // Bar track.
                let bar_rect = Rect::from_min_size(
                    Pos2::new(bar_left, bar_top),
                    Vec2::new(bar_max_width, 14.0),
                );
                ui.painter().rect_filled(bar_rect, 5.0, track_color);

                // Bar fill — right-anchored so the bar grows leftward
                // from the percentage, matching the label on the
                // opposite side for symmetry.
                let fill_width = (*prob as f32 * bar_max_width).max(2.0);
                let fill_rect = Rect::from_min_size(
                    Pos2::new(bar_right - fill_width, bar_top),
                    Vec2::new(fill_width, 14.0),
                );
                ui.painter().rect_filled(fill_rect, 5.0, ACCENT_BLUE);

                // Percentage painted with an explicit right-center
                // anchor at the card's inner right edge.
                ui.painter().text(
                    Pos2::new(pct_right, bar_y_center),
                    Align2::RIGHT_CENTER,
                    format!("{:.1}%", prob * 100.0),
                    font.clone(),
                    Color32::WHITE,
                );

                // Delta annotation (after re-evaluate) — rendered as a
                // separate row directly below, aligned under the label.
                if let Some(ref prev) = previous {
                    if let Some((_, prev_prob)) = prev.iter().find(|(l, _)| l == label) {
                        let delta = prob - prev_prob;
                        if delta.abs() > 0.001 {
                            let (text, color) = if delta > 0.0 {
                                (
                                    format!(
                                        "\u{2191} up from {:.1}%",
                                        prev_prob * 100.0
                                    ),
                                    ALERT_NORMAL_FG,
                                )
                            } else {
                                (
                                    format!(
                                        "\u{2193} down from {:.1}%",
                                        prev_prob * 100.0
                                    ),
                                    ALERT_EXTREME_FG,
                                )
                            };
                            ui.add_space(2.0);
                            ui.label(RichText::new(text).size(12.0).color(color));
                        }
                    }
                }

                if i + 1 < outcomes.len() {
                    ui.add_space(12.0);
                }
            }
        });

        // ── Take-away card ───────────────────────────────────────────
        let summary = self
            .folds_task
            .model
            .as_ref()
            .map(|m| m.short_summary.clone())
            .unwrap_or_default();
        if !summary.is_empty() {
            ui.add_space(16.0);
            section_card(ui, |ui| {
                ui.label(
                    RichText::new("TAKE AWAY")
                        .size(12.0)
                        .strong()
                        .color(TEXT_SECONDARY),
                );
                ui.add_space(10.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(&summary)
                            .size(15.0)
                            .color(TEXT_PRIMARY),
                    )
                    .wrap(),
                );
            });
        }

        // ── Call-to-action ───────────────────────────────────────────
        ui.add_space(22.0);
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Want to fine-tune the drivers to see how the prediction changes?",
                )
                .size(14.0)
                .color(TEXT_SECONDARY),
            )
            .wrap(),
        );
        ui.add_space(12.0);
        ui.scope(|ui| {
            ui.spacing_mut().button_padding = Vec2::new(16.0, 10.0);
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("Show me the drivers  \u{276F}")
                            .size(14.0)
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .fill(ACCENT_BLUE_DIM)
                    .corner_radius(6.0),
                )
                .clicked()
            {
                self.model_view = ModelView::DriverList;
            }
        });
    }

    /// Driver list: each driver in its own dark card with name,
    /// pill selector, and navigation chevron.
    fn render_central_drivers_tab(&mut self, ui: &mut egui::Ui) {
        // Snapshot the re-eval flag up-front so we can disable inputs
        // for the whole view without racing with `start_folds_reevaluate`
        // (which flips the flag synchronously on button click).
        let reevaluating = self.folds_task.reevaluating;
        let mut navigate_to: Option<usize> = None;

        for (i, draft) in self.folds_task.draft_drivers.iter_mut().enumerate() {
            let is_modified = draft.selected_state != draft.original_state;
            let name_color = if is_modified {
                ALERT_APPROACHING_FG
            } else {
                Color32::WHITE
            };

            section_card(ui, |ui| {
                // Top row: driver name (left) + chevron (right).
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{} ({})", &draft.name, &draft.code))
                                .size(17.0)
                                .strong()
                                .color(name_color),
                        )
                        .wrap(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().button_padding = Vec2::new(12.0, 7.0);
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Details  \u{276F}")
                                        .size(13.0)
                                        .color(ACCENT_BLUE),
                                )
                                .fill(Color32::TRANSPARENT)
                                .stroke(egui::Stroke::new(1.0, BORDER))
                                .corner_radius(6.0),
                            )
                            .clicked()
                        {
                            navigate_to = Some(i);
                        }
                    });
                });

                ui.add_space(12.0);

                // Pill selector row — generous padding so labels breathe.
                // During a re-eval the entire row is disabled so the
                // user can't race the server with more edits (Nielsen
                // heuristic #5, error prevention).
                ui.add_enabled_ui(!reevaluating, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().button_padding = Vec2::new(14.0, 8.0);
                        ui.spacing_mut().item_spacing.x = 8.0;
                        for (state_name, _desc) in &draft.state_options {
                            let selected = *state_name == draft.selected_state;
                            let text = RichText::new(state_name).size(13.0).strong();
                            let btn = if selected {
                                egui::Button::new(text.color(Color32::WHITE))
                                    .fill(ACCENT_BLUE_DIM)
                                    .stroke(egui::Stroke::new(1.0, ACCENT_BLUE))
                                    .corner_radius(16.0)
                                    .min_size(Vec2::new(0.0, 30.0))
                            } else {
                                egui::Button::new(text.color(TEXT_SECONDARY))
                                    .fill(PANEL_BG)
                                    .stroke(egui::Stroke::new(1.0, BORDER))
                                    .corner_radius(16.0)
                                    .min_size(Vec2::new(0.0, 30.0))
                            };
                            if ui.add(btn).clicked() {
                                draft.selected_state = state_name.clone();
                            }
                        }
                    });
                });
            });

            ui.add_space(10.0);
        }

        // ── Re-evaluate / Reset bar ────────────────────────────────
        ui.add_space(8.0);
        let any_modified = self
            .folds_task
            .draft_drivers
            .iter()
            .any(|d| d.selected_state != d.original_state);
        let mut reevaluate_clicked = false;
        let mut reset_clicked = false;

        // Both buttons are locked while a re-eval is in flight. The
        // Re-evaluate button additionally swaps its label + spinner and
        // stays in its "working" visual regardless of any_modified, so
        // the user cannot edit pills back to un-modify it and make the
        // button appear to "deactivate" while work is still happening.
        let reset_enabled = any_modified && !reevaluating;
        let reeval_enabled = any_modified && !reevaluating;
        // Revert-to-original is always available when a model is
        // loaded and no re-eval is currently running. It doesn't
        // require pending pill edits — it's a full "go back to the
        // initial state" action, distinct from Reset (which only
        // undoes unsaved pill edits). Implemented by applying
        // revision 1 (the first/oldest entry) from the server's
        // revision history.
        let revert_enabled = !reevaluating
            && self.folds_task.model.is_some()
            && !self.folds_task.refresh_in_flight;
        let mut revert_clicked = false;

        ui.horizontal(|ui| {
            ui.spacing_mut().button_padding = Vec2::new(16.0, 9.0);
            let reset_btn = ui.add_enabled(
                reset_enabled,
                egui::Button::new(
                    RichText::new("Reset")
                        .size(14.0)
                        .color(if reset_enabled { TEXT_PRIMARY } else { TEXT_MUTED }),
                )
                .fill(SURFACE)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .corner_radius(6.0),
            );
            if reset_btn.clicked() {
                reset_clicked = true;
            }
            ui.add_space(10.0);

            let revert_btn = ui.add_enabled(
                revert_enabled,
                egui::Button::new(
                    RichText::new("Revert to original")
                        .size(14.0)
                        .color(if revert_enabled { TEXT_PRIMARY } else { TEXT_MUTED }),
                )
                .fill(SURFACE)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .corner_radius(6.0),
            );
            if revert_btn
                .on_hover_text(
                    "Undo every change and send the drivers back to the model's initial state.",
                )
                .clicked()
            {
                revert_clicked = true;
            }
            ui.add_space(12.0);

            if reevaluating {
                // In-flight state: disabled, with a spinner + label.
                // Wrapped in a dummy add_enabled_ui(false) so the
                // button's colours pick up egui's disabled styling for
                // free, and we get a consistent "locked" look.
                ui.add_enabled_ui(false, |ui| {
                    ui.add(
                        egui::Button::new(
                            RichText::new("\u{27F3}  Re-evaluating\u{2026}")
                                .size(14.0)
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(ACCENT_BLUE_DIM)
                        .corner_radius(6.0),
                    );
                });
                ui.add_space(4.0);
                ui.spinner();
            } else {
                let reeval_btn = ui.add_enabled(
                    reeval_enabled,
                    egui::Button::new(
                        RichText::new("Re-evaluate")
                            .size(14.0)
                            .strong()
                            .color(if reeval_enabled { Color32::WHITE } else { TEXT_MUTED }),
                    )
                    .fill(if reeval_enabled { ACCENT_BLUE_DIM } else { SURFACE })
                    .corner_radius(6.0),
                );
                if reeval_btn.clicked() {
                    reevaluate_clicked = true;
                }
            }
        });

        if reset_clicked {
            for d in &mut self.folds_task.draft_drivers {
                d.selected_state = d.original_state.clone();
            }
            self.folds_task.previous_outcomes = None;
        }
        if revert_clicked {
            // Open the revert-to-original confirmation modal. The
            // modal computes the diff from current → DB baseline so
            // the user sees exactly what's about to happen.
            self.revert_to_original_confirm = true;
        }
        if reevaluate_clicked {
            self.start_folds_reevaluate();
        }

        // Navigate after releasing borrows — suppressed during re-eval
        // so the user stays on the Drivers view and sees the banner.
        if !reevaluating {
            if let Some(idx) = navigate_to {
                self.model_view = ModelView::DriverDetail(idx);
            }
        }
    }

    /// Full-page driver detail — state descriptions and navigable
    /// "Related" section, all rendered as dark cards.
    fn render_driver_detail_page(&mut self, ui: &mut egui::Ui, idx: usize) {
        let back_label = match self.model_view_back {
            ModelView::VisualMap => "Visual Map",
            _ => "Drivers",
        };
        if back_button(ui, back_label).clicked() {
            self.model_view = self.model_view_back;
            self.model_view_back = ModelView::DriverList;
            return;
        }
        ui.add_space(12.0);

        let Some(draft) = self.folds_task.draft_drivers.get(idx) else {
            self.model_view = self.model_view_back;
            self.model_view_back = ModelView::DriverList;
            return;
        };

        // Driver name heading.
        ui.add(
            egui::Label::new(
                RichText::new(format!("{} ({})", &draft.name, &draft.code))
                    .size(22.0)
                    .strong()
                    .color(Color32::WHITE),
            )
            .wrap(),
        );
        ui.add_space(14.0);

        // Current state card — prominent, highlighted.
        let current_desc = draft
            .state_options
            .iter()
            .find(|(name, _)| *name == draft.selected_state)
            .map(|(_, desc)| desc.as_str())
            .unwrap_or("");
        section_card(ui, |ui| {
            ui.label(
                RichText::new("CURRENT STATE")
                    .size(12.0)
                    .strong()
                    .color(TEXT_SECONDARY),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(&draft.selected_state)
                    .size(17.0)
                    .strong()
                    .color(ACCENT_BLUE),
            );
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(
                    RichText::new(current_desc)
                        .size(14.0)
                        .color(TEXT_PRIMARY),
                )
                .wrap(),
            );
        });

        // All state descriptions card.
        ui.add_space(14.0);
        section_card(ui, |ui| {
            ui.label(
                RichText::new("ALL STATES")
                    .size(12.0)
                    .strong()
                    .color(TEXT_SECONDARY),
            );
            ui.add_space(12.0);
            for (state_idx, (name, desc)) in draft.state_options.iter().enumerate() {
                let is_current = *name == draft.selected_state;
                let name_color = if is_current { ACCENT_BLUE } else { Color32::WHITE };
                let body_color = if is_current { TEXT_PRIMARY } else { TEXT_SECONDARY };
                ui.label(
                    RichText::new(name)
                        .size(14.0)
                        .strong()
                        .color(name_color),
                );
                ui.add_space(3.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(desc).size(13.0).color(body_color),
                    )
                    .wrap(),
                );
                if state_idx + 1 < draft.state_options.len() {
                    ui.add_space(12.0);
                }
            }
        });

        // Related navigable rows.
        ui.add_space(18.0);
        ui.label(
            RichText::new("RELATED")
                .size(12.0)
                .strong()
                .color(TEXT_SECONDARY),
        );
        ui.add_space(10.0);

        let sections = [
            (
                DriverDetailSection::WhySelected,
                format!("Why was {} selected?", &draft.original_state),
            ),
            (DriverDetailSection::WhyMatters, "Why does this matter?".to_owned()),
            (DriverDetailSection::WhatShift, "What could shift?".to_owned()),
            (DriverDetailSection::WhatMonitor, "What should we monitor?".to_owned()),
        ];

        for (section, label) in &sections {
            let mut clicked = false;
            section_card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(label)
                                .size(15.0)
                                .color(Color32::WHITE),
                        )
                        .wrap(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("\u{276F}").size(15.0).color(ACCENT_BLUE),
                                )
                                .fill(Color32::TRANSPARENT)
                                .stroke(egui::Stroke::NONE),
                            )
                            .clicked()
                        {
                            clicked = true;
                        }
                    });
                });
            });
            if clicked {
                self.model_view = ModelView::DriverSection(idx, *section);
            }
            ui.add_space(10.0);
        }
    }

    /// Full-page content for a single driver section (justification,
    /// importance, shifts, or monitoring). Reached by clicking a Related
    /// row on the driver detail page.
    fn render_driver_section_page(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        section: DriverDetailSection,
    ) {
        let original_state = self
            .folds_task
            .draft_drivers
            .get(idx)
            .map(|d| d.original_state.clone())
            .unwrap_or_default();
        let driver_code = self
            .folds_task
            .draft_drivers
            .get(idx)
            .map(|d| d.code.clone())
            .unwrap_or_default();

        // Back button — use the driver code (always short) instead of the
        // full name, so wide driver titles don't get truncated here.
        let back_label = if driver_code.is_empty() {
            "Driver".to_owned()
        } else {
            format!("Driver ({driver_code})")
        };
        if back_button(ui, &back_label).clicked() {
            self.model_view = ModelView::DriverDetail(idx);
            return;
        }
        ui.add_space(12.0);

        // Section heading
        let heading = match section {
            DriverDetailSection::WhySelected => format!("Why was {} selected?", original_state),
            DriverDetailSection::WhyMatters => "Why does this matter?".to_owned(),
            DriverDetailSection::WhatShift => "What could shift?".to_owned(),
            DriverDetailSection::WhatMonitor => "What should we monitor?".to_owned(),
        };
        ui.add(
            egui::Label::new(
                RichText::new(&heading)
                    .size(22.0)
                    .strong()
                    .color(Color32::WHITE),
            )
            .wrap(),
        );
        ui.add_space(16.0);

        // Content — fetch from model data
        match section {
            DriverDetailSection::WhySelected => {
                // Justification from current.drivers[].justification
                let justification = self
                    .folds_task
                    .model
                    .as_ref()
                    .and_then(|m| {
                        m.current
                            .drivers
                            .iter()
                            .find(|ds| ds.code == driver_code)
                            .and_then(|ds| ds.justification.clone())
                    });

                if let Some(just) = justification {
                    section_card(ui, |ui| {
                        for (para_idx, para) in just.content.iter().enumerate() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(para)
                                        .size(15.0)
                                        .color(TEXT_PRIMARY),
                                )
                                .wrap(),
                            );
                            if para_idx + 1 < just.content.len() {
                                ui.add_space(12.0);
                            }
                        }
                    });

                    if !just.citations.is_empty() {
                        ui.add_space(16.0);
                        ui.label(
                            RichText::new("SOURCES")
                                .size(12.0)
                                .strong()
                                .color(TEXT_SECONDARY),
                        );
                        ui.add_space(8.0);
                        section_card(ui, |ui| {
                            for (cite_idx, cite) in just.citations.iter().enumerate() {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!("[{}] {}", cite.num, cite.source))
                                            .size(13.0)
                                            .color(TEXT_SECONDARY),
                                    )
                                    .wrap(),
                                );
                                if cite_idx + 1 < just.citations.len() {
                                    ui.add_space(6.0);
                                }
                            }
                        });
                    }
                } else {
                    ui.label(
                        RichText::new("No justification data available for this driver.")
                            .size(14.0)
                            .color(TEXT_MUTED),
                    );
                }
            }

            DriverDetailSection::WhyMatters => {
                self.render_driver_context_section(ui, &driver_code, |ctx| &ctx.importance);
            }

            DriverDetailSection::WhatShift => {
                self.render_driver_context_section(ui, &driver_code, |ctx| &ctx.shifts);
            }

            DriverDetailSection::WhatMonitor => {
                self.render_driver_context_section(ui, &driver_code, |ctx| &ctx.monitor);
            }
        }
    }

    /// Render a driver context section (importance, shifts, or monitor)
    /// with proper paragraph rendering for markdown-like content.
    fn render_driver_context_section(
        &self,
        ui: &mut egui::Ui,
        driver_code: &str,
        field: fn(&fiftyone_folds::DriverContext) -> &str,
    ) {
        let context = self
            .folds_task
            .model
            .as_ref()
            .and_then(|m| {
                m.drivers
                    .iter()
                    .find(|def| def.code == driver_code)
                    .and_then(|def| def.context.as_ref())
            });

        if let Some(ctx) = context {
            let content = field(ctx);
            if content.is_empty() {
                ui.label(
                    RichText::new("No data available for this section.")
                        .size(14.0)
                        .color(TEXT_MUTED),
                );
                return;
            }

            section_card(ui, |ui| {
                // Split on double newlines for paragraph breaks, render
                // bold headers (lines starting with **) differently.
                let paragraphs: Vec<&str> = content
                    .split("\n\n")
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .collect();
                for (p_idx, trimmed) in paragraphs.iter().enumerate() {
                    if trimmed.starts_with("**")
                        && (trimmed.contains("**\n") || trimmed.ends_with("**"))
                    {
                        let clean = trimmed.trim_matches('*').trim();
                        ui.label(
                            RichText::new(clean)
                                .size(15.0)
                                .strong()
                                .color(Color32::WHITE),
                        );
                        ui.add_space(8.0);
                    } else {
                        ui.add(
                            egui::Label::new(
                                RichText::new(*trimmed)
                                    .size(15.0)
                                    .color(TEXT_PRIMARY),
                            )
                            .wrap(),
                        );
                        if p_idx + 1 < paragraphs.len() {
                            ui.add_space(12.0);
                        }
                    }
                }
            });
        } else {
            ui.label(
                RichText::new("No context data available for this driver.")
                    .size(14.0)
                    .color(TEXT_MUTED),
            );
        }
    }

}

// ---------------------------------------------------------------------------
// UI layout
// ---------------------------------------------------------------------------

impl DashboardApp {
    /// Lazily decode and upload the mascot texture. Idempotent — safe to
    /// call every frame. The texture lives for the life of the app and is
    /// shared between the startup splash and the Help window header.
    fn ensure_mascot_texture(&mut self, ctx: &egui::Context) {
        if self.mascot_texture.is_none() {
            self.mascot_texture = load_mascot_texture(ctx);
        }
    }

    /// Render the startup splash overlay. Called from `update()` every
    /// frame while `self.splash.is_active()` is true. Handles lazy texture
    /// loading, auto-dismiss after the display window, and click/key
    /// dismissal. The caller returns early from `update()` so the rest of
    /// the dashboard chrome is suppressed while the splash is visible.
    fn render_splash(&mut self, ctx: &egui::Context) {
        // Display timeline (milliseconds from first shown):
        //   0       → splash appears
        //   FADE_IN → fully opaque
        //   HOLD    → fade-out begins
        //   DISMISS → splash removed and normal UI takes over
        //
        // The base HOLD is the no-loading case. If the app is still
        // fetching market data when the hold would otherwise elapse, the
        // hold is extended to cover the fetch; once the fetch completes
        // an additional POST_LOAD_HOLD_MS of visible "done" state runs
        // before fade-out kicks in.
        const FADE_IN_MS: u128 = 260;
        const HOLD_MS: u128 = 7600;
        const FADE_OUT_MS: u128 = 420;
        const DISMISS_MS: u128 = HOLD_MS + FADE_OUT_MS;
        /// Minimum hold time remaining after a startup fetch completes
        /// so the user sees a brief "done" state rather than the splash
        /// vanishing the instant the last row lands.
        const POST_LOAD_HOLD_MS: u128 = 2000;
        /// How close to fade-out we're allowed to drift while we're
        /// still actively loading. Keeps the splash solidly in the Hold
        /// phase rather than flickering into a partial fade.
        const LOADING_CLAMP_HEADROOM_MS: u128 = 500;

        // Lazy texture load on the first frame we're active — the egui
        // context doesn't exist during `new()`, so this has to happen here.
        self.ensure_mascot_texture(ctx);

        // ── Loading-state tracking ─────────────────────────────────
        let loading = self.refresh_in_flight;
        let now = std::time::Instant::now();

        // Record the first frame we saw an in-flight refresh, and the
        // first frame we saw it drop back to idle after having been in
        // flight. These two timestamps gate the post-load extension.
        if loading && self.splash.loading_start.is_none() {
            self.splash.loading_start = Some(now);
        }
        let just_finished_loading = !loading
            && self.splash.loading_start.is_some()
            && self.splash.loading_end.is_none();
        if just_finished_loading {
            self.splash.loading_end = Some(now);
        }

        let mut elapsed_ms = self.splash.elapsed().as_millis();

        // While a fetch is in flight, pin the clock inside the Hold
        // phase. This is how the splash "waits" for the auto-refresh —
        // we rewind shown_at forward so elapsed never crosses the
        // HOLD_MS threshold.
        if loading && elapsed_ms >= HOLD_MS - LOADING_CLAMP_HEADROOM_MS {
            let anchor_elapsed = HOLD_MS - LOADING_CLAMP_HEADROOM_MS;
            let anchor = now - std::time::Duration::from_millis(anchor_elapsed as u64);
            self.splash.shown_at = Some(anchor);
            elapsed_ms = anchor_elapsed;
        }

        // When loading transitions from in-flight → done, push the
        // clock forward so at least POST_LOAD_HOLD_MS of Hold time
        // remains. If there was already plenty of hold time left (eg.
        // the fetch finished in 2 seconds), leave the clock alone.
        if just_finished_loading {
            let target_elapsed = HOLD_MS.saturating_sub(POST_LOAD_HOLD_MS);
            if elapsed_ms > target_elapsed {
                self.splash.shown_at = Some(
                    now - std::time::Duration::from_millis(target_elapsed as u64),
                );
                elapsed_ms = target_elapsed;
            }
        }

        // User skip — only allowed once loading has finished. While
        // the refresh is still running the splash is effectively
        // modal, because the user asked for the in-flight feed to stay
        // visible until it's done.
        let user_skipped = if loading {
            false
        } else {
            ctx.input(|i| {
                i.pointer.any_click()
                    || (!i.events.is_empty() && i.keys_down.iter().any(|_| true))
            })
        };
        if user_skipped && elapsed_ms < HOLD_MS {
            if let Some(start) = self.splash.shown_at {
                let target = now - std::time::Duration::from_millis(HOLD_MS as u64);
                if start > target {
                    self.splash.shown_at = Some(target);
                    elapsed_ms = HOLD_MS;
                }
            }
        }

        // Auto-dismiss — never fires while we're still loading, because
        // elapsed_ms is clamped below HOLD_MS above.
        if elapsed_ms >= DISMISS_MS {
            self.splash.dismiss();
            ctx.request_repaint();
            return;
        }

        // Keep repainting while the splash is visible so the timer advances
        // and the fade is smooth.
        ctx.request_repaint_after(std::time::Duration::from_millis(16));

        // Snapshot the last few activity-log entries for the loading
        // readout. Copied into a local so the Area closure below can use
        // them without an overlapping borrow of `self`. Only taken when
        // we've seen the refresh at some point during this splash run.
        let show_activity_readout =
            self.splash.loading_start.is_some();
        let activity_snapshot: Vec<(String, String, Color32)> = if show_activity_readout
        {
            let n = self.activity_log.len();
            let start = n.saturating_sub(5);
            self.activity_log[start..]
                .iter()
                .map(|entry| {
                    let (status_text, status_color) = match &entry.status {
                        LogStatus::Fetching => {
                            ("fetching…".to_owned(), TEXT_SECONDARY)
                        }
                        LogStatus::Ok(count) => (
                            format!("loaded {count} pts"),
                            ALERT_NORMAL_FG,
                        ),
                        LogStatus::Cached(date) => {
                            (format!("cached {date}"), TEXT_MUTED)
                        }
                        LogStatus::Failed(err) => {
                            let brief = err.split('\n').next().unwrap_or(err);
                            let brief = if brief.len() > 40 {
                                format!("{}…", &brief[..40])
                            } else {
                                brief.to_owned()
                            };
                            (format!("failed: {brief}"), ALERT_EXTREME_FG)
                        }
                    };
                    (entry.instrument.as_str().to_owned(), status_text, status_color)
                })
                .collect()
        } else {
            Vec::new()
        };
        let loading_all_done = show_activity_readout && !loading;
        let refresh_in_flight_snapshot = loading;

        // Alpha multiplier for fade-in + fade-out.
        let alpha: f32 = if elapsed_ms < FADE_IN_MS {
            (elapsed_ms as f32 / FADE_IN_MS as f32).clamp(0.0, 1.0)
        } else if elapsed_ms < HOLD_MS {
            1.0
        } else {
            let fade_t = (elapsed_ms - HOLD_MS) as f32 / FADE_OUT_MS as f32;
            (1.0 - fade_t).clamp(0.0, 1.0)
        };
        let fade = |c: Color32| -> Color32 {
            let a = (c.a() as f32 * alpha) as u8;
            Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
        };

        // ── Full-window dark backdrop ──────────────────────────────
        // Covers whatever would otherwise be visible underneath. Content
        // is painted by the floating splash card below.
        let backdrop = egui::Frame::default()
            .fill(APP_BG)
            .inner_margin(egui::Margin::ZERO);
        egui::CentralPanel::default()
            .frame(backdrop)
            .show(ctx, |_ui| {});

        // ── Centered floating splash card (Office-style) ───────────
        // Classic splash pattern: fixed-size card centred on screen,
        // with a drop-shadow lifting it off the backdrop. Sized for the
        // portrait-orientation mascot; grows taller when the loading
        // readout is visible so the extra lines don't cramp the layout.
        let card_height = if show_activity_readout { 780.0 } else { 640.0 };
        let card_size = Vec2::new(540.0, card_height);
        let screen = ctx.screen_rect();
        let card_pos = egui::pos2(
            screen.center().x - card_size.x / 2.0,
            screen.center().y - card_size.y / 2.0,
        );

        egui::Area::new(egui::Id::new("splash_card"))
            .order(egui::Order::Foreground)
            .fixed_pos(card_pos)
            .show(ctx, |ui| {
                let shadow_alpha = (140.0 * alpha) as u8;
                egui::Frame::default()
                    .fill(fade(SURFACE))
                    .stroke(egui::Stroke::new(1.0, fade(BORDER)))
                    .corner_radius(14.0)
                    .inner_margin(egui::Margin::symmetric(36, 32))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 12],
                        blur: 48,
                        spread: 0,
                        color: Color32::from_black_alpha(shadow_alpha),
                    })
                    .show(ui, |ui| {
                        let inner_w = card_size.x - 72.0;
                        ui.set_width(inner_w);

                        ui.vertical_centered(|ui| {
                            // Mascot (or a blank spacer if decoding failed).
                            if let Some(ref tex) = self.mascot_texture {
                                let orig = tex.size_vec2();
                                let target_h = 240.0;
                                let scale = target_h / orig.y;
                                let size = orig * scale;
                                ui.add(
                                    egui::Image::new(egui::load::SizedTexture::new(
                                        tex.id(),
                                        size,
                                    ))
                                    .tint(fade(Color32::WHITE)),
                                );
                            } else {
                                ui.add_space(240.0);
                            }

                            ui.add_space(14.0);

                            // App name — the main branding moment.
                            ui.label(
                                RichText::new("The Hedgehog")
                                    .size(32.0)
                                    .strong()
                                    .color(fade(Color32::WHITE)),
                            );

                            ui.add_space(6.0);

                            // Tagline — carries the "one big thing" idea.
                            ui.label(
                                RichText::new(
                                    "Causal, probabilistic modelling of capital-markets regimes",
                                )
                                .size(13.0)
                                .color(fade(TEXT_SECONDARY)),
                            );

                            ui.add_space(14.0);

                            // Archilochus / Berlin epigraph — explains
                            // why this app is a hedgehog.
                            ui.add(
                                egui::Label::new(
                                    RichText::new(
                                        "\u{201C}The fox knows many things, but the hedgehog knows one big thing.\u{201D}",
                                    )
                                    .size(12.0)
                                    .italics()
                                    .color(fade(TEXT_PRIMARY)),
                                )
                                .wrap(),
                            );
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new("Archilochus, via Isaiah Berlin")
                                    .size(10.0)
                                    .color(fade(TEXT_MUTED)),
                            );

                            ui.add_space(18.0);

                            // Version pill — rendered as a Button with
                            // generous padding so the text doesn't crowd
                            // the border.
                            ui.spacing_mut().button_padding = Vec2::new(36.0, 14.0);
                            let _ = ui.add(
                                egui::Button::new(
                                    RichText::new("PREVIEW 0.1")
                                        .size(13.0)
                                        .strong()
                                        .color(fade(ACCENT_BLUE)),
                                )
                                .fill(fade(PANEL_BG))
                                .stroke(egui::Stroke::new(1.0, fade(BORDER)))
                                .corner_radius(16.0),
                            );

                            // ── Loading readout ────────────────────────
                            // Shown only when the startup auto-refresh
                            // was in flight during this splash run.
                            // Mirrors the activity-log panel but in a
                            // compressed two-column form (instrument ·
                            // status), capped to the last 5 entries.
                            if show_activity_readout {
                                ui.add_space(18.0);
                                let header = if refresh_in_flight_snapshot {
                                    "FETCHING MARKET DATA"
                                } else {
                                    "MARKET DATA READY"
                                };
                                let header_color = if refresh_in_flight_snapshot
                                {
                                    ACCENT_BLUE
                                } else {
                                    ALERT_NORMAL_FG
                                };
                                ui.label(
                                    RichText::new(header)
                                        .size(10.0)
                                        .strong()
                                        .color(fade(header_color)),
                                );
                                ui.add_space(6.0);

                                // Activity rows. Left-align inside a
                                // fixed column so wrapping lines up
                                // cleanly under the first character.
                                let readout_w = inner_w * 0.8;
                                ui.allocate_ui_with_layout(
                                    Vec2::new(readout_w, 0.0),
                                    egui::Layout::top_down(egui::Align::LEFT),
                                    |ui| {
                                        if activity_snapshot.is_empty() {
                                            ui.label(
                                                RichText::new("(waiting for first response…)")
                                                    .size(11.0)
                                                    .italics()
                                                    .color(fade(TEXT_MUTED)),
                                            );
                                        }
                                        for (instr, status, status_color) in
                                            &activity_snapshot
                                        {
                                            ui.horizontal(|ui| {
                                                ui.add_sized(
                                                    Vec2::new(
                                                        readout_w * 0.42,
                                                        16.0,
                                                    ),
                                                    egui::Label::new(
                                                        RichText::new(instr)
                                                            .size(11.0)
                                                            .color(fade(
                                                                TEXT_PRIMARY,
                                                            )),
                                                    ),
                                                );
                                                ui.label(
                                                    RichText::new("·")
                                                        .size(11.0)
                                                        .color(fade(TEXT_MUTED)),
                                                );
                                                ui.add_space(6.0);
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(status)
                                                            .size(11.0)
                                                            .color(fade(
                                                                *status_color,
                                                            )),
                                                    )
                                                    .wrap(),
                                                );
                                            });
                                        }
                                    },
                                );
                            }

                            // Bottom hint — pinned to the bottom of the
                            // inner content area. While loading, the
                            // hint is suppressed because clicks don't
                            // dismiss until the fetch completes.
                            let remaining = ui.available_height() - 18.0;
                            if remaining > 0.0 {
                                ui.add_space(remaining);
                            }
                            let hint_text = if refresh_in_flight_snapshot {
                                "Loading market data — please wait"
                            } else if loading_all_done {
                                "Ready · click anywhere to continue"
                            } else {
                                "Click anywhere to continue"
                            };
                            ui.label(
                                RichText::new(hint_text)
                                    .size(11.0)
                                    .color(fade(TEXT_MUTED)),
                            );
                        });
                    });
            });
    }
}

impl eframe::App for DashboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Fatal init error — show a full-screen message and nothing else.
        if let Some(ref msg) = self.init_error {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(80.0);
                        ui.label(
                            RichText::new("Failed to start")
                                .size(22.0)
                                .color(ALERT_EXTREME_FG)
                                .strong(),
                        );
                        ui.add_space(16.0);
                        ui.label(
                            RichText::new(msg)
                                .size(13.0)
                                .color(TEXT_SECONDARY)
                                .monospace(),
                        );
                        ui.add_space(24.0);
                        if ui.button("Quit").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });
            });
            return;
        }

        // Hide the DAG webview when not on the Visual Map tab (it's a
        // native view on top of the GPU surface and would float over
        // other egui content).
        let dag_should_be_visible = self.central_view == CentralView::Model
            && self.model_view == ModelView::VisualMap
            && self.folds_task.is_complete()
            && !self.show_help
            && !self.revert_to_original_confirm
            && !self.splash.is_active();
        if !dag_should_be_visible {
            self.dag_webview.set_visible(false);
        }

        self.poll_refresh();
        self.poll_ai();
        self.poll_bias_judge();
        self.poll_outcomes_reroll();
        self.poll_report();
        self.poll_folds();
        self.poll_research_terminal();
        if self.folds_task.in_flight {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        sanitize_overlay_selection(&mut self.settings);
        self.refresh_analysis_cache();

        // -- Startup splash overlay --
        // Takes over the whole window until the auto-dismiss timer elapses
        // or the user clicks / presses a key. Background polling above
        // continues to run so any auto-refresh kicked off at launch
        // finishes while the splash is visible.
        if self.splash.is_active() {
            self.render_splash(ctx);
            return;
        }

        // -- Global top bar --
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading(RichText::new("The Hedgehog").strong());
                ui.separator();
                if self.refresh_in_flight {
                    ui.spinner();
                }
                if ui.button("Refresh").clicked() {
                    self.start_refresh();
                }
                if ui.button("Save").clicked() {
                    if let Err(e) = self.settings.threshold_config.validate() {
                        self.set_status(&format!("Invalid threshold config: {e}"), StatusKind::Error);
                    } else {
                        let mut errors: Vec<String> = Vec::new();
                        if let Err(e) = self.save_keys_to_env() {
                            errors.push(e);
                        }
                        if let Err(e) = self.storage.save_settings(&self.settings) {
                            errors.push(format!("Settings save failed: {e:#}"));
                        }
                        if errors.is_empty() {
                            self.set_status("Settings saved.", StatusKind::Success);
                        } else {
                            self.set_status(&errors.join("; "), StatusKind::Error);
                        }
                    }
                }
                ui.separator();
                if ui.button("Help").clicked() {
                    self.show_help = !self.show_help;
                }
            });
            ui.horizontal(|ui| {
                if !self.status_line.is_empty() {
                    let color = match self.status_kind {
                        StatusKind::Info => TEXT_SECONDARY,
                        StatusKind::Success => ALERT_NORMAL_FG,
                        StatusKind::Error => ALERT_EXTREME_FG,
                    };
                    ui.label(RichText::new(&self.status_line).color(color).size(12.0));
                }
            });
        });

        self.show_dashboard(ctx);

        // -- Revert-to-original confirmation dialog --
        // Shown when the user clicks "Revert to original" on the
        // Drivers tab. Diffs the current in-memory model against the
        // immutable DB baseline so the user can see exactly what's
        // about to change before committing.
        if self.revert_to_original_confirm {
            let (title, lines, disabled): (String, Vec<String>, bool) = {
                let baseline = self
                    .last_inference_id
                    .and_then(|id| self.storage.load_folds_response_for_inference(id).ok().flatten())
                    .and_then(|json| serde_json::from_str::<fiftyone_folds::ModelResponse>(&json).ok());
                let current = self.folds_task.model.as_deref();
                match (baseline, current) {
                    (Some(b), Some(c)) => {
                        let lines = diff_model_states(c, &b);
                        let disabled = self.folds_task.in_flight
                            || self.folds_task.reevaluating;
                        ("Revert to original?".to_owned(), lines, disabled)
                    }
                    (None, _) => (
                        "Original unavailable".to_owned(),
                        vec!["No baseline stored locally for this model.".to_owned()],
                        true,
                    ),
                    (_, None) => (
                        "No current state".to_owned(),
                        vec!["Nothing to revert from.".to_owned()],
                        true,
                    ),
                }
            };
            let (cancel, confirm) =
                render_apply_confirm_dialog(ctx, &title, &lines, disabled);
            if cancel {
                self.revert_to_original_confirm = false;
            }
            if confirm {
                self.revert_to_original_confirm = false;
                self.start_folds_revert_to_original();
            }
        }

        // -- Help window (accessible from any tab) --
        if self.show_help {
            // Lazy-load the mascot texture before we hand off a mutable
            // borrow of `self.show_help` to `Window::open`.
            self.ensure_mascot_texture(ctx);
            let screen = ctx.screen_rect();
            let win_width = (screen.width() * 0.7).min(900.0).max(500.0);
            let win_height = (screen.height() * 0.85).min(800.0);
            // Destructure `self` into disjoint field borrows so the
            // closure below can touch `help_cache` and `mascot_texture`
            // without conflicting with `Window::open(&mut self.show_help)`.
            let Self { show_help, help_cache, mascot_texture, .. } = self;
            egui::Window::new("The Hedgehog - Help")
                .open(show_help)
                .default_size([win_width, win_height])
                .default_pos([
                    (screen.width() - win_width) / 2.0,
                    (screen.height() - win_height) / 2.0,
                ])
                .resizable(true)
                .collapsible(false)
                .show(ctx, |ui| {
                    render_help_header(ui, mascot_texture.as_ref());
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        egui_commonmark::CommonMarkViewer::new()
                            .show(ui, help_cache, help::HELP_TEXT);
                    });
                });
        }

    }
}

impl DashboardApp {
    fn show_dashboard(&mut self, ctx: &egui::Context) {
        // -- [P] price-picker / price-panel toggle --
        let text_focused = ctx.memory(|m| m.focused().is_some());
        if !text_focused && ctx.input(|i| i.key_pressed(egui::Key::P)) {
            if self.show_price_picker {
                self.show_price_picker = false;
                self.price_picker_filter.clear();
                self.price_picker_cursor = 0;
            } else if self.price_panel_instrument.is_some() {
                self.price_panel_instrument = None;
            } else {
                self.show_price_picker = true;
                self.price_picker_just_opened = true;
                self.price_picker_filter.clear();
                self.price_picker_cursor = 0;
            }
        }

        // -- Dashboard sub-toolbar --
        egui::TopBottomPanel::top("rs_toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                // Central view tabs
                ui.selectable_value(&mut self.central_view, CentralView::Charts, "Charts");
                let model_label = if self.folds_task.is_complete() {
                    RichText::new("51Folds").color(Color32::from_rgb(59, 130, 246))
                } else {
                    RichText::new("51Folds")
                };
                ui.selectable_value(&mut self.central_view, CentralView::Model, model_label);
                if self.folds_task.in_flight && !self.folds_task.reevaluating {
                    ui.spinner();
                }
                ui.selectable_value(
                    &mut self.central_view,
                    CentralView::ResearchAgent,
                    "Research Agent",
                );
                ui.separator();

                // Chart-specific controls (only when Charts tab is active)
                if self.central_view == CentralView::Charts {
                    for window in ChartWindow::ALL {
                        let resp = ui.selectable_value(
                            &mut self.settings.chart_window,
                            window,
                            window.label(),
                        );
                        if resp.clicked() {
                            self.custom_zoom = None;
                        }
                    }
                    if let Some((start, end)) = &self.custom_zoom {
                        ui.label(
                            RichText::new(format!(
                                "Zoom: {} - {}",
                                start.format("%d %b"),
                                end.format("%d %b"),
                            ))
                            .size(11.0)
                            .color(TEXT_SECONDARY),
                        );
                        if ui.small_button("Reset").clicked() {
                            self.custom_zoom = None;
                        }
                    }
                }

                // Model navigation (only when 51Folds tab is active)
                if self.central_view == CentralView::Model {
                    ui.selectable_value(&mut self.model_view, ModelView::Outcome, "Outcome");
                    ui.selectable_value(&mut self.model_view, ModelView::DriverList, "Drivers");
                    ui.selectable_value(&mut self.model_view, ModelView::VisualMap, "Visual Map");
                }

                // Report — always available (covers both AI and Research analyses)
                ui.separator();
                if ui.button("Report").clicked() {
                    if self.central_view == CentralView::Report {
                        self.central_view = CentralView::Charts;
                    } else {
                        self.central_view = CentralView::Report;
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !self.ai_panel_open && self.ai_response.is_some() {
                        if ui.small_button("AI").on_hover_text("Show AI Analysis panel").clicked() {
                            self.ai_panel_open = true;
                        }
                    }
                    if !self.show_activity_log && !self.activity_log.is_empty() {
                        if ui.small_button("Activity").on_hover_text("Show Activity log").clicked() {
                            self.show_activity_log = true;
                        }
                    }
                });
            });
        });

        // -- Bottom activity log (registered before side panels per egui ordering rules) --
        let mut clear_log = false;
        if !self.activity_log.is_empty() && self.show_activity_log {
            egui::TopBottomPanel::bottom("activity_log")
                .resizable(true)
                .min_height(80.0)
                .default_height(180.0)
                .max_height(600.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Activity")
                                .strong()
                                .size(11.0)
                                .color(TEXT_SECONDARY),
                        );
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui.small_button("\u{2715}").clicked() {
                                    clear_log = true;
                                }
                            },
                        );
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for entry in &self.activity_log {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(&entry.timestamp_str)
                                        .size(11.0)
                                        .color(TEXT_MUTED)
                                        .monospace(),
                                    );
                                    // Instrument color swatch
                                    let inst_color = instrument_color(entry.instrument);
                                    let (swatch_rect, _) = ui.allocate_exact_size(
                                        Vec2::new(10.0, 10.0),
                                        Sense::hover(),
                                    );
                                    ui.painter().rect_filled(swatch_rect, 2.0, inst_color);
                                    let label = format!(
                                        "{} ({})",
                                        entry.instrument.as_str(),
                                        entry.source.split_whitespace().next().unwrap_or("")
                                    );
                                    ui.label(
                                        RichText::new(label)
                                        .size(11.0)
                                        .monospace(),
                                    );
                                    match &entry.status {
                                        LogStatus::Fetching => {
                                            ui.spinner();
                                        }
                                        LogStatus::Ok(_) => {
                                            ui.label(
                                                RichText::new(format_log_status(&entry.status))
                                                    .size(11.0)
                                                    .color(TEXT_SECONDARY),
                                            );
                                        }
                                        LogStatus::Cached(_) => {
                                            ui.label(
                                                RichText::new(format_log_status(&entry.status))
                                                    .size(11.0)
                                                    .color(TEXT_MUTED),
                                            );
                                        }
                                        LogStatus::Failed(_) => {
                                            ui.label(
                                                RichText::new(format_log_status(&entry.status))
                                                    .size(11.0)
                                                    .color(ALERT_EXTREME_FG),
                                            );
                                        }
                                    }
                                });
                            }
                        });
                });
        }
        if clear_log {
            self.show_activity_log = false;
        }

        // -- AI analysis panel (bottom or right sidebar) --
        let mut close_ai_panel = false;
        let mut reanalyze = false;
        if self.ai_panel_open {
            match self.settings.ai_panel_dock {
                AiPanelDock::Bottom => {
                    // Cap height to content so the panel doesn't dwarf the charts.
                    let max_h = (self.ai_panel_content_height + 30.0).max(80.0);
                    egui::TopBottomPanel::bottom("ai_panel_bottom")
                        .min_height(80.0)
                        .max_height(max_h)
                        .resizable(true)
                        .show(ctx, |ui| {
                            self.render_ai_panel_contents(ui, &mut close_ai_panel, &mut reanalyze);
                        });
                }
                AiPanelDock::Right => {
                    egui::SidePanel::right("ai_panel_right")
                        .min_width(240.0)
                        .max_width(600.0)
                        .default_width(360.0)
                        .resizable(true)
                        .show(ctx, |ui| {
                            self.render_ai_panel_contents(ui, &mut close_ai_panel, &mut reanalyze);
                        });
                }
            }
        }
        if close_ai_panel {
            self.ai_panel_open = false;
        }
        if reanalyze {
            self.start_ai_analysis();
        }

        // -- Left sidebar --
        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(240.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if let Some(status) = &self.cached_vix_status {
                        sidebar_vix_summary(ui, status);
                        ui.separator();
                    }

                    sidebar_overlay_controls(ui, &mut self.settings);
                    ui.separator();

                    sidebar_spike_episodes(
                        ui,
                        &self.cached_spike_episodes,
                        &mut self.highlighted_spike,
                    );
                    ui.separator();

                    let keys_empty = self.api_keys.all_empty();
                    egui::CollapsingHeader::new("Data Source")
                        .default_open(keys_empty)
                        .show(ui, |ui| {
                            api_key_field(ui, "FRED", &mut self.api_keys.fred);
                            api_key_field(
                                ui,
                                "Alpha Vantage",
                                &mut self.api_keys.alpha_vantage,
                            );
                            ui.add_space(4.0);
                            ui.checkbox(
                                &mut self.settings.auto_refresh_on_startup,
                                "Auto-refresh on startup",
                            );
                        });

                    egui::CollapsingHeader::new("AI Analysis")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("Provider")
                                    .size(11.0)
                                    .color(TEXT_MUTED),
                            );
                            ui.horizontal(|ui| {
                                ui.selectable_value(
                                    &mut self.settings.ai_provider,
                                    LlmProvider::Anthropic,
                                    "Claude",
                                );
                                ui.selectable_value(
                                    &mut self.settings.ai_provider,
                                    LlmProvider::OpenAI,
                                    "GPT",
                                );
                            });
                            ui.add_space(4.0);
                            match self.settings.ai_provider {
                                LlmProvider::Anthropic => {
                                    api_key_field(
                                        ui,
                                        "Anthropic API Key",
                                        &mut self.api_keys.anthropic,
                                    );
                                }
                                LlmProvider::OpenAI => {
                                    api_key_field(
                                        ui,
                                        "OpenAI API Key",
                                        &mut self.api_keys.openai,
                                    );
                                }
                            }
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new("Model")
                                    .size(11.0)
                                    .color(TEXT_MUTED),
                            );
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(self.settings.effective_model_mut())
                                        .desired_width(140.0),
                                );
                                if ui.small_button("Default").clicked() {
                                    *self.settings.effective_model_mut() =
                                        self.settings.ai_provider.default_model().to_owned();
                                }
                            });
                            ui.add_space(4.0);
                            if self.ai_task.in_flight {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(
                                        RichText::new("Analyzing...")
                                            .size(11.0)
                                            .color(TEXT_SECONDARY),
                                    );
                                });
                            } else if ui.button("Analyze Current View").clicked() {
                                self.start_ai_analysis();
                            }

                            // Inference history
                            if !self.inference_history.is_empty() {
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new("History")
                                        .size(11.0)
                                        .color(TEXT_MUTED),
                                );
                                // Each row: short label visible, full
                                // label (with hypothesis question) shown
                                // on hover. Truncation is done MANUALLY
                                // — not via Label::truncate() — because
                                // egui's truncate() attaches its own
                                // auto-tooltip when the label is elided,
                                // and that combined with our explicit
                                // on_hover_text produced two tooltips.
                                let mut load_inference: Option<SavedInference> = None;
                                let page_size = 10;
                                let total = self.inference_history.len();
                                let total_pages = (total + page_size - 1) / page_size;
                                let start = self.history_page * page_size;
                                let end = (start + page_size).min(total);
                                let page_items = &self.inference_history[start..end];

                                for inf in page_items {
                                    let level_color = match inf.vix_level.as_deref() {
                                        Some("extreme") => ALERT_EXTREME_FG,
                                        Some("approaching_extreme") => ALERT_APPROACHING_FG,
                                        _ => ALERT_NORMAL_FG,
                                    };
                                    let short = inference_label_short(inf);
                                    let full = inference_label_full(inf);
                                    let folds_badge = self.folds_status_by_inference.get(&inf.id);
                                    let display = truncate_with_ellipsis(&short, 50);
                                    let hover = if let Some((mid, status)) = folds_badge {
                                        format!("51Folds model {mid} — {status}\n\n{full}")
                                    } else {
                                        full
                                    };
                                    ui.horizontal(|ui| {
                                        let (dot_rect, _) = ui.allocate_exact_size(
                                            Vec2::new(8.0, 8.0),
                                            Sense::hover(),
                                        );
                                        ui.painter().circle_filled(
                                            dot_rect.center(),
                                            4.0,
                                            level_color,
                                        );
                                        // Fixed-width model badge column
                                        let (badge_char, badge_color) = match folds_badge {
                                            Some((_, status)) => match status.as_str() {
                                                "success" => ("\u{25A0}", ALERT_NORMAL_FG),   // ■
                                                "pending" => ("\u{25A0}", ACCENT_BLUE),
                                                _ => ("\u{25A0}", ALERT_EXTREME_FG),
                                            },
                                            None => ("\u{25A1}", TEXT_MUTED),                  // □
                                        };
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(12.0, 10.0),
                                            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(badge_char)
                                                        .size(8.0)
                                                        .color(badge_color),
                                                );
                                            },
                                        );
                                        let resp = ui
                                            .add(
                                                egui::Label::new(
                                                    RichText::new(&display)
                                                        .size(10.0)
                                                        .color(TEXT_SECONDARY),
                                                )
                                                .sense(Sense::click()),
                                            )
                                            .on_hover_text(&hover);
                                        if resp.clicked() {
                                            load_inference = Some(inf.clone());
                                        }
                                    });
                                }
                                if let Some(inf) = load_inference {
                                    self.load_historical_inference(inf);
                                }
                                // Paging controls
                                if total_pages > 1 {
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| {
                                        if ui
                                            .add_enabled(
                                                self.history_page > 0,
                                                egui::Button::new(
                                                    RichText::new("\u{25C0}")
                                                        .size(9.0)
                                                        .color(TEXT_SECONDARY),
                                                ),
                                            )
                                            .clicked()
                                        {
                                            self.history_page -= 1;
                                        }
                                        ui.label(
                                            RichText::new(format!(
                                                "{}/{}",
                                                self.history_page + 1,
                                                total_pages,
                                            ))
                                            .size(9.0)
                                            .color(TEXT_MUTED),
                                        );
                                        if ui
                                            .add_enabled(
                                                self.history_page + 1 < total_pages,
                                                egui::Button::new(
                                                    RichText::new("\u{25B6}")
                                                        .size(9.0)
                                                        .color(TEXT_SECONDARY),
                                                ),
                                            )
                                            .clicked()
                                        {
                                            self.history_page += 1;
                                        }
                                    });
                                }
                                ui.add_space(6.0);
                                if ui.button("Clear History").clicked() {
                                    if let Err(e) = self.storage.clear_inferences() {
                                        self.set_status(&format!("Failed to clear history: {e}"), StatusKind::Error);
                                    } else {
                                        self.inference_history.clear();
                                        self.set_status("Inference history cleared.", StatusKind::Success);
                                    }
                                }
                            }
                        });

                    egui::CollapsingHeader::new("Research Agent")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("LLM Provider")
                                    .size(11.0)
                                    .color(TEXT_MUTED),
                            );
                            ui.horizontal(|ui| {
                                ui.selectable_value(
                                    &mut self.settings.research_provider,
                                    LlmProvider::Anthropic,
                                    "Claude",
                                );
                                ui.selectable_value(
                                    &mut self.settings.research_provider,
                                    LlmProvider::OpenAI,
                                    "GPT",
                                );
                            });
                            ui.add_space(4.0);
                            match self.settings.research_provider {
                                LlmProvider::Anthropic => {
                                    api_key_field(
                                        ui,
                                        "Anthropic API Key",
                                        &mut self.api_keys.anthropic,
                                    );
                                }
                                LlmProvider::OpenAI => {
                                    api_key_field(
                                        ui,
                                        "OpenAI API Key",
                                        &mut self.api_keys.openai,
                                    );
                                }
                            }
                            ui.add_space(4.0);
                            if let Some(code) = self.research_terminal.child_exited {
                                ui.label(
                                    RichText::new(format!("Dexter exited (code {code})"))
                                        .size(11.0)
                                        .color(ALERT_APPROACHING_FG),
                                );
                                if ui.button("Restart").clicked() {
                                    self.spawn_research_terminal();
                                }
                            } else if self.research_terminal.backend.is_some() {
                                ui.label(
                                    RichText::new("Running")
                                        .size(11.0)
                                        .color(ALERT_NORMAL_FG),
                                );
                                if ui.button("Restart").clicked() {
                                    self.spawn_research_terminal();
                                }
                            }
                            if let Some(ref err) = self.research_terminal.error {
                                ui.label(
                                    RichText::new(err)
                                        .size(11.0)
                                        .color(ALERT_EXTREME_FG),
                                );
                            }
                        });

                    let threshold_config_before = self.settings.threshold_config.clone();
                    ui.collapsing("Thresholds", |ui| {
                        sidebar_threshold_controls(ui, &mut self.settings);
                    });
                    if self.settings.threshold_config != threshold_config_before {
                        self.refresh_analysis_cache();
                    }

                    ui.collapsing("51Folds", |ui| {
                        api_key_field_with_hint(
                            ui,
                            "API Key",
                            &mut self.api_keys.folds,
                            "at_sk_...",
                        );
                    });
                });
            });

        // -- Central panel --
        // Pin an explicit dark Frame here so the central area can never
        // fall back to a light system theme. This is especially important
        // for the 51Folds model tabs, which otherwise rendered white text
        // on a near-white background on macOS Light mode.
        // Widen left margin when the Research Agent terminal is active so
        // the first-column circle glyphs aren't clipped by the panel edge.
        let central_left = if self.central_view == CentralView::ResearchAgent { 30 } else { 16 };
        let central_frame = egui::Frame::default()
            .fill(PANEL_BG)
            .inner_margin(egui::Margin { left: central_left, right: 16, top: 12, bottom: 12 });
        egui::CentralPanel::default().frame(central_frame).show(ctx, |ui| {
            match self.central_view {
                CentralView::Charts => {
                    let has_any_data = !self.series(Instrument::Vix).is_empty();

                    if !has_any_data {
                        empty_state_panel(ui, self.refresh_in_flight);
                        return;
                    }

                    let mut run_analysis = false;
                    if let Some(status) = &self.cached_vix_status {
                        run_analysis = status_banner(ui, status, self.ai_task.in_flight);
                    }

                    ui.add_space(8.0);

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let mut synced_x = self.synced_hover_x;
                        let mut any_hovered = false;
                        let mut drag_start = self.zoom_drag_start;
                        let mut custom_zoom = self.custom_zoom;
                        let mut vix_collapsed = self.vix_collapsed;
                        let mut correlation_collapsed = self.correlation_collapsed;
                        let mut price_panel_collapsed = self.price_panel_collapsed;

                        ui.add_space(8.0);
                        chart_vix(
                            ui,
                            self,
                            &mut synced_x,
                            &mut any_hovered,
                            &mut drag_start,
                            &mut custom_zoom,
                            &mut vix_collapsed,
                        );

                        ui.add_space(8.0);
                        chart_correlation(
                            ui,
                            self,
                            &mut synced_x,
                            &mut any_hovered,
                            &mut drag_start,
                            &mut custom_zoom,
                            &mut correlation_collapsed,
                        );

                        let price_instr = self.price_panel_instrument;
                        if let Some(instrument) = price_instr {
                            ui.add_space(8.0);
                            ui.separator();
                            chart_price_panel(
                                ui,
                                self,
                                instrument,
                                &mut synced_x,
                                &mut any_hovered,
                                &mut drag_start,
                                &mut custom_zoom,
                                &mut price_panel_collapsed,
                            );
                        }

                        self.synced_hover_x = if any_hovered { synced_x } else { None };
                        self.zoom_drag_start = drag_start;
                        self.custom_zoom = custom_zoom;
                        self.vix_collapsed = vix_collapsed;
                        self.correlation_collapsed = correlation_collapsed;
                        self.price_panel_collapsed = price_panel_collapsed;
                    });

                    if run_analysis {
                        self.start_ai_analysis();
                    }
                }
                CentralView::Model => {
                    self.render_central_model_view(ui);
                }
                CentralView::ResearchAgent => {
                    self.render_research_agent_panel(ui);
                }
                CentralView::Report => {
                    self.render_central_report_view(ui);
                }
            }
        });

        // -- Price picker overlay --
        if self.show_price_picker {
            match price_picker_area(ctx, self) {
                PricePickerAction::StillOpen => {}
                PricePickerAction::Cancelled => {
                    self.show_price_picker = false;
                    self.price_picker_filter.clear();
                    self.price_picker_cursor = 0;
                }
                PricePickerAction::Selected(instrument) => {
                    self.show_price_picker = false;
                    self.price_picker_filter.clear();
                    self.price_picker_cursor = 0;
                    self.price_panel_instrument = Some(instrument);
                }
            }
        }

    }

}

// ---------------------------------------------------------------------------
// Sidebar widgets
// ---------------------------------------------------------------------------

/// Render an API-key text field with a visible "set / not set" indicator so
/// the user can see at a glance which providers are configured. The actual
/// key value remains masked behind a password field.
fn api_key_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    api_key_field_with_hint(ui, label, value, "")
}

fn api_key_field_with_hint(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    hint: &str,
) {
    let is_set = !value.trim().is_empty();
    ui.horizontal(|ui| {
        ui.label(label);
        let (marker, color) = if is_set {
            ("● set", ALERT_NORMAL_FG)
        } else {
            ("○ not set", ALERT_EXTREME_FG)
        };
        ui.label(RichText::new(marker).size(10.0).color(color));
    });
    let mut edit = egui::TextEdit::singleline(value).password(true);
    if !hint.is_empty() {
        edit = edit.hint_text(hint);
    }
    ui.add(edit);
}

fn sidebar_vix_summary(ui: &mut egui::Ui, status: &VixStatus) {
    ui.heading("VIX Status");

    let (color, label) = match status.level {
        AlertLevel::Normal => (ALERT_NORMAL_FG, "Normal"),
        AlertLevel::ApproachingExtreme => (ALERT_APPROACHING_FG, "Approaching Extreme"),
        AlertLevel::Extreme => (ALERT_EXTREME_FG, "EXTREME"),
    };

    ui.horizontal(|ui| {
        // Draw a solid filled circle in the alert colour — avoids font glyph fallback.
        let (dot_rect, _) = ui.allocate_exact_size(Vec2::new(14.0, 14.0), Sense::hover());
        ui.painter().circle_filled(dot_rect.center(), 7.0, color);
        ui.label(RichText::new(label).color(color).strong().size(14.0));
    });

    ui.label(format!("Latest: {:.2}", status.latest.close));
    ui.label(format!("Date: {}", status.latest.date));
    ui.label(
        RichText::new(format!(
            "Approaching {:.1}  /  Extreme {:.1}",
            status.thresholds.approaching, status.thresholds.extreme
        ))
        .size(11.0)
        .color(TEXT_MUTED),
    );

    let src = if status.latest.source == "Seeded sample" {
        "example"
    } else {
        "live"
    };
    ui.label(
        RichText::new(format!("Source: {src}"))
            .size(11.0)
            .color(TEXT_MUTED),
    );
}

fn sidebar_overlay_controls(ui: &mut egui::Ui, settings: &mut AppSettings) {
    ui.heading("Overlay on VIX");

    let n = settings.overlay_instruments.len();
    ui.label(
        RichText::new(format!(
            "{n} asset{} selected",
            if n == 1 { "" } else { "s" }
        ))
        .size(11.0)
        .color(if n > 0 { ALERT_NORMAL_FG } else { TEXT_MUTED }),
    );

    ui.horizontal_wrapped(|ui| {
        if ui.small_button("Core 3").clicked() {
            settings.overlay_instruments =
                vec![Instrument::Gold, Instrument::Silver, Instrument::Bitcoin];
        }
        if ui.small_button("Energy").clicked() {
            settings.overlay_instruments = vec![Instrument::CrudeOil, Instrument::NaturalGas];
        }
        if ui.small_button("Metals").clicked() {
            settings.overlay_instruments = vec![
                Instrument::Gold,
                Instrument::Silver,
                Instrument::Copper,
                Instrument::Aluminum,
            ];
        }
        if ui.small_button("All").clicked() {
            settings.overlay_instruments = Instrument::ALL
                .iter()
                .copied()
                .filter(|i| *i != Instrument::Vix)
                .collect();
        }
        if ui.small_button("Clear").clicked() {
            settings.overlay_instruments.clear();
        }
    });

    ui.add_space(4.0);

    for group in AssetGroup::ALL {
        if group == AssetGroup::Volatility {
            continue;
        }
        ui.label(RichText::new(group.label()).size(11.0).color(TEXT_MUTED));
        for instrument in Instrument::group_members(group) {
            let mut enabled = settings.overlay_instruments.contains(instrument);
            let color = instrument_color(*instrument);
            ui.horizontal(|ui| {
                // Dot is always the instrument colour; checkbox state alone
                // conveys selection — no translucency effect needed.
                let (swatch_rect, _) =
                    ui.allocate_exact_size(Vec2::new(10.0, 10.0), Sense::hover());
                ui.painter().rect_filled(swatch_rect, 2.0, color);
                if ui.checkbox(&mut enabled, instrument.as_str()).changed() {
                    if enabled {
                        settings.overlay_instruments.push(*instrument);
                    } else {
                        settings.overlay_instruments.retain(|i| i != instrument);
                    }
                }
            });
        }
    }
}

fn sidebar_spike_episodes(
    ui: &mut egui::Ui,
    episodes: &[analysis::SpikeEpisode],
    highlighted: &mut Option<(chrono::NaiveDate, chrono::NaiveDate)>,
) {
    ui.heading("Recent Spikes");

    if episodes.is_empty() {
        ui.label(
            RichText::new("No spike episodes detected.")
                .size(11.0)
                .color(TEXT_MUTED),
        );
        return;
    }

    for ep in episodes {
        let level_color = match ep.max_level {
            AlertLevel::Normal => ALERT_NORMAL_FG,
            AlertLevel::ApproachingExtreme => ALERT_APPROACHING_FG,
            AlertLevel::Extreme => ALERT_EXTREME_FG,
        };
        let is_selected = *highlighted == Some((ep.start, ep.end));
        ui.horizontal_wrapped(|ui| {
            // Solid filled circle, clickable
            let (circle_rect, circle_resp) =
                ui.allocate_exact_size(Vec2::new(12.0, 12.0), Sense::click());
            let center = circle_rect.center();
            let radius = 5.0;
            ui.painter().circle_filled(center, radius, level_color);
            if is_selected {
                ui.painter().circle_stroke(
                    center,
                    radius + 1.5,
                    Stroke::new(1.5, Color32::WHITE),
                );
            }
            if circle_resp.clicked() {
                if is_selected {
                    *highlighted = None;
                } else {
                    *highlighted = Some((ep.start, ep.end));
                }
            }
            if circle_resp.hovered() {
                ui.painter().circle_stroke(
                    center,
                    radius + 1.0,
                    Stroke::new(1.0, Color32::from_gray(140)),
                );
            }
            ui.label(
                RichText::new(format!(
                    "{} to {} | peak {:.1} | {}d",
                    ep.start.format("%b %d"),
                    ep.end.format("%b %d"),
                    ep.peak,
                    ep.duration_points,
                ))
                .size(11.0),
            );
        });
    }
}

fn sidebar_threshold_controls(ui: &mut egui::Ui, settings: &mut AppSettings) {
    ui.horizontal(|ui| {
        ui.selectable_value(
            &mut settings.threshold_config.mode,
            ThresholdMode::RollingPercentile,
            "Percentile",
        );
        ui.selectable_value(
            &mut settings.threshold_config.mode,
            ThresholdMode::Fixed,
            "Fixed",
        );
    });

    ui.add(
        egui::Slider::new(&mut settings.threshold_config.lookback_days, 60..=504).text("Lookback"),
    );

    match settings.threshold_config.mode {
        ThresholdMode::RollingPercentile => {
            ui.add(
                egui::Slider::new(
                    &mut settings.threshold_config.percentile_approaching,
                    50.0..=99.0,
                )
                .text("Approaching %"),
            );
            ui.add(
                egui::Slider::new(
                    &mut settings.threshold_config.percentile_extreme,
                    70.0..=99.9,
                )
                .text("Extreme %"),
            );
        }
        ThresholdMode::Fixed => {
            ui.add(
                egui::Slider::new(
                    &mut settings.threshold_config.fixed_approaching,
                    10.0..=60.0,
                )
                .text("Approaching"),
            );
            ui.add(
                egui::Slider::new(&mut settings.threshold_config.fixed_extreme, 12.0..=80.0)
                    .text("Extreme"),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Central panel widgets
// ---------------------------------------------------------------------------

/// Back-navigation button used in the 51Folds model explorer detail pages.
fn back_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(format!("\u{276E}  {label}"))
                .size(14.0)
                .color(ACCENT_BLUE),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(egui::Stroke::NONE),
    )
}

/// Full-width banner shown at the top of the 51Folds model explorer
/// while a driver re-evaluate is in flight. Uses `SURFACE_HOVER` fill
/// with an `ACCENT_BLUE` left accent border so it reads as "system is
/// working" without shouting. Paired with egui's built-in spinner.
fn render_reeval_in_flight_banner(ui: &mut egui::Ui) {
    egui::Frame::default()
        .fill(SURFACE_HOVER)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(16, 12))
        .show(ui, |ui| {
            // Left accent stripe + content row.
            ui.horizontal(|ui| {
                ui.spinner();
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Re-evaluating with your driver changes\u{2026}")
                            .size(14.0)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(
                            "Driver edits are locked while 51Folds recomputes outcome probabilities. This usually takes a few seconds.",
                        )
                        .size(12.0)
                        .color(TEXT_SECONDARY),
                    );
                });
            });
        });
}

/// Red error banner shown at the top of the 51Folds model explorer when
/// a re-evaluate failed. The user's driver edits are preserved so they
/// can click Re-evaluate again to retry.
fn render_reeval_error_banner(ui: &mut egui::Ui, err: &str) {
    egui::Frame::default()
        .fill(Color32::from_rgb(60, 20, 25))
        .stroke(egui::Stroke::new(1.0, ALERT_EXTREME_FG))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Re-evaluation failed")
                        .size(14.0)
                        .strong()
                        .color(ALERT_EXTREME_FG),
                );
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(err).size(12.0).color(TEXT_PRIMARY),
                    )
                    .wrap(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Your driver edits are preserved. Click Re-evaluate on the Drivers tab to retry.",
                    )
                    .size(11.0)
                    .color(TEXT_SECONDARY),
                );
            });
        });
}

/// Fading success toast shown at the top of the Outcome tab for a few
/// seconds after a re-evaluation completes. The alpha multiplier is
/// computed from the caller's elapsed-time ratio (`0.0..1.0` where 1.0
/// is fully faded out).
fn render_reeval_success_toast(ui: &mut egui::Ui, fade_out: f32) {
    let alpha_f = (1.0 - fade_out).clamp(0.0, 1.0);
    let fade = |c: Color32| -> Color32 {
        let a = (c.a() as f32 * alpha_f) as u8;
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
    };
    egui::Frame::default()
        .fill(fade(Color32::from_rgb(18, 52, 36)))
        .stroke(egui::Stroke::new(1.0, fade(ALERT_NORMAL_FG)))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(16, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("\u{2713}")
                        .size(16.0)
                        .strong()
                        .color(fade(ALERT_NORMAL_FG)),
                );
                ui.add_space(8.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "Outcome probabilities updated from your driver edits. Rows below show before / after deltas.",
                        )
                        .size(13.0)
                        .color(fade(Color32::WHITE)),
                    )
                    .wrap(),
                );
            });
        });
}

/// Render a reusable "apply this change?" confirmation window that
/// shows a bulleted plain-English diff and Cancel/Apply buttons.
/// Returns `(cancelled, confirmed)`. Used for both "Apply this
/// snapshot" (History tab) and "Revert to original" (Drivers tab).
fn render_apply_confirm_dialog(
    ctx: &egui::Context,
    title: &str,
    lines: &[String],
    disabled: bool,
) -> (bool, bool) {
    let screen = ctx.screen_rect();
    let win_w = 520.0_f32.min(screen.width() * 0.85);
    let win_h = 420.0_f32.min(screen.height() * 0.85);
    let mut cancel = false;
    let mut confirm = false;
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .fixed_size([win_w, win_h])
        .default_pos([
            (screen.width() - win_w) / 2.0,
            (screen.height() - win_h) / 2.0,
        ])
        .show(ctx, |ui| {
            ui.label(
                RichText::new(title)
                    .size(16.0)
                    .strong()
                    .color(Color32::WHITE),
            );
            ui.add_space(10.0);
            ui.label(
                RichText::new(
                    "Here's what will change. 51Folds will re-infer and return updated probabilities.",
                )
                .size(12.0)
                .color(TEXT_SECONDARY),
            );
            ui.add_space(14.0);
            egui::Frame::default()
                .fill(SURFACE)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            if lines.is_empty() {
                                ui.label(
                                    RichText::new(
                                        "(This snapshot is identical to the current state — applying it will have no effect.)",
                                    )
                                    .size(12.0)
                                    .italics()
                                    .color(TEXT_MUTED),
                                );
                            } else {
                                for line in lines {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!("• {line}"))
                                                .size(12.0)
                                                .color(TEXT_PRIMARY),
                                        )
                                        .wrap(),
                                    );
                                    ui.add_space(3.0);
                                }
                            }
                        });
                });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().button_padding = Vec2::new(14.0, 8.0);
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("Cancel")
                                .size(13.0)
                                .color(TEXT_PRIMARY),
                        )
                        .fill(SURFACE)
                        .stroke(egui::Stroke::new(1.0, BORDER))
                        .corner_radius(6.0),
                    )
                    .clicked()
                {
                    cancel = true;
                }
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        let btn = ui.add_enabled(
                            !disabled && !lines.is_empty(),
                            egui::Button::new(
                                RichText::new("Apply")
                                    .size(13.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(ACCENT_BLUE_DIM)
                            .corner_radius(6.0),
                        );
                        if btn.clicked() {
                            confirm = true;
                        }
                    },
                );
            });
        });
    (cancel, confirm)
}

/// Compute a plain-English diff summary between two model states.
/// Reads `current.drivers[]` and `current.outcomes[]` from each
/// `ModelResponse` and produces a bulleted list of human-readable
/// change sentences suitable for the revert-to-original confirmation
/// dialog.
fn diff_model_states(
    from: &fiftyone_folds::ModelResponse,
    to: &fiftyone_folds::ModelResponse,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();

    // Driver state changes — keyed by code.
    for d in &to.current.drivers {
        let before = from
            .current
            .drivers
            .iter()
            .find(|f| f.code == d.code)
            .map(|f| f.state.as_str());
        match before {
            Some(b) if b != d.state => {
                lines.push(format!("{}: {b} → {}", d.code, d.state));
            }
            None => {
                lines.push(format!("{}: (new) → {}", d.code, d.state));
            }
            _ => {}
        }
    }

    // Outcome probability shifts — keyed by label.
    for o in &to.current.outcomes {
        if let Some(from_o) = from
            .current
            .outcomes
            .iter()
            .find(|f| f.label == o.label)
        {
            let from_prob = from_o.probability.unwrap_or(0.0);
            let to_prob = o.probability.unwrap_or(0.0);
            let delta = to_prob - from_prob;
            if delta.abs() > 0.001 {
                let arrow = if delta > 0.0 { "↑" } else { "↓" };
                lines.push(format!(
                    "{}: {:.1}% → {:.1}% {arrow}",
                    o.label,
                    from_prob * 100.0,
                    to_prob * 100.0,
                ));
            }
        }
    }

    lines
}

/// Dark surface card used throughout the 51Folds model explorer — groups
/// related content with a visible border on the panel background.
fn section_card<R>(
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::default()
        .fill(SURFACE)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(18, 16))
        .show(ui, add_contents)
        .inner
}

fn empty_state_panel(ui: &mut egui::Ui, refreshing: bool) {
    ui.add_space(80.0);
    ui.vertical_centered(|ui| {
        ui.heading(RichText::new("No Market Data Loaded").size(20.0));
        ui.add_space(16.0);

        if refreshing {
            ui.spinner();
            ui.label("Fetching data...");
        } else {
            egui::Frame::default()
                .fill(SURFACE)
                .corner_radius(8.0)
                .inner_margin(egui::Margin::same(20))
                .show(ui, |ui| {
                    ui.label(RichText::new("To get started:").strong().size(14.0));
                    ui.add_space(8.0);
                    ui.label("1. Open \"API Keys\" in the sidebar");
                    ui.label("2. Enter your FRED and/or Alpha Vantage API keys");
                    ui.label("3. Click \"Refresh\" for live market data");
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Free FRED key: fred.stlouisfed.org (Account > API Keys)  |  Free Alpha Vantage key: alphavantage.co/support/#api-key")
                            .size(11.0)
                            .color(TEXT_MUTED),
                    );
                });
        }
    });
}

/// Returns `true` when the user clicks the Analyze button in the banner.
fn status_banner(ui: &mut egui::Ui, status: &VixStatus, ai_in_flight: bool) -> bool {
    let accent = match status.level {
        AlertLevel::Normal => ALERT_NORMAL_FG,
        AlertLevel::ApproachingExtreme => ALERT_APPROACHING_FG,
        AlertLevel::Extreme => ALERT_EXTREME_FG,
    };

    let mut analyze_clicked = false;

    let outer = egui::Frame::default()
        .fill(SURFACE)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "VIX {:.2} - {}",
                        status.latest.close,
                        status.level.label()
                    ))
                    .color(accent)
                    .strong()
                    .size(16.0),
                );
                ui.label(
                    RichText::new(format!(
                        "Thresholds: {:.1} / {:.1}",
                        status.thresholds.approaching, status.thresholds.extreme
                    ))
                    .color(TEXT_SECONDARY)
                    .size(12.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ai_in_flight {
                        ui.spinner();
                        ui.label(
                            RichText::new("Analyzing...")
                                .size(11.0)
                                .color(TEXT_MUTED),
                        );
                    } else if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Analyze")
                                    .size(11.0)
                                    .color(ACCENT_BLUE),
                            )
                            .fill(PANEL_BG)
                            .stroke(egui::Stroke::new(1.0, ACCENT_BLUE_DIM))
                            .corner_radius(4.0),
                        )
                        .on_hover_text("Run AI analysis on the current view")
                        .clicked()
                    {
                        analyze_clicked = true;
                    }
                });
            });
        });

    // 3-px coloured left-accent bar painted over the frame edge.
    let r = outer.response.rect;
    ui.painter().rect_filled(
        Rect::from_min_max(r.min, Pos2::new(r.min.x + 3.0, r.max.y)),
        egui::CornerRadius::same(6),
        accent,
    );

    analyze_clicked
}

// ---------------------------------------------------------------------------
// Collapsible chart header
// ---------------------------------------------------------------------------

/// Full-width clickable header row with a collapse chevron, title, and right-
/// aligned summary text.  Returns `true` when clicked; caller should toggle
/// its `collapsed` flag in response.
fn collapsible_chart_header(
    ui: &mut egui::Ui,
    id_salt: &str,
    collapsed: bool,
    title: &str,
    right_text: &str,
) -> bool {
    let mut clicked = false;
    ui.push_id(id_salt, |ui| {
        let height = 30.0;
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());

        // Always-dark header background — no hover colour change.
        ui.painter().rect_filled(rect, 0.0, SURFACE);

        // Painter-drawn chevron — avoids Unicode glyph fallback issues.
        // ▶ (right-pointing) when collapsed, ▼ (down-pointing) when expanded.
        let cx = rect.min.x + 12.0;
        let cy = rect.center().y;
        let chevron_stroke = Stroke::new(1.5, TEXT_PRIMARY);
        if collapsed {
            // Right-pointing chevron: two lines from left-center to right-tip.
            let tip = Pos2::new(cx + 4.0, cy);
            let top = Pos2::new(cx - 2.0, cy - 4.0);
            let bot = Pos2::new(cx - 2.0, cy + 4.0);
            ui.painter().line_segment([top, tip], chevron_stroke);
            ui.painter().line_segment([bot, tip], chevron_stroke);
        } else {
            // Down-pointing chevron: two lines from upper-left and upper-right to bottom-tip.
            let tip = Pos2::new(cx, cy + 3.0);
            let left = Pos2::new(cx - 4.0, cy - 3.0);
            let right = Pos2::new(cx + 4.0, cy - 3.0);
            ui.painter().line_segment([left, tip], chevron_stroke);
            ui.painter().line_segment([right, tip], chevron_stroke);
        }

        // Title
        ui.painter().text(
            Pos2::new(rect.min.x + 24.0, rect.center().y),
            Align2::LEFT_CENTER,
            title,
            FontId::proportional(14.0),
            TEXT_PRIMARY,
        );

        // Right-side summary / hint — always white for legibility on dark bg.
        if !right_text.is_empty() {
            ui.painter().text(
                Pos2::new(rect.max.x - 6.0, rect.center().y),
                Align2::RIGHT_CENTER,
                right_text,
                FontId::proportional(11.0),
                TEXT_PRIMARY,
            );
        }

        clicked = resp.clicked();
    });
    clicked
}

// ---------------------------------------------------------------------------
// Charts
// ---------------------------------------------------------------------------

fn chart_vix(
    ui: &mut egui::Ui,
    app: &DashboardApp,
    synced_x: &mut Option<f64>,
    any_hovered: &mut bool,
    drag_start: &mut Option<f64>,
    custom_zoom: &mut Option<(chrono::NaiveDate, chrono::NaiveDate)>,
    collapsed: &mut bool,
) {
    // Header summary from cached pre-formatted string — no recompute per frame.
    let summary = &app.cached_vix_summary;

    // Pre-compute chart data (cheap — only filtering/mapping a Vec).
    let windowed = filter_for_zoom(
        app.series(Instrument::Vix),
        app.settings.chart_window,
        custom_zoom,
        app.cached_chart_end_date,
    );
    let date_label = if let (Some(first), Some(last)) = (windowed.first(), windowed.last()) {
        format!(
            "{}  -  {}  |  Latest {:.2}",
            first.date.format("%d %b %Y"),
            last.date.format("%d %b %Y"),
            last.close,
        )
    } else {
        String::new()
    };

    // Header + date info share one dark SURFACE band; chart canvas sits below.
    egui::Frame::default()
        .fill(SURFACE)
        .inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 6 })
        .show(ui, |ui| {
            if collapsible_chart_header(ui, "vix_hdr", *collapsed, "VIX Index", summary) {
                *collapsed = !*collapsed;
            }
            if !*collapsed && !date_label.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(&date_label)
                            .size(11.0)
                            .color(TEXT_PRIMARY),
                    );
                });
            }
        });
    if *collapsed {
        return;
    }

    let raw = analysis::raw_series(&windowed);
    let thresholds = app.cached_vix_status.as_ref().map(|s| &s.thresholds);

    // Convert highlighted spike dates to x-coords for the chart
    let highlight = app.highlighted_spike.map(|(start, end)| {
        (analysis::date_to_x(start), analysis::date_to_x(end))
    });

    paint_chart(
        ui,
        &[ChartLine {
            label: "VIX",
            points: raw,
            color: instrument_color(Instrument::Vix),
        }],
        thresholds,
        None,
        highlight.as_ref(),
        None,
        synced_x,
        any_hovered,
        drag_start,
        custom_zoom,
        280.0,
    );
}

fn chart_correlation(
    ui: &mut egui::Ui,
    app: &DashboardApp,
    synced_x: &mut Option<f64>,
    any_hovered: &mut bool,
    drag_start: &mut Option<f64>,
    custom_zoom: &mut Option<(chrono::NaiveDate, chrono::NaiveDate)>,
    collapsed: &mut bool,
) {
    // Header: asset count as summary; [P] hint as right-side affordance.
    let n = app.settings.overlay_instruments.len();
    let summary = format!(
        "{}  ·  [P] price view",
        if n == 0 {
            "no assets selected".to_owned()
        } else {
            format!("{n} asset{}", if n == 1 { "" } else { "s" })
        }
    );
    // Pre-compute all data before rendering frames.
    let vix_windowed = filter_for_zoom(
        app.series(Instrument::Vix),
        app.settings.chart_window,
        custom_zoom,
        app.cached_chart_end_date,
    );
    let vix_norm = analysis::normalize_series(&vix_windowed);
    let date_label = if let (Some(first), Some(last)) = (vix_windowed.first(), vix_windowed.last()) {
        format!(
            "{}  -  {}  |  % change from window start",
            first.date.format("%d %b %Y"),
            last.date.format("%d %b %Y"),
        )
    } else {
        String::new()
    };

    let mut chart_lines: Vec<ChartLine> = Vec::new();
    for &instrument in &app.settings.overlay_instruments {
        let windowed = filter_for_zoom(
            app.series(instrument),
            app.settings.chart_window,
            custom_zoom,
            app.cached_chart_end_date,
        );
        let normalized = analysis::normalize_series(&windowed);
        if !normalized.is_empty() {
            chart_lines.push(ChartLine {
                label: instrument.as_str(),
                color: instrument_color(instrument),
                points: normalized,
            });
        }
    }

    let empty_msg = if chart_lines.is_empty() {
        if custom_zoom.is_some() && !app.settings.overlay_instruments.is_empty() {
            "No commodity data available for this period."
        } else {
            "Select assets in the sidebar under Overlay on VIX."
        }
    } else {
        ""
    };

    // Header + date info + legend all share the dark SURFACE band.
    egui::Frame::default()
        .fill(SURFACE)
        .inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 6 })
        .show(ui, |ui| {
            if collapsible_chart_header(
                ui, "corr_hdr", *collapsed, "Asset Performance vs VIX", &summary,
            ) {
                *collapsed = !*collapsed;
            }
            if !*collapsed {
                if !date_label.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(6.0);
                        ui.label(RichText::new(&date_label).size(11.0).color(TEXT_PRIMARY));
                    });
                }
                if !empty_msg.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(6.0);
                        ui.label(RichText::new(empty_msg).size(12.0).color(TEXT_SECONDARY));
                    });
                }
                if !chart_lines.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(6.0);
                        for line in &chart_lines {
                            let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), Sense::hover());
                            ui.painter().rect_filled(rect, 2.0, line.color);
                            ui.label(RichText::new(line.label).size(11.0).color(TEXT_PRIMARY));
                            ui.add_space(6.0);
                        }
                    });
                }
            }
        });
    if *collapsed {
        return;
    }

    // Pass VIX normalized data as reference for relative hover
    let vix_ref = if vix_norm.is_empty() { None } else { Some(vix_norm.as_slice()) };

    paint_chart(
        ui,
        &chart_lines,
        None,
        Some(100.0),
        None,
        vix_ref,
        synced_x,
        any_hovered,
        drag_start,
        custom_zoom,
        340.0,
    );
}

// ---------------------------------------------------------------------------
// Chart painter
// ---------------------------------------------------------------------------

struct ChartLine {
    label: &'static str,
    points: Vec<(f64, f64)>,
    color: Color32,
}

fn paint_chart(
    ui: &mut egui::Ui,
    lines: &[ChartLine],
    thresholds: Option<&ThresholdSnapshot>,
    baseline: Option<f64>,
    highlight: Option<&(f64, f64)>,
    reference: Option<&[(f64, f64)]>,
    synced_x: &mut Option<f64>,
    any_hovered: &mut bool,
    drag_start: &mut Option<f64>,
    custom_zoom: &mut Option<(chrono::NaiveDate, chrono::NaiveDate)>,
    height: f32,
) {
    let is_normalized = baseline.is_some();
    let desired = Vec2::new(ui.available_width().max(200.0), height);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::drag());
    let painter = ui.painter_at(rect);

    // Background
    painter.rect(
        rect,
        6.0,
        APP_BG,
        Stroke::new(1.0, BORDER),
        StrokeKind::Outside,
    );

    // Collect bounds across all series
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;

    for line in lines {
        for &(x, y) in &line.points {
            x_min = x_min.min(x);
            x_max = x_max.max(x);
            y_min = y_min.min(y);
            y_max = y_max.max(y);
        }
    }

    if let Some(t) = thresholds {
        y_min = y_min.min(t.approaching);
        y_max = y_max.max(t.extreme);
    }
    if let Some(b) = baseline {
        y_min = y_min.min(b);
        y_max = y_max.max(b);
    }

    if !x_min.is_finite() || !x_max.is_finite() || !y_min.is_finite() || !y_max.is_finite() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No data",
            FontId::proportional(14.0),
            TEXT_MUTED,
        );
        return;
    }

    if (x_max - x_min).abs() < f64::EPSILON {
        x_max += 1.0;
        x_min -= 1.0;
    }
    if (y_max - y_min).abs() < f64::EPSILON {
        y_max += 1.0;
        y_min -= 1.0;
    }

    let y_pad = (y_max - y_min) * 0.08;
    y_min -= y_pad;
    y_max += y_pad;

    // Chart area (margins: left for y-labels, bottom for x-labels)
    let chart = Rect::from_min_max(
        Pos2::new(rect.left() + 50.0, rect.top() + 8.0),
        Pos2::new(rect.right() - 10.0, rect.bottom() - 22.0),
    );

    // -- Threshold bands (VIX chart only) --
    if let Some(t) = thresholds {
        let approaching_y = map_val(t.approaching, y_min, y_max, chart.bottom(), chart.top())
            .clamp(chart.top(), chart.bottom());
        let extreme_y = map_val(t.extreme, y_min, y_max, chart.bottom(), chart.top())
            .clamp(chart.top(), chart.bottom());

        // Green zone: below approaching
        if approaching_y < chart.bottom() {
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(chart.left(), approaching_y),
                    Pos2::new(chart.right(), chart.bottom()),
                ),
                0.0,
                Color32::from_rgba_unmultiplied(56, 161, 105, 10),
            );
        }

        // Amber zone: between approaching and extreme
        if approaching_y > extreme_y {
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(chart.left(), extreme_y),
                    Pos2::new(chart.right(), approaching_y),
                ),
                0.0,
                Color32::from_rgba_unmultiplied(214, 158, 46, 10),
            );
        }

        // Red zone: above extreme
        if extreme_y > chart.top() {
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(chart.left(), chart.top()),
                    Pos2::new(chart.right(), extreme_y),
                ),
                0.0,
                Color32::from_rgba_unmultiplied(229, 62, 62, 10),
            );
        }

        // Threshold lines
        painter.line_segment(
            [
                Pos2::new(chart.left(), approaching_y),
                Pos2::new(chart.right(), approaching_y),
            ],
            Stroke::new(1.0, ALERT_APPROACHING_FG),
        );
        painter.text(
            Pos2::new(chart.right() - 4.0, approaching_y - 2.0),
            Align2::RIGHT_BOTTOM,
            format!("Approaching {:.1}", t.approaching),
            FontId::monospace(10.0),
            ALERT_APPROACHING_FG,
        );

        painter.line_segment(
            [
                Pos2::new(chart.left(), extreme_y),
                Pos2::new(chart.right(), extreme_y),
            ],
            Stroke::new(1.0, ALERT_EXTREME_FG),
        );
        painter.text(
            Pos2::new(chart.right() - 4.0, extreme_y - 2.0),
            Align2::RIGHT_BOTTOM,
            format!("Extreme {:.1}", t.extreme),
            FontId::monospace(10.0),
            ALERT_EXTREME_FG,
        );
    }

    // -- Spike highlight band --
    if let Some(&(hx_start, hx_end)) = highlight {
        let sx = map_val(hx_start, x_min, x_max, chart.left(), chart.right());
        let ex = map_val(hx_end, x_min, x_max, chart.left(), chart.right());
        let band_left = sx.min(ex).max(chart.left());
        let band_right = sx.max(ex).min(chart.right()).max(band_left + 2.0);
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(band_left, chart.top()),
                Pos2::new(band_right, chart.bottom()),
            ),
            0.0,
            Color32::from_rgba_unmultiplied(148, 163, 184, 18),
        );
        painter.line_segment(
            [
                Pos2::new(band_left, chart.top()),
                Pos2::new(band_left, chart.bottom()),
            ],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(148, 163, 184, 70)),
        );
        painter.line_segment(
            [
                Pos2::new(band_right, chart.top()),
                Pos2::new(band_right, chart.bottom()),
            ],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(148, 163, 184, 70)),
        );
    }

    // -- Baseline (correlation chart) --
    if let Some(b) = baseline {
        let by = map_val(b, y_min, y_max, chart.bottom(), chart.top());
        painter.line_segment(
            [Pos2::new(chart.left(), by), Pos2::new(chart.right(), by)],
            Stroke::new(1.0, BORDER),
        );
        painter.text(
            Pos2::new(chart.left() - 4.0, by),
            Align2::RIGHT_CENTER,
            format!("{b:.0}"),
            FontId::monospace(10.0),
            TEXT_MUTED,
        );
    }

    // -- Y-axis grid + labels --
    let y_ticks = 5;
    for step in 0..=y_ticks {
        let t = step as f32 / y_ticks as f32;
        let y = egui::lerp(chart.bottom()..=chart.top(), t);
        let value = egui::lerp(y_min..=y_max, t as f64);

        painter.line_segment(
            [Pos2::new(chart.left(), y), Pos2::new(chart.right(), y)],
            Stroke::new(1.0, Color32::from_rgb(26, 34, 54)),
        );
        let y_label = if is_normalized {
            let pct = value - 100.0;
            if pct.abs() < 0.05 {
                "0%".to_owned()
            } else {
                format!("{pct:+.0}%")
            }
        } else {
            format!("{value:.1}")
        };
        painter.text(
            Pos2::new(rect.left() + 4.0, y),
            Align2::LEFT_CENTER,
            y_label,
            FontId::monospace(10.0),
            TEXT_MUTED,
        );
    }

    // -- X-axis: day-level tick marks and adaptive labels --
    let range_days = (x_max - x_min).round() as i32;
    let first_day = x_min.round() as i32;
    let last_day = x_max.round() as i32;

    // Determine label interval and format based on range
    let (label_interval, tick_interval, date_fmt): (i32, i32, &str) = if range_days <= 45 {
        (5, 1, "%d %b")
    } else if range_days <= 100 {
        (7, 1, "%d %b")
    } else if range_days <= 200 {
        (14, 3, "%d %b")
    } else if range_days <= 400 {
        (30, 7, "%b")
    } else {
        (60, 14, "%b '%y")
    };

    // Draw ticks and labels
    let mut day = first_day;
    while day <= last_day {
        let x_pos = map_val(day as f64, x_min, x_max, chart.left(), chart.right());
        if x_pos >= chart.left() && x_pos <= chart.right() {
            let is_label_tick = (day - first_day) % label_interval == 0;

            if is_label_tick {
                // Labeled tick: taller line + text
                painter.line_segment(
                    [
                        Pos2::new(x_pos, chart.bottom()),
                        Pos2::new(x_pos, chart.bottom() + 4.0),
                    ],
                    Stroke::new(1.0, BORDER),
                );
                if let Some(date) = chrono::NaiveDate::from_num_days_from_ce_opt(day) {
                    painter.text(
                        Pos2::new(x_pos, rect.bottom() - 2.0),
                        Align2::CENTER_BOTTOM,
                        date.format(date_fmt).to_string(),
                        FontId::monospace(9.0),
                        TEXT_SECONDARY,
                    );
                }
            } else {
                // Minor tick: subtle notch
                painter.line_segment(
                    [
                        Pos2::new(x_pos, chart.bottom()),
                        Pos2::new(x_pos, chart.bottom() + 2.0),
                    ],
                    Stroke::new(1.0, Color32::from_rgb(34, 45, 66)),
                );
            }
        }
        day += tick_interval;
    }

    // -- Data lines --
    for line in lines {
        if line.points.len() < 2 {
            continue;
        }

        let screen_points: Vec<Pos2> = line
            .points
            .iter()
            .map(|&(x, y)| {
                Pos2::new(
                    map_val(x, x_min, x_max, chart.left(), chart.right()),
                    map_val(y, y_min, y_max, chart.bottom(), chart.top()),
                )
            })
            .collect();

        // Copy the endpoint before Shape::line() consumes the vec; avoids cloning
        // the entire point list just to draw the terminal dot.
        let last_point = screen_points.last().copied();
        painter.add(Shape::line(screen_points, Stroke::new(2.0, line.color)));

        if let Some(last) = last_point {
            painter.circle_filled(last, 3.5, line.color);
        }
    }

    // -- Drag-to-zoom selection --
    let is_dragging = response.dragged_by(egui::PointerButton::Primary);

    if response.drag_started_by(egui::PointerButton::Primary) {
        if let Some(pos) = response.interact_pointer_pos() {
            if pos.x >= chart.left() && pos.x <= chart.right() {
                let ratio =
                    (pos.x - chart.left()) / (chart.right() - chart.left());
                *drag_start = Some(x_min + ratio as f64 * (x_max - x_min));
            }
        }
    }

    if is_dragging {
        if let (Some(start_x), Some(pos)) = (*drag_start, response.interact_pointer_pos()) {
            let ratio =
                ((pos.x - chart.left()) / (chart.right() - chart.left())).clamp(0.0, 1.0);
            let current_x = x_min + ratio as f64 * (x_max - x_min);
            let sx = map_val(start_x, x_min, x_max, chart.left(), chart.right());
            let ex = map_val(current_x, x_min, x_max, chart.left(), chart.right());
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(sx.min(ex).max(chart.left()), chart.top()),
                    Pos2::new(sx.max(ex).min(chart.right()), chart.bottom()),
                ),
                0.0,
                Color32::from_rgba_unmultiplied(148, 163, 184, 30),
            );
            painter.line_segment(
                [Pos2::new(sx.min(ex), chart.top()), Pos2::new(sx.min(ex), chart.bottom())],
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(148, 163, 184, 110)),
            );
            painter.line_segment(
                [Pos2::new(sx.max(ex), chart.top()), Pos2::new(sx.max(ex), chart.bottom())],
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(148, 163, 184, 110)),
            );
        }
    }

    if response.drag_stopped_by(egui::PointerButton::Primary) {
        if let (Some(start_x), Some(pos)) = (drag_start.take(), response.interact_pointer_pos()) {
            let ratio =
                ((pos.x - chart.left()) / (chart.right() - chart.left())).clamp(0.0, 1.0);
            let end_x = x_min + ratio as f64 * (x_max - x_min);
            let lo = start_x.min(end_x);
            let hi = start_x.max(end_x);
            // Minimum 3 days to avoid accidental micro-drags
            if (hi - lo) >= 3.0 {
                let d1 = chrono::NaiveDate::from_num_days_from_ce_opt(lo.round() as i32);
                let d2 = chrono::NaiveDate::from_num_days_from_ce_opt(hi.round() as i32);
                if let (Some(d1), Some(d2)) = (d1, d2) {
                    *custom_zoom = Some((d1, d2));
                }
            }
        }
    }

    // -- Hover crosshair + value readout --
    // Suppress hover tooltips while dragging
    // Determine hover_x: either from direct hover on this chart, or synced from the other chart
    let direct_hover = if is_dragging {
        None
    } else {
        response.hover_pos().filter(|p| {
            p.x >= chart.left() && p.x <= chart.right()
        })
    };

    let hover_x = if let Some(pointer) = direct_hover {
        let ratio = (pointer.x - chart.left()) / (chart.right() - chart.left());
        let hx = x_min + ratio as f64 * (x_max - x_min);
        *synced_x = Some(hx);
        *any_hovered = true;
        Some(hx)
    } else {
        *synced_x
    };

    if let Some(hover_x) = hover_x {
        let screen_x = map_val(hover_x, x_min, x_max, chart.left(), chart.right());

        if screen_x >= chart.left() && screen_x <= chart.right() {
            let is_direct = direct_hover.is_some();
            let crosshair_alpha = if is_direct { 70 } else { 45 };

            // Vertical crosshair
            painter.line_segment(
                [
                    Pos2::new(screen_x, chart.top()),
                    Pos2::new(screen_x, chart.bottom()),
                ],
                Stroke::new(
                    1.0,
                    Color32::from_rgba_unmultiplied(255, 255, 255, crosshair_alpha),
                ),
            );

            // Date label at bottom of crosshair
            let hover_date_str = {
                let days = hover_x.round() as i32;
                chrono::NaiveDate::from_num_days_from_ce_opt(days)
                    .map(|d| d.format("%d %b %Y").to_string())
                    .unwrap_or_default()
            };
            let date_align = if screen_x > chart.right() - 60.0 {
                Align2::RIGHT_TOP
            } else if screen_x < chart.left() + 60.0 {
                Align2::LEFT_TOP
            } else {
                Align2::CENTER_TOP
            };
            painter.text(
                Pos2::new(screen_x, chart.bottom() + 2.0),
                date_align,
                hover_date_str,
                FontId::monospace(10.0),
                if is_direct {
                    TEXT_PRIMARY
                } else {
                    TEXT_SECONDARY
                },
            );

            // Flip tooltip to left side when near the right edge
            let flip = screen_x > chart.right() - 180.0;
            let (tip_offset, tip_align) = if flip {
                (-8.0, Align2::RIGHT_TOP)
            } else {
                (8.0, Align2::LEFT_TOP)
            };

            // Interpolate reference (VIX) value at hover_x for relative display
            let ref_pct = reference.and_then(|r| interpolate_at(r, hover_x));

            // Interpolate each line's value at hover_x
            let tooltip_anchor_y = direct_hover
                .map(|p| p.y.max(chart.top() + 8.0))
                .unwrap_or(chart.top() + 8.0);
            let mut tooltip_y = tooltip_anchor_y;
            for line in lines {
                if line.points.len() < 2 {
                    continue;
                }
                if let Some(val) = interpolate_at(&line.points, hover_x) {
                    let screen_y =
                        map_val(val, y_min, y_max, chart.bottom(), chart.top());
                    painter.circle_filled(
                        Pos2::new(screen_x, screen_y),
                        3.5,
                        line.color,
                    );
                    let value_text = if let Some(vix_val) = ref_pct {
                        let asset_pct = val - 100.0;
                        let vix_pct = vix_val - 100.0;
                        let spread = asset_pct - vix_pct;
                        format!(
                            "{}: {:+.1}%  ({:+.1} vs VIX)",
                            line.label, asset_pct, spread
                        )
                    } else if is_normalized {
                        let pct = val - 100.0;
                        format!("{}: {:+.1}%", line.label, pct)
                    } else {
                        format!("{}: {:.2}", line.label, val)
                    };
                    painter.text(
                        Pos2::new(screen_x + tip_offset, tooltip_y),
                        tip_align,
                        value_text,
                        FontId::monospace(10.0),
                        line.color,
                    );
                    tooltip_y += 14.0;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

fn format_log_status(status: &LogStatus) -> String {
    match status {
        LogStatus::Fetching => String::new(),
        LogStatus::Ok(count) => format!("{count} pts"),
        LogStatus::Cached(date) => format!("cached ({})", date),
        LogStatus::Failed(err) => err.clone(),
    }
}

fn map_val(value: f64, src_min: f64, src_max: f64, dst_min: f32, dst_max: f32) -> f32 {
    let ratio = ((value - src_min) / (src_max - src_min)).clamp(0.0, 1.0) as f32;
    egui::lerp(dst_min..=dst_max, ratio)
}

fn interpolate_at(points: &[(f64, f64)], target_x: f64) -> Option<f64> {
    if points.is_empty() {
        return None;
    }
    if target_x <= points[0].0 {
        return Some(points[0].1);
    }
    if target_x >= points[points.len() - 1].0 {
        return Some(points[points.len() - 1].1);
    }
    for window in points.windows(2) {
        let (x0, y0) = window[0];
        let (x1, y1) = window[1];
        if target_x >= x0 && target_x <= x1 {
            if (x1 - x0).abs() < f64::EPSILON {
                return Some(y0);
            }
            let t = (target_x - x0) / (x1 - x0);
            return Some(y0 + t * (y1 - y0));
        }
    }
    None
}

fn filter_for_zoom<'a>(
    obs: &'a [Observation],
    window: ChartWindow,
    zoom: &Option<(chrono::NaiveDate, chrono::NaiveDate)>,
    ref_end_date: Option<chrono::NaiveDate>,
) -> &'a [Observation] {
    if obs.is_empty() {
        return obs;
    }
    let (start, end) = match zoom {
        Some((s, e)) => (*s, *e),
        None => match window.approx_days() {
            None => return obs,
            Some(days) => {
                let last = ref_end_date.unwrap_or_else(|| obs.last().unwrap().date);
                let start = last - chrono::Duration::days(days as i64);
                (start, last)
            }
        },
    };
    let lo = obs.partition_point(|o| o.date < start);
    let hi = obs.partition_point(|o| o.date <= end);
    &obs[lo..hi]
}

fn instrument_color(instrument: Instrument) -> Color32 {
    match instrument {
        Instrument::Vix => Color32::from_rgb(235, 106, 74),
        Instrument::Gold => Color32::from_rgb(232, 194, 86),
        Instrument::Silver => Color32::from_rgb(148, 190, 230),
        Instrument::Bitcoin => Color32::from_rgb(240, 149, 66),
        Instrument::CrudeOil => Color32::from_rgb(186, 109, 71),
        Instrument::NaturalGas => Color32::from_rgb(91, 168, 189),
        Instrument::Copper => Color32::from_rgb(201, 119, 84),
        Instrument::Aluminum => Color32::from_rgb(181, 190, 204),
        Instrument::Wheat => Color32::from_rgb(214, 174, 78),
        Instrument::Corn => Color32::from_rgb(235, 214, 77),
        Instrument::Soybeans => Color32::from_rgb(153, 187, 86),
    }
}

/// Update or insert key=value pairs in `.env` file content, preserving all
/// other lines (comments, blank lines, unrelated variables).
/// Terminal color palette matching The Hedgehog's dark UI.
fn hedgehog_terminal_theme() -> egui_term::TerminalTheme {
    let palette = egui_term::ColorPalette {
        background: String::from("#111827"), // PANEL_BG
        foreground: String::from("#e2e8f0"), // TEXT_PRIMARY
        black: String::from("#0a0e1a"),      // APP_BG
        red: String::from("#e53e3e"),        // ALERT_EXTREME_FG
        green: String::from("#38a169"),       // ALERT_NORMAL_FG
        yellow: String::from("#d69e2e"),      // ALERT_APPROACHING_FG
        blue: String::from("#60a5fa"),        // ACCENT_BLUE
        magenta: String::from("#aa759f"),
        cyan: String::from("#75b5aa"),
        white: String::from("#e2e8f0"),       // TEXT_PRIMARY
        bright_black: String::from("#4a5568"), // TEXT_MUTED
        bright_red: String::from("#fc8181"),
        bright_green: String::from("#68d391"),
        bright_yellow: String::from("#feca88"),
        bright_blue: String::from("#82b8c8"),
        bright_magenta: String::from("#c28cb8"),
        bright_cyan: String::from("#93d3c3"),
        bright_white: String::from("#f8f8f8"),
        bright_foreground: None,
        dim_foreground: String::from("#94a3b8"), // TEXT_SECONDARY
        dim_black: String::from("#0a0e1a"),
        dim_red: String::from("#712b2b"),
        dim_green: String::from("#5f6f3a"),
        dim_yellow: String::from("#a17e4d"),
        dim_blue: String::from("#456877"),
        dim_magenta: String::from("#704d68"),
        dim_cyan: String::from("#4d7770"),
        dim_white: String::from("#8e8e8e"),
    };
    egui_term::TerminalTheme::new(Box::new(palette))
}

fn update_env_content(content: &str, updates: &[(&str, &str)]) -> String {
    let mut lines: Vec<String> = content.lines().map(|l| l.to_owned()).collect();
    for &(key, value) in updates {
        let prefix = format!("{key}=");
        let new_line = format!("{key}={value}");
        if let Some(pos) = lines.iter().position(|l| l.starts_with(&prefix)) {
            lines[pos] = new_line;
        } else {
            lines.push(new_line);
        }
    }
    let mut result = lines.join("\n");
    if !result.is_empty() && !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Build a one-line label for an inference list entry. Format:
/// `MM-DD HH:MM  [Kind] VIX 23.9  Gold/Silver  · {hypothesis snippet}`
///
/// Used in places with horizontal room (the report window). For the
/// narrow sidebar history list, see `inference_label_short`, which omits
/// the hypothesis snippet so the row fits in a 240px-wide panel.
fn inference_label(inf: &SavedInference) -> String {
    let header = inference_label_short(inf);
    let snippet = inference_hypothesis_snippet(inf, 60);
    if snippet.is_empty() {
        header
    } else {
        format!("{header}  · {snippet}")
    }
}

/// Build the full label used in tooltips. Same shape as `inference_label`
/// but the hypothesis text is not snippetted — the user wants to read the
/// whole question on hover, not just the first 60 characters.
fn inference_label_full(inf: &SavedInference) -> String {
    let header = inference_label_short(inf);
    let hypothesis: String = if let Some(ref q) = inf.hypothesis_question {
        q.trim().to_owned()
    } else {
        inf.response
            .lines()
            .find(|l| !l.trim().is_empty() && !l.starts_with('#') && !l.starts_with("**Regime"))
            .unwrap_or("")
            .trim()
            .to_owned()
    };
    if hypothesis.is_empty() {
        header
    } else {
        format!("{header}\n\n{hypothesis}")
    }
}

/// Truncate `s` to at most `max_chars` characters, appending an ellipsis
/// if the original was longer. Operates on Unicode scalar values, not
/// bytes, so it does not split a multi-byte character.
fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let head: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{head}…")
}

/// Compact label for the sidebar history list. Stops at the overlay
/// instruments — no hypothesis snippet, so the line is short enough to
/// fit comfortably in a 240px sidebar without forcing a horizontal stretch.
/// The full label (with snippet) is shown as a hover tooltip.
fn inference_label_short(inf: &SavedInference) -> String {
    let ts: String = if inf.created_at.len() >= 16 {
        inf.created_at[5..16].replace('T', " ")
    } else {
        inf.created_at.clone()
    };
    let is_report = inf.provider.starts_with("report:");
    let kind = if is_report { "Report" } else { "Analysis" };
    let vix = inf
        .vix_close
        .map(|v| format!("VIX {v:.1}"))
        .unwrap_or_else(|| "n/a".to_owned());

    let overlay = inf
        .overlay_instruments
        .as_ref()
        .map(|keys| format_overlay_label(keys))
        .filter(|s| !s.is_empty())
        .unwrap_or_default();

    let mut label = format!("{ts}  [{kind}] {vix}");
    if !overlay.is_empty() {
        label.push_str("  ");
        label.push_str(&overlay);
    }
    label
}

/// Extract the most useful short snippet from an inference: the parsed
/// hypothesis question if we have it, otherwise the first non-header line
/// of the raw response. Used in `inference_label` and as the hover-text
/// continuation in the sidebar.
fn inference_hypothesis_snippet(inf: &SavedInference, max_chars: usize) -> String {
    let raw: String = if let Some(ref q) = inf.hypothesis_question {
        q.clone()
    } else {
        inf.response
            .lines()
            .find(|l| !l.trim().is_empty() && !l.starts_with('#') && !l.starts_with("**Regime"))
            .unwrap_or("")
            .to_owned()
    };
    raw.chars().take(max_chars).collect::<String>().trim().to_owned()
}

/// Convert a list of instrument storage keys (e.g. ["gold","silver","bitcoin"])
/// into a compact display label (e.g. "Gold/Silver/Bitcoin"). Caps at 3
/// names and appends `+N` for the rest so a long overlay does not push the
/// inference list off the right edge.
fn format_overlay_label(keys: &[String]) -> String {
    let names: Vec<&'static str> = keys
        .iter()
        .filter_map(|k| {
            Instrument::ALL
                .iter()
                .find(|i| i.storage_key() == k)
                .map(|i| i.as_str())
        })
        .collect();
    if names.is_empty() {
        return String::new();
    }
    if names.len() <= 3 {
        names.join("/")
    } else {
        let head: Vec<&str> = names.iter().take(3).copied().collect();
        format!("{}+{}", head.join("/"), names.len() - 3)
    }
}

/// Return the prefix of the LLM response up to (but not including) the
/// `**Hypothesis**:` marker. The 51Folds editor renders the hypothesis,
/// outcomes, and context as editable fields, so the markdown view above it
/// only needs the regime-classification portion (Regime, Confidence, Signal
/// Reading, Key Confirmation, Key Divergence, Watch For). If the marker is
/// not present (e.g. an older or malformed response) the full response is
/// returned unchanged.
fn split_off_hypothesis(response: &str) -> &str {
    response
        .find("**Hypothesis**:")
        .map(|idx| response[..idx].trim_end())
        .unwrap_or(response)
}

fn validate_model_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 128 {
        return Err("Model name must be 1-128 characters.".to_owned());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == ':')
    {
        return Err("Model name contains invalid characters.".to_owned());
    }
    Ok(())
}

fn database_path() -> PathBuf {
    let mut path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    path.push("data");
    path.push("regime_shift_dashboard.sqlite3");
    path
}

// ---------------------------------------------------------------------------
// Price picker
// ---------------------------------------------------------------------------

enum PricePickerAction {
    StillOpen,
    Cancelled,
    Selected(Instrument),
}

fn price_picker_area(ctx: &egui::Context, app: &mut DashboardApp) -> PricePickerAction {
    // Consume navigation keys before the Area is rendered so they don't bleed
    // through to the scroll area beneath.
    let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if esc {
        return PricePickerAction::Cancelled;
    }
    let arrow_down = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
    let arrow_up = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
    let enter = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));

    // Build (or reuse) the filtered candidate list from all non-VIX instruments.
    if app.price_picker_filter != app.price_picker_filter_prev || app.price_picker_candidates.is_empty() {
        app.price_picker_candidates = Instrument::ALL
            .iter()
            .copied()
            .filter(|&i| i != Instrument::Vix)
            .filter(|i| {
                app.price_picker_filter.is_empty()
                    || i.as_str().to_ascii_lowercase().contains(&app.price_picker_filter.to_ascii_lowercase())
            })
            .collect();
        app.price_picker_filter_prev = app.price_picker_filter.clone();
    }
    // Clone to a local variable so we don't hold a borrow on `app` while the UI
    // closure mutates other fields (e.g. price_picker_filter via TextEdit).
    let candidates: Vec<Instrument> = app.price_picker_candidates.clone();

    // Keep cursor in bounds after filter changes.
    if !candidates.is_empty() && app.price_picker_cursor >= candidates.len() {
        app.price_picker_cursor = candidates.len() - 1;
    }

    // Apply keyboard navigation now that we know the list length.
    if arrow_down && !candidates.is_empty() {
        app.price_picker_cursor = (app.price_picker_cursor + 1) % candidates.len();
    }
    if arrow_up && !candidates.is_empty() {
        app.price_picker_cursor = app
            .price_picker_cursor
            .checked_sub(1)
            .unwrap_or(candidates.len() - 1);
    }
    if enter {
        if let Some(&instrument) = candidates.get(app.price_picker_cursor) {
            return PricePickerAction::Selected(instrument);
        }
        return PricePickerAction::Cancelled;
    }

    let mut action = PricePickerAction::StillOpen;

    egui::Area::new(egui::Id::new("price_picker"))
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::default()
                .fill(PANEL_BG)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::same(14))
                .show(ui, |ui| {
                    ui.set_width(280.0);

                    ui.label(
                        RichText::new("Price view")
                            .strong()
                            .size(13.0)
                            .color(TEXT_PRIMARY),
                    );
                    ui.add_space(6.0);

                    // Filter text input — auto-focused when picker first opens.
                    let te_resp = ui.add(
                        egui::TextEdit::singleline(&mut app.price_picker_filter)
                            .hint_text("Filter instruments...")
                            .desired_width(f32::INFINITY),
                    );
                    if app.price_picker_just_opened {
                        te_resp.request_focus();
                        app.price_picker_just_opened = false;
                    }

                    ui.add_space(6.0);

                    if candidates.is_empty() {
                        ui.label(
                            RichText::new("No matching instruments")
                                .size(11.0)
                                .color(TEXT_MUTED),
                        );
                    } else {
                        for (idx, &instrument) in candidates.iter().enumerate() {
                            let is_cursor = idx == app.price_picker_cursor;
                            let color = instrument_color(instrument);
                            let row_height = 26.0;
                            let (row_rect, row_resp) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), row_height),
                                Sense::click(),
                            );

                            // Mouse hover moves the keyboard cursor.
                            if row_resp.hovered() {
                                app.price_picker_cursor = idx;
                            }

                            if ui.is_rect_visible(row_rect) {
                                if is_cursor || row_resp.hovered() {
                                    ui.painter().rect_filled(
                                        row_rect,
                                        4.0,
                                        SURFACE_HOVER,
                                    );
                                }
                                // Colour swatch
                                let swatch = egui::Rect::from_min_size(
                                    Pos2::new(row_rect.min.x + 10.0, row_rect.center().y - 4.0),
                                    Vec2::new(8.0, 8.0),
                                );
                                ui.painter().rect_filled(swatch, 2.0, color);
                                // Label
                                ui.painter().text(
                                    Pos2::new(row_rect.min.x + 26.0, row_rect.center().y),
                                    Align2::LEFT_CENTER,
                                    instrument.as_str(),
                                    FontId::proportional(12.5),
                                    TEXT_PRIMARY,
                                );
                            }

                            if row_resp.clicked() {
                                action = PricePickerAction::Selected(instrument);
                            }
                        }
                    }

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("\u{2191}\u{2193} navigate  \u{00b7}  Enter select  \u{00b7}  Esc cancel")
                            .size(10.0)
                            .color(TEXT_MUTED),
                    );
                });
        });

    action
}

// ---------------------------------------------------------------------------
// Price panel chart
// ---------------------------------------------------------------------------

fn chart_price_panel(
    ui: &mut egui::Ui,
    app: &DashboardApp,
    instrument: Instrument,
    synced_x: &mut Option<f64>,
    any_hovered: &mut bool,
    drag_start: &mut Option<f64>,
    custom_zoom: &mut Option<(chrono::NaiveDate, chrono::NaiveDate)>,
    collapsed: &mut bool,
) {
    // Header: latest close as summary; [P] dismiss hint on the right.
    let latest_close = app.series(instrument).last().map(|o| o.close);
    let summary = match latest_close {
        Some(v) => format!("{:.2}  ·  [P] close", v),
        None => "[P] close".to_owned(),
    };
    let title = format!("{} - Price", instrument.as_str());

    // Pre-compute chart data.
    let windowed = filter_for_zoom(
        app.series(instrument),
        app.settings.chart_window,
        custom_zoom,
        app.cached_chart_end_date,
    );
    let raw = analysis::raw_series(&windowed);
    let date_label = if let (Some(first), Some(last)) = (windowed.first(), windowed.last()) {
        format!(
            "{}  -  {}  |  Latest {:.2}",
            first.date.format("%d %b %Y"),
            last.date.format("%d %b %Y"),
            last.close,
        )
    } else {
        String::new()
    };

    // Header + date info on shared SURFACE band.
    egui::Frame::default()
        .fill(SURFACE)
        .inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 6 })
        .show(ui, |ui| {
            if collapsible_chart_header(ui, "price_hdr", *collapsed, &title, &summary) {
                *collapsed = !*collapsed;
            }
            if !*collapsed && !date_label.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new(&date_label).size(11.0).color(TEXT_PRIMARY));
                });
            }
        });
    if *collapsed {
        return;
    }

    paint_chart(
        ui,
        &[ChartLine {
            label: instrument.as_str(),
            points: raw,
            color: instrument_color(instrument),
        }],
        None,
        None,
        None,
        None,
        synced_x,
        any_hovered,
        drag_start,
        custom_zoom,
        240.0,
    );
}

// ---------------------------------------------------------------------------

fn sanitize_overlay_selection(settings: &mut AppSettings) {
    // Strip VIX (it is always shown on its own chart) and deduplicate, all in
    // two passes with no heap allocation.  A stack-allocated bitmask is safe
    // because Instrument::ALL has exactly 11 variants.
    settings
        .overlay_instruments
        .retain(|instrument| *instrument != Instrument::Vix);

    let mut seen = [false; 11];
    settings.overlay_instruments.retain(|instrument| {
        let idx = match instrument {
            Instrument::Vix => 0,
            Instrument::Gold => 1,
            Instrument::Silver => 2,
            Instrument::Bitcoin => 3,
            Instrument::CrudeOil => 4,
            Instrument::NaturalGas => 5,
            Instrument::Copper => 6,
            Instrument::Aluminum => 7,
            Instrument::Wheat => 8,
            Instrument::Corn => 9,
            Instrument::Soybeans => 10,
        };
        if seen[idx] {
            return false;
        }
        seen[idx] = true;
        true
    });
}
