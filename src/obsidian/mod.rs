//! Export a 51Folds `ModelResponse` as a self-contained Obsidian vault.
//!
//! The vault is opinionated: every model produces the same directory
//! layout (Overview / Drivers / Outcomes / Sources / Model.canvas /
//! Drivers.base) plus a seeded `.obsidian/` config so the folder opens
//! correctly with zero manual setup. See `/Users/simonsegal/.claude/plans/`
//! for the design write-up.
//!
//! Entry point: [`export_model`]. Sub-modules are kept `pub(crate)` so they
//! can be unit-tested individually against the SDK's fixture JSON.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub(crate) mod base;
pub(crate) mod canvas;
pub(crate) mod driver;
pub(crate) mod history;
pub mod merge;
pub(crate) mod mermaid;
pub(crate) mod names;
pub(crate) mod outcome;
pub(crate) mod overview;
pub(crate) mod snapshot;
pub(crate) mod source;
pub(crate) mod user_guide;
pub(crate) mod vault;

/// How the per-doc emitters should write their output.
///
/// `Fresh` overwrites the file unconditionally — used for the very first
/// export, where there are no user notes to preserve.
///
/// `Selective` reads any existing file first and preserves H2 sections
/// whose headings aren't in the writer's "model-derived" set (e.g.
/// `## Notes`, custom user sections). Used for the post-merge re-export
/// so the user's reasoning accumulates across merges instead of being
/// clobbered every time the server returns fresh probabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteMode {
    Fresh,
    Selective,
}

/// Export `model` as a new vault inside `parent_dir`. Returns the absolute
/// path to the created vault root (a fresh subdirectory of `parent_dir`).
///
/// The export is atomic-ish: we write everything into a temp sibling
/// directory and `rename` into place on success, so a mid-export failure
/// won't leave a half-written vault visible to the user.
pub fn export_model(model: &fiftyone_folds::ModelResponse, parent_dir: &Path) -> Result<PathBuf> {
    let names = names::Names::compute(model);
    let vault_path = parent_dir.join(&names.vault_dir);

    // Write into a sibling temp dir, then rename — keeps the final
    // path absent until the whole vault is on disk.
    let tmp_path = parent_dir.join(format!(".{}.tmp", names.vault_dir));
    if tmp_path.exists() {
        std::fs::remove_dir_all(&tmp_path).ok();
    }

    write_vault(model, &names, &tmp_path)
        .with_context(|| format!("writing vault into {}", tmp_path.display()))?;

    if vault_path.exists() {
        // The user picked a parent that already contains an export of
        // this model. Refuse rather than silently overwriting their notes.
        std::fs::remove_dir_all(&tmp_path).ok();
        anyhow::bail!(
            "{} already exists — move or rename it before re-exporting",
            vault_path.display()
        );
    }

    std::fs::rename(&tmp_path, &vault_path)
        .with_context(|| format!("finalising vault at {}", vault_path.display()))?;
    Ok(vault_path)
}

fn write_vault(
    model: &fiftyone_folds::ModelResponse,
    names: &names::Names,
    root: &Path,
) -> Result<()> {
    vault::scaffold(root)?;

    let history = history::History::empty();
    let sources = source::write_all(model, names, root)?;
    driver::write_all(model, names, &sources, &history, root, WriteMode::Fresh)?;
    outcome::write_all(model, names, &history, root, WriteMode::Fresh)?;
    overview::write(model, names, &history, root, WriteMode::Fresh)?;
    canvas::write(model, names, root)?;
    base::write_all(root)?;
    user_guide::write(root)?;

    // Provenance: ship the raw response so the vault is self-describing
    // and a future tool could re-derive everything. The Provenance.md
    // sibling exists so the `Data/` folder doesn't look empty in
    // Obsidian's explorer (which only previews supported file types).
    let data_dir = root.join("Data");
    std::fs::create_dir_all(&data_dir)?;
    let pretty = serde_json::to_string_pretty(model)?;
    std::fs::write(data_dir.join("model.json"), pretty)?;
    std::fs::write(data_dir.join("Provenance.md"), provenance_readme(model))?;

    // The first numbered snapshot anchors all future merge diffs. This
    // is a fresh vault so `write_next` lands on `v001.json`.
    let snap = snapshot::Snapshot::from_model(model);
    snapshot::write_next(root, &snap)?;
    // Empty history is still persisted so the file shape is stable from
    // export #1 onward — the merge orchestration always reads-then-writes.
    history::write(root, &history)?;

    Ok(())
}

