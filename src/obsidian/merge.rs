//! Read an exported Obsidian vault back and compute what's changed since
//! the last snapshot.
//!
//! Pure function: no network, no UI, no SDK. The merge orchestration
//! consumes the [`VaultDiff`] this returns, shows it to the user in a
//! preview dialog (slice 5), and — on confirm — feeds the
//! `driver_state_changes` into `patch_drivers()` and triggers the
//! selective re-export with the resulting audit.
//!
//! Diff sources:
//! - **Driver state changes** come from comparing each driver
//!   markdown's `current_state:` frontmatter against the latest
//!   snapshot's `driver_states`. These are the only edits v1 can
//!   actually round-trip to the server.
//! - **Note additions** come from a fresh `## Notes` section in any
//!   driver file. Surfaced in the preview as parked (Q1).
//! - **Driver metadata changes** are detected against the original
//!   names in `Data/model.json`. Surfaced as parked (Q2).
//! - **Edge changes** are wiki-links to *other drivers* found inside
//!   `## Notes` sections — heuristic, but the only signal we have for
//!   "the user wanted to add an edge here." Surfaced as parked (Q3).

use crate::obsidian::{history, snapshot};
use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct VaultDiff {
    pub model_id: String,
    pub vault_path: PathBuf,
    /// Snapshot version we read as the baseline (e.g. `2` if the latest
    /// snapshot file was `v002.json`). The next merge will write
    /// `v{baseline_version + 1}.json`.
    pub baseline_version: u32,
    pub driver_state_changes: Vec<DriverStateChange>,
    pub note_additions: Vec<NoteAddition>,
    pub driver_metadata_changes: Vec<DriverMetadataChange>,
    pub edge_changes: Vec<EdgeChange>,
}

impl VaultDiff {
    pub fn total_parked(&self) -> usize {
        self.note_additions.len()
            + self.driver_metadata_changes.len()
            + self.edge_changes.len()
    }

    pub fn next_version(&self) -> u32 {
        self.baseline_version + 1
    }
}

#[derive(Debug, Clone)]
pub struct DriverStateChange {
    pub code: String,
    pub name: String,
    pub old_state: String,
    pub new_state: String,
}

