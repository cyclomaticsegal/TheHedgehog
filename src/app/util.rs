//! Pure helpers extracted from `app.rs` — no `DashboardApp` coupling, no
//! egui state, just data → data transforms used across the dashboard.
//!
//! When adding new helpers here keep the module a true leaf: depend on
//! `std`, `chrono`, `eframe::egui` (only stateless utilities like `lerp`),
//! and crate-level `models` types. If you reach for `super::*` you've
//! picked the wrong file.

use crate::models::{ChartWindow, Instrument, Observation};
use eframe::egui;
use std::path::PathBuf;

pub(super) fn database_path() -> PathBuf {
    let mut path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    path.push("data");
    path.push("regime_shift_dashboard.sqlite3");
    path
}

/// POSIX-safe single-quote escape: wrap the value in `'…'`, replacing any
/// embedded `'` with `'\''` (close, escape, reopen). Used when building
/// `sh -c` command strings so stray apostrophes can't break out.
pub(super) fn sh_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

pub(super) fn validate_model_name(name: &str) -> Result<(), String> {
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

/// Update or insert key=value pairs in `.env` file content, preserving all
/// other lines (comments, blank lines, unrelated variables).
pub(super) fn update_env_content(content: &str, updates: &[(&str, &str)]) -> String {
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

/// Truncate `s` to at most `max_chars` characters, appending an ellipsis
/// if the original was longer. Operates on Unicode scalar values, not
/// bytes, so it does not split a multi-byte character.
pub(super) fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let head: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{head}…")
}

/// Return the prefix of the LLM response up to (but not including) the
/// `**Hypothesis**:` marker.
pub(super) fn split_off_hypothesis(response: &str) -> &str {
    response
        .find("**Hypothesis**:")
        .map(|idx| response[..idx].trim_end())
        .unwrap_or(response)
}

pub(super) fn interpolate_at(points: &[(f64, f64)], target_x: f64) -> Option<f64> {
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

pub(super) fn map_val(value: f64, src_min: f64, src_max: f64, dst_min: f32, dst_max: f32) -> f32 {
    let ratio = ((value - src_min) / (src_max - src_min)).clamp(0.0, 1.0) as f32;
    egui::lerp(dst_min..=dst_max, ratio)
}

pub(super) fn filter_for_zoom<'a>(
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

/// Convert a list of instrument storage keys (e.g. ["gold","silver","bitcoin"])
/// into a compact display label (e.g. "Gold/Silver/Bitcoin"). Caps at 3
/// names and appends `+N` for the rest.
pub(super) fn format_overlay_label(keys: &[String]) -> String {
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

/// Launch Obsidian and open `path` as a vault. On macOS we use `open
/// -a Obsidian`, which registers a first-time-opened folder as a vault
/// automatically. Other platforms fall back to a best-effort
/// `obsidian://` URL.
pub(super) fn open_in_obsidian(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-a")
            .arg("Obsidian")
            .arg(path)
            .spawn()?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        // The `obsidian://open?path=` URL is a stable scheme handler.
        let url = format!(
            "obsidian://open?path={}",
            urlencoding_minimal(&path.to_string_lossy())
        );
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("cmd").args(["/C", "start", "", &url]).spawn()?;
        }
        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        {
            std::process::Command::new("xdg-open").arg(&url).spawn()?;
        }
        Ok(())
    }
}

/// Tiny URL-encoder for paths — enough for the `obsidian://` scheme on
/// non-macOS platforms. Spaces and a handful of structural characters,
/// nothing fancy.
#[cfg(not(target_os = "macos"))]
fn urlencoding_minimal(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' => out.push(c),
            _ => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    out
}

/// Open the OS file manager focused on `path`. Best-effort: errors are
/// returned to the caller, who decides whether to surface them.
pub(super) fn reveal_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()?;
        Ok(())
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        // Most Linux file managers don't accept a "select this file"
        // flag; open the parent directory instead.
        let target = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent().unwrap_or(path).to_path_buf()
        };
        std::process::Command::new("xdg-open").arg(target).spawn()?;
        Ok(())
    }
}

/// Run the apply phase of an Obsidian-vault merge on the calling thread.
///
/// PATCHes the user's selected driver state changes through the SDK
/// (blocking until re-inference completes), builds a `MergeAudit` from
/// the post-server `ModelResponse`, then runs `re_export_with_merge`
/// over the same vault path. Returns the `MergeEvent` the caller should
/// post back on the `MergeTask` channel.
///
/// Lives in `util.rs` rather than `app.rs` so the heavy thread-body
/// logic stays out of the dashboard struct.
pub(super) fn apply_merge_blocking(
    api_key: String,
    model_id: String,
    db_path: std::path::PathBuf,
    vault: std::path::PathBuf,
    diff: crate::obsidian::merge::VaultDiff,
    applied_changes: Vec<crate::obsidian::merge::DriverStateChange>,
) -> super::tasks::MergeEvent {
    use super::tasks::MergeEvent;

    let drivers: Vec<fiftyone_folds::DriverStateInput> = applied_changes
        .iter()
        .map(|c| fiftyone_folds::DriverStateInput {
            code: c.code.clone(),
            state: c.new_state.clone(),
        })
        .collect();

    let new_model = match crate::folds::merge_drivers(&api_key, &model_id, drivers) {
        Ok(m) => m,
        Err(msg) => return MergeEvent::Failed(msg),
    };

    // Persist the post-merge response to `folds_models.response_json`
    // so that re-opening this model from the registry (which reads
    // straight from that column) reflects the merged state instead of
    // silently reverting to the pre-merge build response.
    crate::folds::persist_completed(&db_path, &model_id, &new_model);

    let audit = match crate::obsidian::merge::build_audit_from(&diff, &applied_changes, &new_model)
    {
        Ok(a) => a,
        Err(e) => return MergeEvent::Failed(format!("Building merge audit: {e:#}")),
    };
    let version = audit.version;
    let applied_state_changes = audit.driver_state_changes.len();
    let parked_total = audit.parked_note_additions
        + audit.parked_metadata_changes
        + audit.parked_edge_changes;

    if let Err(e) = crate::obsidian::re_export_with_merge(&new_model, &vault, audit) {
        return MergeEvent::Failed(format!("Writing merged vault: {e:#}"));
    }

    MergeEvent::Applied {
        new_model: Box::new(new_model),
        version,
        applied_state_changes,
        parked_total,
    }
}
