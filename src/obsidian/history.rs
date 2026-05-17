//! Per-vault history — the per-merge audit trail and the inline
//! `## History` tables that surface that trail wherever the user is
//! reading.
//!
//! `Data/history.json` is the source of truth. The `## History` sections
//! emitted into each driver, outcome, and into `Overview.md` are *projections*
//! of that JSON regenerated on every export. The per-merge audit notes
//! at `Merges/<timestamp>.md` are also projections, written once per
//! merge and back-linked from the per-node history rows.
//!
//! Schema (`history.json#schema_version = 1`):
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "drivers":  { "UEBSP": [ { version, timestamp, before, after, merge_note } ] },
//!   "outcomes": {     "1": [ { version, timestamp, before_pct, after_pct, merge_note } ] },
//!   "merges":   [ { version, timestamp, model_id, applied_state_changes, parked_note_additions, parked_metadata_changes, parked_edge_changes } ]
//! }
//! ```

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub(crate) const HISTORY_SCHEMA_VERSION: u32 = 1;
pub(crate) const HISTORY_FILENAME: &str = "history.json";
pub(crate) const MERGES_DIRNAME: &str = "Merges";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct History {
    pub schema_version: u32,
    /// Driver code → ordered list of state-change rows, oldest first.
    pub drivers: BTreeMap<String, Vec<DriverHistoryRow>>,
    /// Outcome id → ordered list of probability-shift rows, oldest first.
    pub outcomes: BTreeMap<i64, Vec<OutcomeHistoryRow>>,
    /// All merges in chronological order, oldest first. The Overview's
    /// `## History` section renders this list newest-first for the user.
    pub merges: Vec<MergeSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DriverHistoryRow {
    pub version: u32,
    pub timestamp: DateTime<Utc>,
    pub before: String,
    pub after: String,
    /// Wiki-link target without the `[[ ]]` braces, e.g.
    /// `Merges/2026-05-15-1042`.
    pub merge_note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OutcomeHistoryRow {
    pub version: u32,
    pub timestamp: DateTime<Utc>,
    pub before_pct: f64,
    pub after_pct: f64,
    pub merge_note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MergeSummary {
    pub version: u32,
    pub timestamp: DateTime<Utc>,
    pub model_id: String,
    pub applied_state_changes: usize,
    pub parked_note_additions: usize,
    pub parked_metadata_changes: usize,
    pub parked_edge_changes: usize,
    /// Wiki-link target without the `[[ ]]` braces.
    pub merge_note: String,
}

impl History {
    pub(crate) fn empty() -> Self {
        Self {
            schema_version: HISTORY_SCHEMA_VERSION,
            ..Self::default()
        }
    }
}

/// Read `Data/history.json`, or return an empty `History` if the file
/// doesn't exist yet (fresh vault). Schema-mismatched files are an
/// error — the user must re-export to migrate.
pub(crate) fn read_or_init(vault_root: &Path) -> Result<History> {
    let path = vault_root.join("Data").join(HISTORY_FILENAME);
    if !path.exists() {
        return Ok(History::empty());
    }
    let bytes = std::fs::read(&path)
        .with_context(|| format!("reading history {}", path.display()))?;
    let parsed: History = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing history {}", path.display()))?;
    if parsed.schema_version != HISTORY_SCHEMA_VERSION {
        anyhow::bail!(
            "{} has schema_version {}, expected {} — re-export the vault to migrate",
            path.display(),
            parsed.schema_version,
            HISTORY_SCHEMA_VERSION
        );
    }
    Ok(parsed)
}

pub(crate) fn write(vault_root: &Path, history: &History) -> Result<()> {
    let dir = vault_root.join("Data");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(HISTORY_FILENAME);
    let pretty = serde_json::to_string_pretty(history)?;
    std::fs::write(&path, pretty)
        .with_context(|| format!("writing history {}", path.display()))?;
    Ok(())
}

/// Format the timestamp into the per-merge filename component (no `.md`).
pub(crate) fn merge_note_slug(ts: &DateTime<Utc>) -> String {
    ts.format("%Y-%m-%d-%H%M").to_string()
}

/// Write `Merges/<slug>.md`. Returns the relative wiki-link target
/// (e.g. `Merges/2026-05-15-1042`).
pub(crate) fn write_merge_note(vault_root: &Path, audit: &MergeAudit) -> Result<String> {
    let dir = vault_root.join(MERGES_DIRNAME);
    std::fs::create_dir_all(&dir)?;
    let slug = merge_note_slug(&audit.timestamp);
    let path = dir.join(format!("{slug}.md"));
    std::fs::write(&path, render_merge_note(audit))
        .with_context(|| format!("writing merge note {}", path.display()))?;
    Ok(format!("{MERGES_DIRNAME}/{slug}"))
}

/// Inputs the merge action collects so the audit note and history rows
/// can be written together. All fields are post-server-reinference.
pub(crate) struct MergeAudit {
    pub version: u32,
    pub timestamp: DateTime<Utc>,
    pub model_id: String,
    pub driver_state_changes: Vec<DriverStateChangeAudit>,
    pub outcome_shifts: Vec<OutcomeShiftAudit>,
    pub parked_note_additions: usize,
    pub parked_metadata_changes: usize,
    pub parked_edge_changes: usize,
}

pub(crate) struct DriverStateChangeAudit {
    pub code: String,
    pub name: String,
    pub before: String,
    pub after: String,
}

pub(crate) struct OutcomeShiftAudit {
    pub id: i64,
    pub label: String,
    pub before_pct: f64,
    pub after_pct: f64,
}

fn render_merge_note(audit: &MergeAudit) -> String {
    let mut s = String::new();
    s.push_str("---\n");
    s.push_str(&format!("version: {}\n", audit.version));
    s.push_str(&format!(
        "timestamp: {}\n",
        audit.timestamp.format("%Y-%m-%dT%H:%M:%SZ")
    ));
    s.push_str(&format!("model_id: {}\n", audit.model_id));
    s.push_str("tags: [merge]\n");
    s.push_str("---\n\n");

    s.push_str(&format!("# Merge v{:03}\n\n", audit.version));
    s.push_str(&format!(
        "> [!info] {}\n> Model `{}` re-elicited after merge from vault.\n\n",
        audit.timestamp.format("%Y-%m-%d %H:%M UTC"),
        audit.model_id
    ));

    s.push_str("## Applied — driver state changes\n\n");
    if audit.driver_state_changes.is_empty() {
        s.push_str("_No state changes applied._\n\n");
    } else {
        s.push_str("| Driver | Before | After |\n|---|---|---|\n");
        for c in &audit.driver_state_changes {
            s.push_str(&format!(
                "| {} ({}) | {} | {} |\n",
                escape_pipe(&c.name),
                c.code,
                escape_pipe(&c.before),
                escape_pipe(&c.after),
            ));
        }
        s.push('\n');
    }

    s.push_str("## Outcome shifts\n\n");
    if audit.outcome_shifts.is_empty() {
        s.push_str("_No outcome probabilities moved._\n\n");
    } else {
        s.push_str("| Outcome | Before | After | Δ |\n|---|---:|---:|---:|\n");
        for o in &audit.outcome_shifts {
            let delta = o.after_pct - o.before_pct;
            s.push_str(&format!(
                "| {} (#{}) | {:.2}% | {:.2}% | {:+.2} |\n",
                escape_pipe(&o.label),
                o.id,
                o.before_pct,
                o.after_pct,
                delta,
            ));
        }
        s.push('\n');
    }

    s.push_str("## Detected, not applied (pending SDK support)\n\n");
    let total_parked = audit.parked_note_additions
        + audit.parked_metadata_changes
        + audit.parked_edge_changes;
    if total_parked == 0 {
        s.push_str("_None._\n");
    } else {
        s.push_str(&format!(
            "- Note additions: {} (pending Q1 — `submit_evidence()` semantics)\n",
            audit.parked_note_additions
        ));
        s.push_str(&format!(
            "- Driver metadata edits: {} (pending Q2 — metadata PATCH endpoint)\n",
            audit.parked_metadata_changes
        ));
        s.push_str(&format!(
            "- Edge changes: {} (pending Q3 — edge mutation endpoint)\n",
            audit.parked_edge_changes
        ));
        s.push_str(
            "\nThese stayed in the vault and will surface in the next merge preview until the matching SDK path lands.\n",
        );
    }

    s
}

fn escape_pipe(s: &str) -> String {
    s.replace('|', "\\|")
}

// ---------------------------------------------------------------------------
// Inline `## History` section renderers
// ---------------------------------------------------------------------------

/// Renders the `## History` section body for a single driver. Includes
/// the heading. Used by `driver.rs` when emitting each driver page.
pub(crate) fn render_driver_section(rows: &[DriverHistoryRow]) -> String {
    let mut s = String::from("## History\n\n");
    if rows.is_empty() {
        s.push_str(
            "_No merges yet — this driver's state journey will appear here after the first merge from the vault._\n\n",
        );
        return s;
    }
    s.push_str("| Version | Timestamp | Before | After | Merge |\n|---|---|---|---|---|\n");
    for r in rows.iter().rev() {
        s.push_str(&format!(
            "| v{:03} | {} | {} | {} | [[{}]] |\n",
            r.version,
            r.timestamp.format("%Y-%m-%d %H:%M"),
            escape_pipe(&r.before),
            escape_pipe(&r.after),
            r.merge_note,
        ));
    }
    s.push('\n');
    s
}

pub(crate) fn render_outcome_section(rows: &[OutcomeHistoryRow]) -> String {
    let mut s = String::from("## History\n\n");
    if rows.is_empty() {
        s.push_str(
            "_No merges yet — this outcome's probability trajectory will appear here after the first merge from the vault._\n\n",
        );
        return s;
    }
    s.push_str("| Version | Timestamp | Before | After | Δ | Merge |\n|---|---|---:|---:|---:|---|\n");
    for r in rows.iter().rev() {
        let delta = r.after_pct - r.before_pct;
        s.push_str(&format!(
            "| v{:03} | {} | {:.2}% | {:.2}% | {:+.2} | [[{}]] |\n",
            r.version,
            r.timestamp.format("%Y-%m-%d %H:%M"),
            r.before_pct,
            r.after_pct,
            delta,
            r.merge_note,
        ));
    }
    s.push('\n');
    s
}

/// Renders the `## History` section body for `Overview.md`. One row per
/// merge across the whole vault.
pub(crate) fn render_overview_section(merges: &[MergeSummary]) -> String {
    let mut s = String::from("## History\n\n");
    if merges.is_empty() {
        s.push_str(
            "_No merges yet — press **Merge from Vault** in Hedgehog after editing this vault to record the first one._\n",
        );
        return s;
    }
    s.push_str("| Version | Timestamp | Applied | Parked | Merge note |\n|---|---|---:|---:|---|\n");
    for m in merges.iter().rev() {
        let parked = m.parked_note_additions
            + m.parked_metadata_changes
            + m.parked_edge_changes;
        s.push_str(&format!(
            "| v{:03} | {} | {} | {} | [[{}]] |\n",
            m.version,
            m.timestamp.format("%Y-%m-%d %H:%M"),
            m.applied_state_changes,
            parked,
            m.merge_note,
        ));
    }
    s
}

// ---------------------------------------------------------------------------
// User-zone extraction — used by the selective re-export path
// ---------------------------------------------------------------------------

/// Returns the markdown text of every H2 section in `body` whose
/// heading is *not* in `known_model_headings`. The text returned for
/// each section includes the heading line and all content up to (but
/// not including) the next H2.
///
/// Used during selective re-export to preserve user-authored sections
/// (e.g. `## Notes`) when overwriting model-derived sections. Sections
/// are returned in their original order, joined by a single blank line
/// between them — caller appends the block at the end of the new
/// model-derived content.
pub(crate) fn extract_user_zones(body: &str, known_model_headings: &[&str]) -> String {
    let known: std::collections::HashSet<&str> = known_model_headings.iter().copied().collect();
    let mut out = String::new();
    let mut current: Option<String> = None;
    let mut buf = String::new();
    let flush = |head: &Option<String>, buf: &str, out: &mut String| {
        let Some(h) = head else { return };
        if known.contains(h.as_str()) {
            return;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(buf.trim_end());
        out.push('\n');
    };
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            // Flush the previous section before starting a new one.
            flush(&current, &buf, &mut out);
            buf.clear();
            buf.push_str(line);
            buf.push('\n');
            current = Some(rest.trim().to_owned());
        } else if current.is_some() {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    flush(&current, &buf, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_user_zones_keeps_unknown_sections() {
        let body = "intro\n\n## Possible States\n\nA\nB\n\n## Notes\n\nuser note\n\n## In The Model\n\nfoo\n";
        let kept = extract_user_zones(body, &["Possible States", "In The Model"]);
        assert!(kept.contains("## Notes"), "{kept}");
        assert!(kept.contains("user note"), "{kept}");
        assert!(!kept.contains("Possible States"), "{kept}");
        assert!(!kept.contains("In The Model"), "{kept}");
    }

    #[test]
    fn extract_user_zones_preserves_multiple_user_sections_in_order() {
        let body =
            "## Possible States\nx\n\n## Notes\nfirst\n\n## Custom Block\n[[Foo]]\n\n## In The Model\nz\n";
        let kept = extract_user_zones(body, &["Possible States", "In The Model"]);
        let notes_at = kept.find("## Notes").unwrap();
        let custom_at = kept.find("## Custom Block").unwrap();
        assert!(notes_at < custom_at, "order preserved: {kept}");
    }

    #[test]
    fn extract_user_zones_returns_empty_when_no_user_sections() {
        let body = "## Possible States\nx\n\n## In The Model\nz\n";
        let kept = extract_user_zones(body, &["Possible States", "In The Model"]);
        assert!(kept.trim().is_empty(), "{kept:?}");
    }

    #[test]
    fn render_driver_section_empty_placeholder() {
        let s = render_driver_section(&[]);
        assert!(s.starts_with("## History"));
        assert!(s.contains("_No merges yet"), "{s}");
    }

    #[test]
    fn render_driver_section_newest_first() {
        let rows = vec![
            DriverHistoryRow {
                version: 2,
                timestamp: Utc::now(),
                before: "A".into(),
                after: "B".into(),
                merge_note: "Merges/x".into(),
            },
            DriverHistoryRow {
                version: 3,
                timestamp: Utc::now(),
                before: "B".into(),
                after: "C".into(),
                merge_note: "Merges/y".into(),
            },
        ];
        let s = render_driver_section(&rows);
        let v003_at = s.find("v003").expect("v003 present");
        let v002_at = s.find("v002").expect("v002 present");
        assert!(v003_at < v002_at, "newest first: {s}");
    }

    #[test]
    fn merge_note_slug_format() {
        let ts = chrono::DateTime::parse_from_rfc3339("2026-05-15T10:42:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(merge_note_slug(&ts), "2026-05-15-1042");
    }
}
