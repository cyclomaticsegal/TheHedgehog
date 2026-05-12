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
pub(crate) mod mermaid;
pub(crate) mod names;
pub(crate) mod outcome;
pub(crate) mod overview;
pub(crate) mod source;
pub(crate) mod vault;

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

    let sources = source::write_all(model, names, root)?;
    driver::write_all(model, names, &sources, root)?;
    outcome::write_all(model, names, root)?;
    overview::write(model, names, root)?;
    canvas::write(model, names, root)?;
    base::write_all(root)?;

    // Provenance: ship the raw response so the vault is self-describing
    // and a future tool could re-derive everything. The Provenance.md
    // sibling exists so the `Data/` folder doesn't look empty in
    // Obsidian's explorer (which only previews supported file types).
    let data_dir = root.join("Data");
    std::fs::create_dir_all(&data_dir)?;
    let pretty = serde_json::to_string_pretty(model)?;
    std::fs::write(data_dir.join("model.json"), pretty)?;
    std::fs::write(data_dir.join("Provenance.md"), provenance_readme(model))?;

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
