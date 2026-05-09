//! Design-system palette + typography constants for the dashboard UI.
//!
//! Every colour the app paints flows through this module. The constants
//! are `pub(super)` so sibling modules under `app::` can `use super::theme::*`
//! without exposing the palette outside the crate.

use crate::models::Instrument;
use eframe::egui::Color32;

// Backgrounds
pub(super) const APP_BG: Color32 = Color32::from_rgb(10, 14, 26);
pub(super) const PANEL_BG: Color32 = Color32::from_rgb(17, 24, 39);
pub(super) const SURFACE: Color32 = Color32::from_rgb(26, 34, 54);
pub(super) const SURFACE_HOVER: Color32 = Color32::from_rgb(34, 45, 66);
// Borders
pub(super) const BORDER: Color32 = Color32::from_rgb(45, 55, 72);
// Text
pub(super) const TEXT_PRIMARY: Color32 = Color32::from_rgb(226, 232, 240);
pub(super) const TEXT_SECONDARY: Color32 = Color32::from_rgb(148, 163, 184);
pub(super) const TEXT_MUTED: Color32 = Color32::from_rgb(74, 85, 104);
// Alert levels
pub(super) const ALERT_NORMAL_FG: Color32 = Color32::from_rgb(56, 161, 105);
pub(super) const ALERT_APPROACHING_FG: Color32 = Color32::from_rgb(214, 158, 46);
pub(super) const ALERT_EXTREME_FG: Color32 = Color32::from_rgb(229, 62, 62);
// Accent colors (51Folds model explorer)
pub(super) const ACCENT_BLUE: Color32 = Color32::from_rgb(96, 165, 250);
pub(super) const ACCENT_BLUE_DIM: Color32 = Color32::from_rgb(59, 130, 246);

/// Per-instrument line/swatch colour used in charts and the price picker.
pub(super) fn instrument_color(instrument: Instrument) -> Color32 {
    match instrument {
        Instrument::Vix => Color32::from_rgb(235, 106, 74),
        Instrument::Gold => Color32::from_rgb(232, 194, 86),
        Instrument::Silver => Color32::from_rgb(148, 190, 230),
        Instrument::Bitcoin => Color32::from_rgb(240, 149, 66),
        Instrument::CrudeOil => Color32::from_rgb(186, 109, 71),
        Instrument::NaturalGas => Color32::from_rgb(91, 168, 189),
    }
}