/// Post-merge selective re-export. Same vault path, fresh model from
/// the server, plus a pre-computed audit describing what changed and
/// what was parked. Preserves any user-authored sections (`## Notes`,
/// custom H2 blocks, etc.) while refreshing every model-derived
/// section and appending new history rows.
///
/// Sequence:
/// 1. Read existing `Data/history.json` (or initialise empty).
/// 2. Append the audit's state changes / outcome shifts / merge summary.
/// 3. Write `Merges/<timestamp>.md`.
/// 4. Selective-rewrite every driver, outcome, and `Overview.md`.
/// 5. Refresh `Data/model.json` with the new raw response.
/// 6. Append the next numbered snapshot under `Data/snapshots/`.
/// 7. Persist the updated history JSON.
pub fn re_export_with_merge(
    model: &fiftyone_folds::ModelResponse,
    vault_path: &Path,
    audit: history::MergeAudit,
) -> Result<()> {
    let names = names::Names::compute(model);

    let mut hist = history::read_or_init(vault_path)?;
    let merge_note = history::write_merge_note(vault_path, &audit)?;

    let ts = audit.timestamp;
    let version = audit.version;
    for change in &audit.driver_state_changes {
        hist.drivers
            .entry(change.code.clone())
            .or_default()
            .push(history::DriverHistoryRow {
                version,
                timestamp: ts,
                before: change.before.clone(),
                after: change.after.clone(),
                merge_note: merge_note.clone(),
            });
    }
    for shift in &audit.outcome_shifts {
        hist.outcomes
            .entry(shift.id)
            .or_default()
            .push(history::OutcomeHistoryRow {
                version,
                timestamp: ts,
                before_pct: shift.before_pct,
                after_pct: shift.after_pct,
                merge_note: merge_note.clone(),
            });
    }
    hist.merges.push(history::MergeSummary {
        version,
        timestamp: ts,
        model_id: audit.model_id.clone(),
        applied_state_changes: audit.driver_state_changes.len(),
        parked_note_additions: audit.parked_note_additions,
        parked_metadata_changes: audit.parked_metadata_changes,
        parked_edge_changes: audit.parked_edge_changes,
        merge_note,
    });

    let sources = source::write_all(model, &names, vault_path)?;
    driver::write_all(model, &names, &sources, &hist, vault_path, WriteMode::Selective)?;
    outcome::write_all(model, &names, &hist, vault_path, WriteMode::Selective)?;
    overview::write(model, &names, &hist, vault_path, WriteMode::Selective)?;
    canvas::write(model, &names, vault_path)?;
    base::write_all(vault_path)?;
    user_guide::write(vault_path)?;

    let data_dir = vault_path.join("Data");
    std::fs::create_dir_all(&data_dir)?;
    let pretty = serde_json::to_string_pretty(model)?;
    std::fs::write(data_dir.join("model.json"), pretty)?;
    std::fs::write(data_dir.join("Provenance.md"), provenance_readme(model))?;

    let snap = snapshot::Snapshot::from_model(model);
    snapshot::write_next(vault_path, &snap)?;
    history::write(vault_path, &hist)?;

    Ok(())
}

fn provenance_readme(model: &fiftyone_folds::ModelResponse) -> String {
    format!(
        "---\nmodel_id: {}\nstatus: {}\nexported_at: {}\ntags: [provenance]\n---\n\n\
         # Provenance\n\n\
         > [!info] Raw response\n\
         > The full 51Folds API response for this model is stored beside this file as `model.json`.\n\
         > Open it from your OS file manager for the unmodified payload — Obsidian doesn't render JSON inline.\n\n\
         **Model ID:** `{}`\n\n\
         **Drivers:** {}  ·  **Edges:** {}  ·  **Outcomes:** {}\n",
        model.model_id,
        model.status,
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
        model.model_id,
        model.drivers.len(),
        model.edges.len(),
        model.current.outcomes.len(),
    )
}

#[cfg(test)]
mod e2e_tests {
    use super::*;

    fn load_rich_fixture() -> fiftyone_folds::ModelResponse {
        let bytes = std::fs::read("vendor/fiftyone-folds/tests/fixtures/model-rich.response.json")
            .expect("fixture present");
        let env: fiftyone_folds::ApiEnvelope<fiftyone_folds::ModelResponse> =
            serde_json::from_slice(&bytes).expect("fixture deserialises");
        env.data
    }

    fn tempdir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "hedgehog-obsidian-e2e-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn rich_fixture_writes_complete_vault() {
        let model = load_rich_fixture();
        let parent = tempdir();
        let vault = export_model(&model, &parent).expect("export succeeds");

