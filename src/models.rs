use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssetGroup {
    Volatility,
    Monetary,
    Energy,
}

impl AssetGroup {
    pub const ALL: [AssetGroup; 3] = [
        AssetGroup::Volatility,
        AssetGroup::Monetary,
        AssetGroup::Energy,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Volatility => "Volatility",
            Self::Monetary => "Monetary / Store of Value",
            Self::Energy => "Energy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Instrument {
    Vix,
    Gold,
    Silver,
    Bitcoin,
    CrudeOil,
    NaturalGas,
}

impl Instrument {
    pub const ALL: [Instrument; 6] = [
        Instrument::Vix,
        Instrument::Gold,
        Instrument::Silver,
        Instrument::Bitcoin,
        Instrument::CrudeOil,
        Instrument::NaturalGas,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vix => "VIX",
            Self::Gold => "Gold",
            Self::Silver => "Silver",
            Self::Bitcoin => "Bitcoin",
            Self::CrudeOil => "Crude Oil",
            Self::NaturalGas => "Natural Gas",
        }
    }

    pub fn storage_key(self) -> &'static str {
        match self {
            Self::Vix => "vix",
            Self::Gold => "gold",
            Self::Silver => "silver",
            Self::Bitcoin => "bitcoin",
            Self::CrudeOil => "crude_oil",
            Self::NaturalGas => "natural_gas",
        }
    }

    pub fn group_members(group: AssetGroup) -> &'static [Instrument] {
        match group {
            AssetGroup::Volatility => &[Instrument::Vix],
            AssetGroup::Monetary => &[Instrument::Gold, Instrument::Silver, Instrument::Bitcoin],
            AssetGroup::Energy => &[Instrument::CrudeOil, Instrument::NaturalGas],
        }
    }
}

