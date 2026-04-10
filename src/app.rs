use crate::ai;
use crate::analysis;
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

const MAX_LOG_ENTRIES: usize = 500;

/// Which view the central panel is showing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CentralView {
    Charts,
    Model,
}

/// Sub-tab within the 51Folds model explorer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModelTab {
    Outcome,
    Drivers,
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
                let current_state = model
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
                DraftDriverState {
                    code: def.code.clone(),
                    name: def.name.clone(),
                    selected_state: current_state.clone(),
                    original_state: current_state,
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

    /// Load a completed model response from a JSON blob (e.g. from the
    /// database). On success, initializes draft drivers and marks as complete.
    fn load_from_json(&mut self, json: &str) {
        match serde_json::from_str::<fiftyone_folds::ModelResponse>(json) {
            Ok(model) => {
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
    ai_panel_open: bool,
    ai_panel_content_height: f32,
    ai_markdown_cache: egui_commonmark::CommonMarkCache,
    inference_history: Vec<SavedInference>,
    // Report generation state (Phase 2)
    show_report_window: bool,
    report_from: String,
    report_to: String,
    report_inferences: Vec<SavedInference>,
    report_task: LlmTask,
    report_result: Option<String>,
    report_markdown_cache: egui_commonmark::CommonMarkCache,
    // Central panel view mode
    central_view: CentralView,
    model_tab: ModelTab,
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
}

impl DashboardApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
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
        visuals.widgets.inactive.bg_fill = SURFACE;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
        visuals.widgets.hovered.bg_fill = SURFACE_HOVER;
        visuals.widgets.active.bg_fill = SURFACE_HOVER;
        // Selection (e.g. selectable_value active state): use a visible accent
        // blue so selected items are clearly distinct from unselected.
        visuals.selection.bg_fill = Color32::from_rgb(37, 65, 130);
        visuals.selection.stroke = egui::Stroke::new(1.0, TEXT_SECONDARY);
        _cc.egui_ctx.set_visuals(visuals);

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

        // Keys live in .env only — never in the database.
        // dotenvy::dotenv() returns the path of the loaded file so we know
        // where to write back when the user saves from the settings panel.
        let env_path = match dotenvy::dotenv() {
            Ok(path) => path,
            Err(_) => std::env::current_dir().unwrap_or_default().join(".env"),
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
            ai_panel_open: false,
            ai_panel_content_height: 200.0,
            ai_markdown_cache: egui_commonmark::CommonMarkCache::default(),
            inference_history: Vec::new(),
            show_report_window: false,
            report_from: String::new(),
            report_to: String::new(),
            report_inferences: Vec::new(),
            report_task: LlmTask::new(),
            report_result: None,
            report_markdown_cache: egui_commonmark::CommonMarkCache::default(),
            central_view: CentralView::Charts,
            model_tab: ModelTab::Outcome,
            parsed_hypothesis: None,
            draft_hypothesis: None,
            folds_task: FoldsTask::new(),
            last_inference_id: None,
            outcomes_task: LlmTask::new(),
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

            // Filter by source (VIX and Soybeans from FRED, commodities from Alpha Vantage)
            if instrument == Instrument::Vix {
                observations.retain(|o| o.source == "FRED VIXCLS");
            } else if instrument == Instrument::Soybeans {
                observations.retain(|o| o.source == "FRED PSOYBUSDM");
            } else {
                // All other commodities use Alpha Vantage
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
            if instrument == Instrument::Vix || instrument == Instrument::Soybeans {
                continue; // FRED-only instruments
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
        if let Ok(Some(date)) = self
            .storage
            .last_observation_date_for_provider(Instrument::Soybeans, "FRED PSOYBUSDM")
        {
            cached_dates_fred.insert(Instrument::Soybeans, date);
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

                self.parsed_hypothesis = hypothesis.clone();
                self.draft_hypothesis = hypothesis;
                self.folds_task.reset();
                self.ai_response = Some(result.response);
                self.reload_inference_history();
            }
            LlmPoll::Failed | LlmPoll::Pending | LlmPoll::Idle => {}
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
        // Clear previous folds_task, then try to load the linked model
        // from the database so completed models appear immediately.
        self.folds_task.reset();
        if let Ok(Some(json)) = self.storage.load_folds_response_for_inference(inf.id) {
            self.folds_task.load_from_json(&json);
            if self.folds_task.is_complete() {
                self.central_view = CentralView::Model;
                self.model_tab = ModelTab::Outcome;
            }
        }
        self.outcomes_task = LlmTask::new();
        self.ai_response = Some(inf.response);
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

        // Snapshot current outcomes for before/after comparison
        let current_outcomes: Vec<(String, f64)> = model
            .current
            .outcomes
            .iter()
            .map(|o| (o.label.clone(), o.probability.unwrap_or(0.0)))
            .collect();
        self.folds_task.previous_outcomes = Some(current_outcomes);

        // Build driver state inputs from modified drafts only
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

        let (tx, rx) = mpsc::channel();
        self.folds_task.rx = Some(rx);
        self.folds_task.reevaluating = true;
        self.folds_task.in_flight = true;
        self.folds_task.error = None;

        thread::spawn(move || {
            crate::folds::patch_drivers(api_key, model_id, changed, tx);
        });
    }

    fn poll_folds(&mut self) {
        let was_complete = self.folds_task.is_complete();
        self.folds_task.poll();
        // Auto-switch to the model view when a build just completed
        if !was_complete && self.folds_task.is_complete() {
            self.central_view = CentralView::Model;
            self.model_tab = ModelTab::Outcome;
        }
    }

    /// Resume polling for any 51Folds models that were still `pending` when
    /// the app last shut down. Called once from `App::new`.
    ///
    /// - elapsed > 35 min → mark `undisclosed_failure` immediately
    /// - elapsed > 25 min → still poll, but count as "suspect"
    /// - otherwise → spawn a background polling thread (DB-only, no UI)
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

        for record in pending {
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
            thread::spawn(move || {
                crate::folds::resume_poll(
                    api_key_c,
                    model_id_c,
                    db_path_c,
                    created_at_c,
                );
            });
            resumed += 1;
        }

        if resumed > 0 || marked_failure > 0 {
            let mut msg = format!("Resumed polling for {resumed} pending 51Folds model(s)");
            if suspect > 0 {
                msg.push_str(&format!(
                    " — {suspect} suspect (>25 min pending)"
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
            .load_recent_inferences(20)
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
            ui.label(
                RichText::new("AI Analysis")
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

                // 51Folds section — shown after a successful analysis
                if self.ai_response.is_some() && !self.ai_task.in_flight {
                    self.render_folds_section(ui);
                }
            });
        self.ai_panel_content_height = scroll_out.content_size.y;
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

        // ── Post-creation: completed model → compact summary ──────
        if self.folds_task.is_complete() {
            let model_id = self.folds_task.model_id.clone().unwrap_or_default();
            ui.label(
                RichText::new(format!("Model {model_id} — ✓ complete"))
                    .size(11.0)
                    .color(ALERT_NORMAL_FG),
            );
            ui.add_space(4.0);
            // Brief outcome listing
            if let Some(ref model) = self.folds_task.model {
                for o in &model.current.outcomes {
                    ui.label(
                        RichText::new(format!(
                            "  {} — {:.1}%",
                            o.label,
                            o.probability.unwrap_or(0.0) * 100.0
                        ))
                        .size(10.0)
                        .color(TEXT_SECONDARY),
                    );
                }
            }
            ui.add_space(4.0);
            if ui.button("View in 51Folds tab").clicked() {
                self.central_view = CentralView::Model;
                self.model_tab = ModelTab::Outcome;
            }
            return;
        }

        // ── Error state ────────────────────────────────────────────
        if let Some(ref err) = self.folds_task.error.clone() {
            ui.label(
                RichText::new(format!("Error: {err}"))
                    .size(11.0)
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
                format!("Model {model_id} — building…")
            };
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(label)
                        .size(11.0)
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

    /// Render the 51Folds model explorer in the central panel with
    /// Outcome / Drivers sub-tabs.
    fn render_central_model_view(&mut self, ui: &mut egui::Ui) {
        if !self.folds_task.is_complete() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Run an AI analysis and create a 51Folds model to see results here.")
                        .size(14.0)
                        .color(TEXT_MUTED),
                );
            });
            return;
        }

        ui.add_space(8.0);

        // Model header
        let model_id = self.folds_task.model_id.clone().unwrap_or_default();
        let question = self
            .folds_task
            .model
            .as_ref()
            .map(|m| m.question.clone())
            .unwrap_or_default();
        ui.label(
            RichText::new(format!("Model {model_id}"))
                .size(13.0)
                .strong()
                .color(ALERT_NORMAL_FG),
        );
        if !question.is_empty() {
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(&question)
                        .size(12.0)
                        .color(TEXT_SECONDARY),
                )
                .wrap(),
            );
        }
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                match self.model_tab {
                    ModelTab::Outcome => {
                        self.render_central_outcome_tab(ui);
                    }
                    ModelTab::Drivers => {
                        self.render_central_drivers_tab(ui);
                    }
                }
            });
    }

    /// Outcome sub-tab: probability bars + take away summary.
    fn render_central_outcome_tab(&mut self, ui: &mut egui::Ui) {
        let row_width = ui.available_width().min(800.0);

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
            // Outcome label — bright white for readability
            ui.label(
                RichText::new(label)
                    .size(13.0)
                    .strong()
                    .color(Color32::WHITE),
            );
            ui.add_space(2.0);

            // Probability bar
            let bar_height = 28.0;
            let desired = Vec2::new(row_width, bar_height);
            let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
            let painter = ui.painter();

            painter.rect_filled(rect, 4.0, SURFACE);

            let bar_width = (*prob as f32 * rect.width()).max(0.0);
            let fill_rect = Rect::from_min_size(rect.min, Vec2::new(bar_width, bar_height));
            let bar_color = Color32::from_rgb(59, 130, 246);
            painter.rect_filled(fill_rect, 4.0, bar_color);

            painter.text(
                Pos2::new(rect.right() - 8.0, rect.center().y),
                Align2::RIGHT_CENTER,
                format!("{:.1}%", prob * 100.0),
                FontId::proportional(13.0),
                Color32::WHITE,
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
                        ui.label(RichText::new(text).size(10.0).color(color));
                    }
                }
            }
            ui.add_space(8.0);
        }

        // Take Away summary
        let summary = self
            .folds_task
            .model
            .as_ref()
            .map(|m| m.short_summary.clone())
            .unwrap_or_default();
        if !summary.is_empty() {
            ui.add_space(12.0);
            ui.label(
                RichText::new("Take Away")
                    .size(13.0)
                    .strong()
                    .color(TEXT_SECONDARY),
            );
            ui.add_space(6.0);
            let block_indent: f32 = 8.0;
            let frame_inner_h_margin: f32 = 12.0;
            let inner_text_width = (row_width - block_indent * 2.0 - frame_inner_h_margin * 2.0).max(60.0);
            ui.horizontal(|ui| {
                ui.add_space(block_indent);
                egui::Frame::default()
                    .fill(SURFACE)
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin::symmetric(frame_inner_h_margin as i8, 10))
                    .show(ui, |ui| {
                        ui.set_min_width(inner_text_width);
                        ui.set_max_width(inner_text_width);
                        ui.add(
                            egui::Label::new(
                                RichText::new(&summary)
                                    .size(13.0)
                                    .color(TEXT_PRIMARY),
                            )
                            .wrap(),
                        );
                    });
            });
        }
    }

    /// Drivers sub-tab: driver list with state selectors, expandable
    /// details, and re-evaluate / reset buttons.
    fn render_central_drivers_tab(&mut self, ui: &mut egui::Ui) {
        let driver_count = self.folds_task.draft_drivers.len();
        ui.label(
            RichText::new(format!("Drivers ({driver_count})"))
                .size(13.0)
                .strong()
                .color(TEXT_SECONDARY),
        );
        ui.add_space(6.0);

        // Pre-fetch context and justification data to avoid borrow conflicts
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
                .inner_margin(egui::Margin::symmetric(10, 8))
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
                                .size(12.0)
                                .strong()
                                .color(name_color),
                        );
                        ui.label(
                            RichText::new(format!("({})", draft.code))
                                .size(10.0)
                                .color(TEXT_MUTED),
                        );
                    });
                    ui.add_space(2.0);

                    // Segmented state selector
                    ui.horizontal_wrapped(|ui| {
                        for (state_name, _desc) in &draft.state_options {
                            let selected = *state_name == draft.selected_state;
                            let text = RichText::new(state_name).size(11.0);
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
                        ui.add_space(6.0);

                        // State descriptors
                        ui.label(
                            RichText::new("State descriptions")
                                .size(11.0)
                                .strong()
                                .color(TEXT_SECONDARY),
                        );
                        for (sn, sd) in &draft.state_options {
                            let highlight = *sn == draft.selected_state;
                            let color = if highlight { TEXT_PRIMARY } else { TEXT_MUTED };
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!("  {sn}: {sd}"))
                                        .size(11.0)
                                        .color(color),
                                )
                                .wrap(),
                            );
                        }

                        // Justification
                        if let Some(Some(just)) = driver_justifications.get(i) {
                            if !just.content.is_empty() {
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(format!(
                                        "Why was {} selected?",
                                        draft.original_state
                                    ))
                                    .size(11.0)
                                    .strong()
                                    .color(TEXT_SECONDARY),
                                );
                                for line in &just.content {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(line)
                                                .size(11.0)
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
                                                .size(10.0)
                                                .color(TEXT_MUTED),
                                        );
                                    }
                                }
                            }
                        }

                        // Context sections
                        if let Some(Some(ctx)) = driver_contexts.get(i) {
                            if !ctx.importance.is_empty() {
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new("Why does this matter?")
                                        .size(11.0)
                                        .strong()
                                        .color(TEXT_SECONDARY),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&ctx.importance)
                                            .size(11.0)
                                            .color(TEXT_SECONDARY),
                                    )
                                    .wrap(),
                                );
                            }
                            if !ctx.shifts.is_empty() {
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new("What could shift?")
                                        .size(11.0)
                                        .strong()
                                        .color(TEXT_SECONDARY),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&ctx.shifts)
                                            .size(11.0)
                                            .color(TEXT_SECONDARY),
                                    )
                                    .wrap(),
                                );
                            }
                            if !ctx.monitor.is_empty() {
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new("What should we monitor?")
                                        .size(11.0)
                                        .strong()
                                        .color(TEXT_SECONDARY),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&ctx.monitor)
                                            .size(11.0)
                                            .color(TEXT_SECONDARY),
                                    )
                                    .wrap(),
                                );
                            }
                        }
                    }
                });
            ui.add_space(4.0);
        }

        // Re-evaluate & Reset buttons
        ui.add_space(8.0);
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
    }

}