#[derive(Debug, Clone)]
pub struct NoteAddition {
    pub driver_code: String,
    pub driver_name: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct DriverMetadataChange {
    pub code: String,
    pub field: MetadataField,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataField {
    Name,
}

#[derive(Debug, Clone)]
pub struct EdgeChange {
    pub in_driver_code: String,
    pub target_link: String,
}

/// Read a vault and diff against the latest snapshot.
///
/// Fails fast if the vault was not produced by Hedgehog with snapshot
/// support — the caller surfaces this as "re-export to enable merge."
pub fn read_vault_diff(vault_path: &Path) -> Result<VaultDiff> {
    let (snapshot_path, snap) = snapshot::read_latest(vault_path)?
        .with_context(|| {
            format!(
                "{} has no Hedgehog snapshot; re-export to enable merge",
                vault_path.display()
            )
        })?;

    let baseline_version = parse_version_from_path(&snapshot_path)
        .context("snapshot filename did not match v{NNN}.json pattern")?;

    // The export's full raw response — gives us each driver's canonical
    // name so we can detect renames.
    let model_json_path = vault_path.join("Data").join("model.json");
    let model_bytes = std::fs::read(&model_json_path)
        .with_context(|| format!("reading {}", model_json_path.display()))?;
    let model: fiftyone_folds::ModelResponse =
        serde_json::from_slice(&model_bytes)
            .with_context(|| format!("parsing {}", model_json_path.display()))?;
    let model_drivers: BTreeMap<&str, &fiftyone_folds::Driver> =
        model.drivers.iter().map(|d| (d.code.as_str(), d)).collect();

    let drivers_dir = vault_path.join("Drivers");
    let mut driver_state_changes = Vec::new();
    let mut note_additions = Vec::new();
    let mut driver_metadata_changes = Vec::new();
    let mut edge_changes = Vec::new();

    if drivers_dir.is_dir() {
        for entry in std::fs::read_dir(&drivers_dir)
            .with_context(|| format!("listing {}", drivers_dir.display()))?
            .flatten()
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let body = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let fm = parse_frontmatter(&body);
            let Some(code) = fm.get("code").cloned() else {
                continue;
            };

            // Driver name as the user has it in the vault frontmatter
            // (may differ from model.json if they renamed it).
            let vault_name = fm.get("name").cloned().unwrap_or_default();

            // State change vs the snapshot.
            if let Some(vault_state) = fm.get("current_state").cloned() {
                let baseline_state = snap.driver_states.get(&code).cloned().unwrap_or_default();
                if !baseline_state.is_empty() && vault_state != baseline_state {
                    driver_state_changes.push(DriverStateChange {
                        code: code.clone(),
                        name: vault_name.clone(),
                        old_state: baseline_state,
                        new_state: vault_state,
                    });
                }
            }

            // Rename detection vs model.json.
            if let Some(model_driver) = model_drivers.get(code.as_str())
                && !vault_name.is_empty()
                && vault_name != model_driver.name
            {
                driver_metadata_changes.push(DriverMetadataChange {
                    code: code.clone(),
                    field: MetadataField::Name,
                    old: model_driver.name.clone(),
                    new: vault_name.clone(),
                });
            }

            // Note section.
            if let Some(notes_text) = extract_h2_section(&body, "Notes") {
                let trimmed = notes_text.trim();
                if !trimmed.is_empty() {
                    note_additions.push(NoteAddition {
                        driver_code: code.clone(),
                        driver_name: vault_name.clone(),
                        text: trimmed.to_owned(),
                    });

                    // Heuristic edges: any wiki-link inside `## Notes`
                    // that resembles a driver target (`Drivers/...`).
                    // The known model-derived sections never appear
                    // inside `## Notes`, so any `[[Drivers/...]]` here
                    // is something the user added on purpose.
                    for link in find_wiki_links(&notes_text) {
                        let bare = link.split('|').next().unwrap_or(&link).trim();
                        if bare.starts_with("Drivers/") {
                            edge_changes.push(EdgeChange {
                                in_driver_code: code.clone(),
                                target_link: bare.to_owned(),
                            });
                        }
                    }
                }
            }
        }
    }

    // Deterministic ordering — easier to test and the preview dialog
    // shows changes in a consistent order across runs.
    driver_state_changes.sort_by(|a, b| a.code.cmp(&b.code));
    note_additions.sort_by(|a, b| a.driver_code.cmp(&b.driver_code));
    driver_metadata_changes.sort_by(|a, b| a.code.cmp(&b.code));
    edge_changes.sort_by(|a, b| {
        a.in_driver_code
            .cmp(&b.in_driver_code)
            .then(a.target_link.cmp(&b.target_link))
    });

    Ok(VaultDiff {
        model_id: snap.model_id,
        vault_path: vault_path.to_owned(),
        baseline_version,
        driver_state_changes,
        note_additions,
        driver_metadata_changes,
        edge_changes,
    })
}

/// Convert a [`VaultDiff`] + the post-merge model into a [`history::MergeAudit`]
/// ready for [`crate::obsidian::re_export_with_merge`]. The diff's
/// `baseline_version + 1` becomes the merge's version number; outcome
/// shifts are computed against the *snapshot* probabilities (which the
/// diff carries indirectly via its baseline version).
pub fn build_audit_from(
    diff: &VaultDiff,
    applied_state_changes: &[DriverStateChange],
    post_model: &fiftyone_folds::ModelResponse,
) -> Result<history::MergeAudit> {
    let (_, snap) = snapshot::read_latest(&diff.vault_path)?
        .with_context(|| "snapshot vanished between diff read and audit build")?;

    let outcome_shifts = post_model
        .current
        .outcomes
        .iter()
        .filter_map(|o| {
            let before = snap.outcome_probabilities.get(&o.id).copied().unwrap_or(0.0);
            let after = o.probability.unwrap_or(0.0);
            if (before - after).abs() < f64::EPSILON {
                None
            } else {
                Some(history::OutcomeShiftAudit {
                    id: o.id,
                    label: o.label.clone(),
                    before_pct: before * 100.0,
                    after_pct: after * 100.0,
                })
            }
        })
        .collect();

    let driver_state_changes_audit = applied_state_changes
        .iter()
        .map(|c| history::DriverStateChangeAudit {
            code: c.code.clone(),
            name: c.name.clone(),
            before: c.old_state.clone(),
            after: c.new_state.clone(),
        })
        .collect();

    Ok(history::MergeAudit {
        version: diff.next_version(),
        timestamp: chrono::Utc::now(),
        model_id: post_model.model_id.clone(),
        driver_state_changes: driver_state_changes_audit,
        outcome_shifts,
        parked_note_additions: diff.note_additions.len(),
        parked_metadata_changes: diff.driver_metadata_changes.len(),
        parked_edge_changes: diff.edge_changes.len(),
    })
}

// ---------------------------------------------------------------------------
// Helpers — frontmatter + body parsing
// ---------------------------------------------------------------------------

fn parse_version_from_path(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_suffix(".json")?;
    let digits = stem.strip_prefix('v')?;
    digits.parse::<u32>().ok()
}

fn parse_frontmatter(body: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut lines = body.lines();
    if lines.next() != Some("---") {
        return out;
    }
    for line in lines {
        if line == "---" {
            break;
        }
        // Skip list-continuation lines (`  - foo`) — we only want the
        // top-level scalar entries.
        if line.starts_with(' ') || line.starts_with('-') {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let key = k.trim();
        let mut value = v.trim().to_owned();
        if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            value = value[1..value.len() - 1].to_owned();
        }
        if !key.is_empty() && !value.is_empty() {
            out.insert(key.to_owned(), value);
        }
    }
    out
}

/// Returns the body of an H2 section by heading text (without the `## `
/// prefix), up to the next H2 or end of file. Returns `None` if the
/// heading isn't present.
fn extract_h2_section(body: &str, heading: &str) -> Option<String> {
    let target = format!("## {heading}");
    let mut in_section = false;
    let mut out = String::new();
    for line in body.lines() {
        if !in_section {
            if line == target {
                in_section = true;
            }
            continue;
        }
        if line.starts_with("## ") {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    if in_section { Some(out) } else { None }
}

fn find_wiki_links(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'['
            && bytes[i + 1] == b'['
            && let Some(end) = s[i + 2..].find("]]")
        {
            let target = s[i + 2..i + 2 + end].to_owned();
            if seen.insert(target.clone()) {
                out.push(target);
            }
            i = i + 2 + end + 2;
            continue;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frontmatter_reads_quoted_and_unquoted() {
        let body = "---\ncode: UEBSP\nname: \"US Executive Branch\"\ncurrent_state: Hawkish\n---\nbody\n";
        let fm = parse_frontmatter(body);
        assert_eq!(fm.get("code").map(String::as_str), Some("UEBSP"));
        assert_eq!(
            fm.get("name").map(String::as_str),
            Some("US Executive Branch")
        );
        assert_eq!(
            fm.get("current_state").map(String::as_str),
            Some("Hawkish")
        );
    }

    #[test]
    fn parse_frontmatter_skips_list_entries() {
        let body =
            "---\ncode: X\npossible_states:\n  - Hawkish\n  - Dovish\ntier: root\n---\n";
        let fm = parse_frontmatter(body);
        assert_eq!(fm.get("code").map(String::as_str), Some("X"));
        assert_eq!(fm.get("tier").map(String::as_str), Some("root"));
        assert!(!fm.contains_key("possible_states"));
    }

    #[test]
    fn extract_h2_section_returns_inner_body_only() {
        let body = "intro\n\n## Notes\nfirst\nsecond\n\n## Local Causal Map\nfoo\n";
        let got = extract_h2_section(body, "Notes").expect("found");
        assert!(got.contains("first"));
        assert!(got.contains("second"));
        assert!(!got.contains("Local Causal Map"));
    }

    #[test]
    fn extract_h2_section_returns_none_when_missing() {
        let body = "## Other\nfoo\n";
        assert!(extract_h2_section(body, "Notes").is_none());
    }

    #[test]
    fn find_wiki_links_deduplicates() {
        let s = "see [[Drivers/X — Foo]] and [[Drivers/X — Foo]] again";
        let links = find_wiki_links(s);
        assert_eq!(links, vec!["Drivers/X — Foo"]);
    }

    #[test]
    fn find_wiki_links_handles_aliases() {
        let s = "see [[Drivers/X — Foo|the X driver]]";
        let links = find_wiki_links(s);
        assert_eq!(links, vec!["Drivers/X — Foo|the X driver"]);
    }

    #[test]
    fn parse_version_extracts_v002() {
        assert_eq!(
            parse_version_from_path(Path::new("/some/where/v002.json")),
            Some(2)
        );
    }
}