impl fmt::Display for Instrument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Observation {
    pub instrument: Instrument,
    pub date: NaiveDate,
    pub close: f64,
    pub source: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertLevel {
    Normal,
    ApproachingExtreme,
    Extreme,
}

impl AlertLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::ApproachingExtreme => "Approaching Extreme",
            Self::Extreme => "Extreme",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThresholdMode {
    RollingPercentile,
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChartWindow {
    OneMonth,
    ThreeMonths,
    SixMonths,
    OneYear,
    All,
}

impl ChartWindow {
    pub const ALL: [ChartWindow; 5] = [
        ChartWindow::OneMonth,
        ChartWindow::ThreeMonths,
        ChartWindow::SixMonths,
        ChartWindow::OneYear,
        ChartWindow::All,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::OneMonth => "1M",
            Self::ThreeMonths => "3M",
            Self::SixMonths => "6M",
            Self::OneYear => "1Y",
            Self::All => "All",
        }
    }

    pub fn approx_days(self) -> Option<usize> {
        match self {
            Self::OneMonth => Some(31),
            Self::ThreeMonths => Some(92),
            Self::SixMonths => Some(183),
            Self::OneYear => Some(366),
            Self::All => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThresholdConfig {
    pub mode: ThresholdMode,
    pub lookback_days: usize,
    pub fixed_approaching: f64,
    pub fixed_extreme: f64,
    pub percentile_approaching: f64,
    pub percentile_extreme: f64,
}

impl ThresholdConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.lookback_days == 0 {
            return Err("Lookback days must be greater than 0".into());
        }
        if self.fixed_approaching >= self.fixed_extreme {
            return Err("Fixed approaching threshold must be less than extreme".into());
        }
        if self.percentile_approaching >= self.percentile_extreme {
            return Err("Approaching percentile must be less than extreme percentile".into());
        }
        if self.percentile_extreme > 100.0 {
            return Err("Percentile extreme cannot exceed 100".into());
        }
        Ok(())
    }
}

impl Default for ThresholdConfig {
    fn default() -> Self {
        Self {
            mode: ThresholdMode::RollingPercentile,
            lookback_days: 252,
            fixed_approaching: 25.0,
            fixed_extreme: 35.0,
            percentile_approaching: 85.0,
            percentile_extreme: 95.0,
        }
    }
}

/// A structured hypothesis extracted from the LLM analysis response.
#[derive(Debug, Clone)]
pub struct ParsedHypothesis {
    pub question: String,
    pub outcomes: Vec<String>,
    pub context: String,
}

/// Runtime-only API key store. Never serialised — loaded from environment
/// variables (via `.env`) and written back to `.env` on save.
#[derive(Debug, Clone, Default)]
pub struct ApiKeys {
    pub fred: String,
    pub alpha_vantage: String,
    pub anthropic: String,
    pub openai: String,
    pub folds: String,
}

impl ApiKeys {
    pub fn from_env() -> Self {
        Self {
            fred: std::env::var("FRED_API_KEY").unwrap_or_default(),
            alpha_vantage: std::env::var("ALPHA_VANTAGE_API_KEY").unwrap_or_default(),
            anthropic: std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            openai: std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            folds: std::env::var("FOLDS_API_KEY").unwrap_or_default(),
        }
    }

    pub fn has_folds(&self) -> bool {
        !self.folds.trim().is_empty()
    }

    pub fn has_fred(&self) -> bool {
        !self.fred.trim().is_empty()
    }

    pub fn has_alpha_vantage(&self) -> bool {
        !self.alpha_vantage.trim().is_empty()
    }

    pub fn all_empty(&self) -> bool {
        !self.has_fred() && !self.has_alpha_vantage()
    }

    pub fn ai_key_for(&self, provider: LlmProvider) -> &str {
        match provider {
            LlmProvider::Anthropic => &self.anthropic,
            LlmProvider::OpenAI => &self.openai,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AiPanelDock {
    #[default]
    Bottom,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub threshold_config: ThresholdConfig,
    pub chart_window: ChartWindow,
    pub show_overlay: bool,
    pub auto_refresh_on_startup: bool,
    pub selected_group: AssetGroup,
    pub selected_instrument: Instrument,
    pub overlay_instruments: Vec<Instrument>,
    pub overlay_include_selected_group_index: bool,
    pub ai_provider: LlmProvider,
    pub ai_model_anthropic: String,
    pub ai_model_openai: String,
    pub ai_panel_dock: AiPanelDock,
    pub research_provider: LlmProvider,
}

impl AppSettings {
    pub fn effective_model(&self) -> &str {
        match self.ai_provider {
            LlmProvider::Anthropic => &self.ai_model_anthropic,
            LlmProvider::OpenAI => &self.ai_model_openai,
        }
    }

    pub fn effective_model_mut(&mut self) -> &mut String {
        match self.ai_provider {
            LlmProvider::Anthropic => &mut self.ai_model_anthropic,
            LlmProvider::OpenAI => &mut self.ai_model_openai,
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            threshold_config: ThresholdConfig::default(),
            chart_window: ChartWindow::SixMonths,
            show_overlay: true,
            auto_refresh_on_startup: true,
            selected_group: AssetGroup::Monetary,
            selected_instrument: Instrument::Gold,
            overlay_instruments: vec![Instrument::Gold, Instrument::Silver, Instrument::Bitcoin],
            overlay_include_selected_group_index: true,
            ai_provider: LlmProvider::default(),
            ai_model_anthropic: LlmProvider::Anthropic.default_model().to_owned(),
            ai_model_openai: LlmProvider::OpenAI.default_model().to_owned(),
            ai_panel_dock: AiPanelDock::Right,
            research_provider: LlmProvider::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ThresholdSnapshot {
    pub approaching: f64,
    pub extreme: f64,
}

#[derive(Debug, Clone)]
pub struct VixStatus {
    pub latest: Observation,
    pub level: AlertLevel,
    pub thresholds: ThresholdSnapshot,
}

#[derive(Debug, Clone)]
pub struct AlertEvent {
    pub timestamp_utc: DateTime<Utc>,
    pub instrument: Instrument,
    pub level: AlertLevel,
    pub note: String,
}

#[derive(Debug, Clone)]
pub struct ObservationBatch {
    pub instrument: Instrument,
    #[allow(dead_code)]
    pub source: &'static str,
    pub observations: Vec<Observation>,
}

pub enum RefreshEvent {
    Fetching { instrument: Instrument, source: String },
    Fetched(ObservationBatch),
    Cached { instrument: Instrument, source: String, date: String },
    FetchFailed { instrument: Instrument, source: String, error: String },
    Done,
}

// ---------------------------------------------------------------------------
// AI / LLM provider types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LlmProvider {
    #[default]
    Anthropic,
    OpenAI,
}

impl LlmProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Anthropic => "Claude (Anthropic)",
            Self::OpenAI => "GPT (OpenAI)",
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Self::Anthropic => "claude-sonnet-4-6",
            Self::OpenAI => "gpt-5.4",
        }
    }

    pub fn storage_key(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAI => "openai",
        }
    }
}

pub struct AiInferenceResult {
    pub provider: String,
    pub model: String,
    pub system_prompt: String,
    pub user_message: String,
    pub response: String,
}

pub enum AiEvent {
    Response(AiInferenceResult),
    Failed(String),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SavedInference {
    pub id: i64,
    pub created_at: String,
    pub provider: String,
    pub model: String,
    pub response: String,
    pub vix_close: Option<f64>,
    pub vix_level: Option<String>,
    /// Parsed hypothesis fields, captured at save time so we don't have to
    /// re-parse the markdown response on every load. Optional because (a)
    /// pre-migration rows have NULL and (b) malformed LLM responses may
    /// not parse cleanly.
    pub hypothesis_question: Option<String>,
    pub hypothesis_outcomes: Option<Vec<String>>,
    pub hypothesis_context: Option<String>,
    /// Storage keys of the instruments selected for comparison at the
    /// time of analysis (e.g. ["gold","silver","bitcoin"]). Used to give
    /// list entries a distinguishing label so two analyses run on the
    /// same regime but different instruments are not visually identical.
    pub overlay_instruments: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// 51Folds model tracking
// ---------------------------------------------------------------------------

/// Status of a 51Folds model creation request as we track it locally. The
/// 51Folds API itself only knows about "Successed" and "Failed" — the
/// `pending` and `undisclosed_failure` states are local: `pending` means we
/// are still polling, and `undisclosed_failure` is what we mark a row as
/// when it has been pending for more than 35 minutes and we have given up.
pub const FOLDS_STATUS_PENDING: &str = "pending";
pub const FOLDS_STATUS_SUCCESS: &str = "success";
pub const FOLDS_STATUS_FAIL: &str = "fail";
pub const FOLDS_STATUS_UNDISCLOSED_FAILURE: &str = "undisclosed_failure";

/// "Suspect" is a derived status — never persisted. A model is suspect when
/// it is still `pending` and has been alive for more than this duration.
/// Advanced-tier builds in practice take 45-75 minutes (observed
/// 2026-04-16), so 60 minutes is the "this is taking a while" threshold.
pub const FOLDS_SUSPECT_AFTER_SECS: i64 = 60 * 60; // 60 minutes
/// Pending models older than this are marked as `undisclosed_failure` and
/// polling stops. Two hours gives enough headroom for legitimate long
/// builds while still cutting off anything genuinely stuck.
pub const FOLDS_UNDISCLOSED_AFTER_SECS: i64 = 120 * 60; // 2 hours

/// One row of the `folds_models` table — a 51Folds model the app has asked
/// to be created, with whatever status we last observed.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FoldsModelRecord {
    pub id: i64,
    pub model_id: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub last_polled_at: Option<DateTime<Utc>>,
    pub question: String,
    pub inference_id: Option<i64>,
}

impl FoldsModelRecord {
    /// True when the row is still pending AND has been pending for at
    /// least one hour. Computed at read time, never persisted. The
    /// `undisclosed_failure` transition is a separate concern handled by
    /// the polling thread / resume sweep — once that runs the row is no
    /// longer `pending`, so this check naturally returns false.
    pub fn is_suspect(&self, now: DateTime<Utc>) -> bool {
        if self.status != FOLDS_STATUS_PENDING {
            return false;
        }
        (now - self.created_at).num_seconds() >= FOLDS_SUSPECT_AFTER_SECS
    }
}

/// One row of the `folds_themes` table — a user-managed bucket that
/// groups related Bayesian models on the 51Folds tab's cards landing.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FoldsTheme {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub sort_order: i64,
    pub created_at: DateTime<Utc>,
}

/// Reserved theme name — every install ships with this, and deleting
/// a theme reassigns its models here so nothing is ever orphaned.
pub const UNCATEGORIZED_THEME_NAME: &str = "Uncategorized";