// ---------------------------------------------------------------------------
// UI layout
// ---------------------------------------------------------------------------

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

        self.poll_refresh();
        self.poll_ai();
        self.poll_outcomes_reroll();
        self.poll_report();
        self.poll_folds();
        sanitize_overlay_selection(&mut self.settings);
        self.refresh_analysis_cache();

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

        // -- Help window (accessible from any tab) --
        if self.show_help {
            let screen = ctx.screen_rect();
            let win_width = (screen.width() * 0.7).min(900.0).max(500.0);
            let win_height = (screen.height() * 0.85).min(800.0);
            egui::Window::new("The Hedgehog - Help")
                .open(&mut self.show_help)
                .default_size([win_width, win_height])
                .default_pos([
                    (screen.width() - win_width) / 2.0,
                    (screen.height() - win_height) / 2.0,
                ])
                .resizable(true)
                .collapsible(false)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        egui_commonmark::CommonMarkViewer::new()
                            .show(ui, &mut self.help_cache, help::HELP_TEXT);
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
                    ui.separator();
                    if ui.button("Report").clicked() {
                        self.show_report_window = !self.show_report_window;
                    }
                }

                // Model sub-tabs (only when 51Folds tab is active)
                if self.central_view == CentralView::Model {
                    ui.selectable_value(&mut self.model_tab, ModelTab::Outcome, "Outcome");
                    ui.selectable_value(&mut self.model_tab, ModelTab::Drivers, "Drivers");
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
                                for inf in &self.inference_history {
                                    let level_color = match inf.vix_level.as_deref() {
                                        Some("extreme") => ALERT_EXTREME_FG,
                                        Some("approaching_extreme") => ALERT_APPROACHING_FG,
                                        _ => ALERT_NORMAL_FG,
                                    };
                                    let short = inference_label_short(inf);
                                    let full = inference_label_full(inf);
                                    let display = truncate_with_ellipsis(&short, 50);
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
                                        let resp = ui
                                            .add(
                                                egui::Label::new(
                                                    RichText::new(&display)
                                                        .size(10.0)
                                                        .color(TEXT_SECONDARY),
                                                )
                                                .sense(Sense::click()),
                                            )
                                            .on_hover_text(&full);
                                        if resp.clicked() {
                                            load_inference = Some(inf.clone());
                                        }
                                    });
                                }
                                if let Some(inf) = load_inference {
                                    self.load_historical_inference(inf);
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
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.central_view {
                CentralView::Charts => {
                    let has_any_data = !self.series(Instrument::Vix).is_empty();

                    if !has_any_data {
                        empty_state_panel(ui, self.refresh_in_flight);
                        return;
                    }

                    if let Some(status) = &self.cached_vix_status {
                        status_banner(ui, status);
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
                }
                CentralView::Model => {
                    self.render_central_model_view(ui);
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

        // -- Report window --
        if self.show_report_window {
            let screen = ctx.screen_rect();
            let win_width = (screen.width() * 0.75).min(1000.0).max(600.0);
            let win_height = (screen.height() * 0.85).min(900.0);
            let mut start_report = false;
            let mut load_inferences = false;
            // Hoisted out of the show() closure so we can call
            // self.load_historical_inference(...) AFTER the .open()
            // borrow of self.show_report_window has been released.
            let mut load_history: Option<SavedInference> = None;
            egui::Window::new("Summary Report")
                .open(&mut self.show_report_window)
                .default_size([win_width, win_height])
                .default_pos([
                    (screen.width() - win_width) / 2.0,
                    (screen.height() - win_height) / 2.0,
                ])
                .resizable(true)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("From:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.report_from)
                                .desired_width(100.0)
                                .hint_text("YYYY-MM-DD"),
                        );
                        ui.label("To:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.report_to)
                                .desired_width(100.0)
                                .hint_text("YYYY-MM-DD"),
                        );
                    });
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
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button("Load Inferences").clicked() {
                            load_inferences = true;
                        }
                        if !self.report_inferences.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "{} inferences loaded",
                                    self.report_inferences.len()
                                ))
                                .size(11.0)
                                .color(TEXT_SECONDARY),
                            );
                            if self.report_task.in_flight {
                                ui.spinner();
                            } else if ui.button("Generate Report").clicked() {
                                start_report = true;
                            }
                        }
                    });
                    ui.separator();

                    if let Some(ref err) = self.report_task.error {
                        ui.label(
                            RichText::new(err)
                                .color(ALERT_EXTREME_FG)
                                .size(12.0),
                        );
                    }

                    if self.report_task.in_flight {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                RichText::new("Generating report...")
                                    .size(12.0)
                                    .color(TEXT_SECONDARY),
                            );
                        });
                    } else if let Some(ref report) = self.report_result {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            egui_commonmark::CommonMarkViewer::new()
                                .show(ui, &mut self.report_markdown_cache, report);
                        });
                    } else if !self.report_inferences.is_empty() {
                        // Browsable inference list
                        ui.label(
                            RichText::new("Loaded Inferences")
                                .strong()
                                .size(12.0)
                                .color(TEXT_SECONDARY),
                        );
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .max_height(400.0)
                            .show(ui, |ui| {
                                for inf in &self.report_inferences {
                                    let level_color = match inf.vix_level.as_deref() {
                                        Some("extreme") => ALERT_EXTREME_FG,
                                        Some("approaching_extreme") => ALERT_APPROACHING_FG,
                                        _ => ALERT_NORMAL_FG,
                                    };
                                    let label = inference_label(inf);
                                    let resp = ui.horizontal(|ui| {
                                        let (dot_rect, _) = ui.allocate_exact_size(
                                            Vec2::new(8.0, 8.0),
                                            Sense::hover(),
                                        );
                                        ui.painter().circle_filled(
                                            dot_rect.center(),
                                            4.0,
                                            level_color,
                                        );
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(label)
                                                    .size(11.0)
                                                    .color(APP_BG),
                                            )
                                            .sense(Sense::click()),
                                        )
                                    });
                                    if resp.inner.clicked() {
                                        load_history = Some(inf.clone());
                                    }
                                }
                            });
                    }
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
    ui.heading("Compare Against VIX");

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

fn status_banner(ui: &mut egui::Ui, status: &VixStatus) {
    let accent = match status.level {
        AlertLevel::Normal => ALERT_NORMAL_FG,
        AlertLevel::ApproachingExtreme => ALERT_APPROACHING_FG,
        AlertLevel::Extreme => ALERT_EXTREME_FG,
    };

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
            });
        });

    // 3-px coloured left-accent bar painted over the frame edge.
    let r = outer.response.rect;
    ui.painter().rect_filled(
        Rect::from_min_max(r.min, Pos2::new(r.min.x + 3.0, r.max.y)),
        egui::CornerRadius::same(6),
        accent,
    );
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
            "Select assets in the sidebar to compare against VIX."
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
        .unwrap_or_else(|| "—".to_owned());

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
