//! View-state enums + log records for `DashboardApp`. These describe
//! which panel / detail page is currently showing and what category of
//! feedback banner to surface. No egui, no `DashboardApp` coupling —
//! pure data that the parent module matches on while rendering.
use crate::models::{FoldsTheme, Instrument};

/// Which view the central panel is showing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CentralView {
    Charts,
    Model,
    ResearchAgent,
    Report,
    Help,
}

/// Navigation stack within the 51Folds model explorer. Each variant is
/// a "page" in the central panel. The back button pops to the previous
/// level rather than needing explicit tab management.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum ModelView {
    /// Cards landing — one card per theme.
    /// Default landing state for the 51Folds tab.
    #[default]
    Browse,
    /// Paginated list of models within a single theme.
    ThemeList(i64),
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

/// Sort order applied to the per-theme model list.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[allow(clippy::enum_variant_names)]
pub(super) enum ThemeListSort {
    #[default]
    NewestFirst,
    OldestFirst,
    BuiltFirst,
    FailedFirst,
}

impl ThemeListSort {
    pub(super) fn label(self) -> &'static str {
        match self {
            ThemeListSort::NewestFirst => "Newest first",
            ThemeListSort::OldestFirst => "Oldest first",
            ThemeListSort::BuiltFirst => "Status: built first",
            ThemeListSort::FailedFirst => "Status: failed first",
        }
    }
    pub(super) const ALL: [ThemeListSort; 4] = [
        ThemeListSort::NewestFirst,
        ThemeListSort::OldestFirst,
        ThemeListSort::BuiltFirst,
        ThemeListSort::FailedFirst,
    ];
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DriverDetailSection {
    WhySelected,
    WhyMatters,
    WhatShift,
    WhatMonitor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StatusKind {
    Info,
    Success,
    Error,
}

#[derive(Clone)]
pub(super) struct LogEntry {
    pub(super) timestamp_str: String,
    pub(super) instrument: Instrument,
    pub(super) source: String,
    pub(super) status: LogStatus,
}

#[derive(Clone)]
pub(super) enum LogStatus {
    Fetching,
    Ok(usize),
    Cached(String),
    Failed(String),
}

pub(super) fn format_log_status(status: &LogStatus) -> String {
    match status {
        LogStatus::Fetching => String::new(),
        LogStatus::Ok(count) => format!("{count} pts"),
        LogStatus::Cached(date) => format!("cached ({})", date),
        LogStatus::Failed(err) => err.clone(),
    }
}

pub(super) enum PricePickerAction {
    StillOpen,
    Cancelled,
    Selected(Instrument),
}

/// Pre-aggregated card payload for the 51Folds Browse landing. Holds
/// the theme row, its current model count, and a small slice of
/// example questions (newest first) used as the card preview.
#[derive(Clone)]
pub(super) struct ThemeCardData {
    pub(super) theme: FoldsTheme,
    pub(super) count: i64,
    pub(super) sample_questions: Vec<String>,
}

/// One editable row inside the Manage Themes dialog. The dialog mutates
/// `draft_name` / `draft_description` directly; commit lands when the
/// field loses focus or the user presses Enter. `error` surfaces
/// collision feedback inline (e.g. "name already in use").
#[derive(Clone)]
pub(super) struct ThemeDraft {
    pub(super) id: i64,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) count: i64,
    /// Inline error to surface beneath the row.
    pub(super) error: Option<String>,
    /// True when this row's name has been edited but not yet committed.
    /// Lets us re-render the original name on focus loss without commit.
    pub(super) name_dirty: bool,
    pub(super) description_dirty: bool,
    /// The original (DB-side) values, kept so we can detect dirty vs.
    /// no-op edits and revert on cancel.
    pub(super) original_name: String,
    pub(super) original_description: String,
    /// True for the Uncategorized row — its name is locked (renaming
    /// would break the seed contract used by `persist_theme_assignment`).
    pub(super) locked: bool,
}

/// Events emitted by the Manage Themes dialog. One per frame; the caller
/// applies the matching DB write and updates draft state.
pub(super) enum ManageThemesEvent {
    /// User clicked the close button or pressed Escape.
    Close,
    /// User committed a rename (focus loss or Enter).
    Rename { theme_id: i64, new_name: String },
    /// User committed a description edit.
    UpdateDesc {
        theme_id: i64,
        new_description: String,
    },
    /// User clicked Delete; show the confirm overlay.
    DeleteRequest(i64),
    /// User clicked Confirm in the overlay.
    DeleteConfirm(i64),
    /// User clicked Cancel in the overlay.
    DeleteCancel,
}
