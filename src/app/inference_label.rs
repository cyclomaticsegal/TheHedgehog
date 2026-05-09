//! Compact one-line labels for `SavedInference` rows in the Report view
//! and the right-sidebar history list. Pure functions over inference
//! data; no UI state.

use super::util::format_overlay_label;
use crate::models::SavedInference;

/// Build a one-line label for an inference list entry. Format:
/// `MM-DD HH:MM  [Kind] VIX 23.9  Gold/Silver  · {hypothesis snippet}`
pub(super) fn inference_label(inf: &SavedInference) -> String {
    let header = inference_label_short(inf);
    let snippet = inference_hypothesis_snippet(inf, 60);
    if snippet.is_empty() {
        header
    } else {
        format!("{header}  · {snippet}")
    }
}

/// Compact header for an inference label — timestamp + kind + VIX +
/// overlay. Used as the prefix by `inference_label`.
pub(super) fn inference_label_short(inf: &SavedInference) -> String {
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
/// of the raw response.
pub(super) fn inference_hypothesis_snippet(inf: &SavedInference, max_chars: usize) -> String {
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
