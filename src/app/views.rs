//! View-state enums + log records for `DashboardApp`. These describe
//! which panel / detail page is currently showing and what category of
//! feedback banner to surface. No egui, no `DashboardApp` coupling —
//! pure data that the parent module matches on while rendering.
use crate::models::Instrument;

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
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ModelView {
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