        // Top-level structure exists.
        for required in [
            ".obsidian/app.json",
            ".obsidian/core-plugins.json",
            ".obsidian/graph.json",
            "Overview.md",
            "Model.canvas",
            "Drivers.base",
            "Sources Index.base",
            "Data/model.json",
        ] {
            assert!(vault.join(required).exists(), "missing {required}");
        }

        // Every driver got a file with the rich sections.
        for d in &model.drivers {
            let path = vault.join(format!("Drivers/{} — {}.md", d.code, d.name));
            let body = std::fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("driver file missing for {}", d.code));
            assert!(body.contains(&format!("# {} ({})", d.name, d.code)));
            assert!(body.contains("## Possible States"));
            if d.context.is_some() {
                assert!(body.contains("## Why This Matters"), "ctx for {}", d.code);
            }
        }

        // Every outcome got a file with a probability.
        for o in &model.current.outcomes {
            let body = std::fs::read_to_string(vault.join(format!(
                "Outcomes/{} — {}.md",
                o.id,
                truncated(&o.label)
            )))
            .expect("outcome file");
            assert!(body.contains(&format!("# Outcome {}", o.id)));
        }

        // Sources de-duplicated to unique URLs.
        let unique_urls: std::collections::BTreeSet<&str> = model
            .current
            .drivers
            .iter()
            .flat_map(|ds| {
                ds.justification
                    .iter()
                    .flat_map(|j| j.citations.iter().map(|c| c.source.as_str()))
            })
            .collect();
        let sources_dir = vault.join("Sources");
        let actual = std::fs::read_dir(&sources_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .count();
        assert_eq!(actual, unique_urls.len(), "source file count = unique URLs");

        // Canvas parses and covers every model edge.
        let canvas: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(vault.join("Model.canvas")).unwrap())
                .unwrap();
        let edge_count = canvas["edges"].as_array().unwrap().len();
        // Model edges + synthetic DV → outcome edges.
        assert_eq!(edge_count, model.edges.len() + model.current.outcomes.len());

        // Cleanup. The export rename target is the only thing left in `parent`.
        std::fs::remove_dir_all(parent).ok();
    }

    #[test]
    fn selective_re_export_preserves_user_notes_and_records_history() {
        let model = load_rich_fixture();
        let parent = tempdir();
        let vault = export_model(&model, &parent).expect("first export");

        // Pick the first driver and append a `## Notes` section as if the
        // user wrote some reasoning while reading the export.
        let d = &model.drivers[0];
        let driver_path = vault.join(format!("Drivers/{} — {}.md", d.code, d.name));
        let original = std::fs::read_to_string(&driver_path).unwrap();
        let user_note = "## Notes\n\nMy reasoning here. [[Some Wiki Link]]\nLine two of my note.\n";
        let edited = format!("{original}\n{user_note}");
        std::fs::write(&driver_path, &edited).unwrap();

        // Build a fake merge audit. The state-change "before" matches what
        // the export wrote; the "after" pretends the server moved this
        // driver to a different state.
        let snap_before = snapshot::Snapshot::from_model(&model);
        let before_state = snap_before
            .driver_states
            .get(&d.code)
            .cloned()
            .unwrap_or_default();
        let after_state = if before_state == "Bullish" {
            "Bearish".to_owned()
        } else {
            "Bullish".to_owned()
        };
        let audit = history::MergeAudit {
            version: 2,
            timestamp: chrono::Utc::now(),
            model_id: model.model_id.clone(),
            driver_state_changes: vec![history::DriverStateChangeAudit {
                code: d.code.clone(),
                name: d.name.clone(),
                before: before_state,
                after: after_state,
            }],
            outcome_shifts: vec![],
            parked_note_additions: 1,
            parked_metadata_changes: 0,
            parked_edge_changes: 0,
        };

        re_export_with_merge(&model, &vault, audit).expect("re-export");

        let after_body = std::fs::read_to_string(&driver_path).unwrap();
        assert!(
            after_body.contains("## Notes"),
            "user `## Notes` section must survive selective re-export"
        );
        assert!(
            after_body.contains("My reasoning here."),
            "user note body preserved"
        );
        assert!(
            after_body.contains("[[Some Wiki Link]]"),
            "user wiki-link preserved inside `## Notes`"
        );
        assert!(
            after_body.contains("## History"),
            "history section emitted"
        );
        assert!(
            after_body.contains("v002"),
            "new history row references v002"
        );
        assert!(
            after_body.contains("## Possible States"),
            "model-derived sections still present"
        );

        // Versioned snapshots accumulate: v001 and v002 both present.
        assert!(vault.join("Data/snapshots/v001.json").exists());
        assert!(vault.join("Data/snapshots/v002.json").exists());

        // Per-merge audit note exists and has the applied table.
        let merges_dir = vault.join("Merges");
        let merge_files: Vec<_> = std::fs::read_dir(&merges_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .map(|n| n.ends_with(".md"))
                    .unwrap_or(false)
            })
            .collect();
        assert_eq!(merge_files.len(), 1, "exactly one merge audit note");
        let merge_body = std::fs::read_to_string(merge_files[0].path()).unwrap();
        assert!(
            merge_body.contains("Applied — driver state changes"),
            "{merge_body}"
        );

        // Overview gained a history row.
        let overview = std::fs::read_to_string(vault.join("Overview.md")).unwrap();
        assert!(overview.contains("## History"));
        assert!(overview.contains("v002"));

        // history.json accumulated the driver row.
        let hist = history::read_or_init(&vault).expect("history reads back");
        assert_eq!(hist.merges.len(), 1);
        assert_eq!(hist.drivers.get(&d.code).map(Vec::len), Some(1));

        std::fs::remove_dir_all(parent).ok();
    }

    #[test]
    fn read_vault_diff_picks_up_state_change_note_and_rename() {
        let model = load_rich_fixture();
        let parent = tempdir();
        let vault = export_model(&model, &parent).expect("export");

        let d = &model.drivers[0];
        let driver_path = vault.join(format!("Drivers/{} — {}.md", d.code, d.name));
        let body = std::fs::read_to_string(&driver_path).unwrap();

        // Flip the state to anything different from the snapshot baseline.
        let baseline_state = {
            let (_, s) = snapshot::read_latest(&vault).unwrap().unwrap();
            s.driver_states.get(&d.code).cloned().unwrap_or_default()
        };
        let new_state = if baseline_state == "Bullish" {
            "Bearish"
        } else {
            "Bullish"
        };
        let renamed = body
            .replace(
                &format!("current_state: {baseline_state}"),
                &format!("current_state: {new_state}"),
            );
        // The `name:` frontmatter line may or may not be quoted depending on
        // characters in the name — rewrite it positionally.
        let renamed: String = renamed
            .lines()
            .map(|line| {
                if line.starts_with("name: ") {
                    format!("name: \"{} — Renamed\"", d.name)
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        // Preserve trailing newline if the original had one.
        let renamed = if body.ends_with('\n') {
            format!("{renamed}\n")
        } else {
            renamed
        };
        // Append a user `## Notes` section with a wiki-link to another driver.
        let other = &model.drivers[1];
        let with_notes = format!(
            "{renamed}\n## Notes\n\nMy reasoning — see [[Drivers/{} — {}]] as a parent.\n",
            other.code, other.name
        );
        std::fs::write(&driver_path, with_notes).unwrap();

        let diff = merge::read_vault_diff(&vault).expect("diff");

        let codes: Vec<&str> = diff
            .driver_state_changes
            .iter()
            .map(|c| c.code.as_str())
            .collect();
        assert!(codes.contains(&d.code.as_str()), "state change for {}", d.code);

        let note_codes: Vec<&str> = diff
            .note_additions
            .iter()
            .map(|n| n.driver_code.as_str())
            .collect();
        assert!(
            note_codes.contains(&d.code.as_str()),
            "note addition for {}",
            d.code
        );

        let renamed_codes: Vec<&str> = diff
            .driver_metadata_changes
            .iter()
            .map(|m| m.code.as_str())
            .collect();
        assert!(
            renamed_codes.contains(&d.code.as_str()),
            "rename detected for {}",
            d.code
        );

        assert!(
            !diff.edge_changes.is_empty(),
            "wiki-link in `## Notes` surfaces as a parked edge change"
        );

        std::fs::remove_dir_all(parent).ok();
    }

    #[test]
    fn read_vault_diff_errors_when_snapshot_missing() {
        let model = load_rich_fixture();
        let parent = tempdir();
        let vault = export_model(&model, &parent).expect("export");

        std::fs::remove_dir_all(vault.join("Data/snapshots")).unwrap();
        let err = merge::read_vault_diff(&vault).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("re-export to enable merge"),
            "{msg}"
        );

        std::fs::remove_dir_all(parent).ok();
    }

    fn truncated(s: &str) -> String {
        let mut out = String::new();
        for c in s.chars().take(60) {
            // Same `sanitise` logic that names.rs applies.
            if matches!(
                c,
                '/' | '\\'
                    | ':'
                    | '*'
                    | '?'
                    | '"'
                    | '<'
                    | '>'
                    | '|'
                    | '['
                    | ']'
                    | '#'
                    | '^'
                    | '\n'
                    | '\r'
                    | '\t'
            ) {
                out.push(' ');
            } else {
                out.push(c);
            }
        }
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}
