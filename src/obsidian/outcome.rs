//! Per-outcome pages. Each `current.outcomes[]` entry becomes one note
//! showing the full label, its probability, the drivers that connect
//! directly to `DV` (and thus most directly shape this outcome), and a
//! Mermaid funnel back to those drivers.

use crate::obsidian::{WriteMode, history, mermaid, names::Names};
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::Path;

/// H2 headings emitted by this writer. Anything *not* in this set is
/// treated as user-authored and preserved across selective re-exports.
const KNOWN_MODEL_HEADINGS: &[&str] = &[
    "Direct Drivers",
    "Causal Path",
    "History",
    "In The Model",
];

pub(crate) fn write_all(
    model: &fiftyone_folds::ModelResponse,
    names: &Names,
    hist: &history::History,
    root: &Path,
    mode: WriteMode,
) -> Result<()> {
    std::fs::create_dir_all(root.join("Outcomes"))?;

    let dv_parents = dv_parents(model);
    let by_code: BTreeMap<&str, &fiftyone_folds::Driver> =
        model.drivers.iter().map(|d| (d.code.as_str(), d)).collect();

    let top_id = top_outcome_id(model);

    for o in &model.current.outcomes {
        let path = root.join(names.outcomes.get(&o.id).expect("outcome name precomputed"));
        let history_rows = hist.outcomes.get(&o.id).map(Vec::as_slice).unwrap_or(&[]);
        let body = render_outcome(
            o,
            top_id == Some(o.id),
            &dv_parents,
            &by_code,
            names,
            history_rows,
        );
        let final_body = match mode {
            WriteMode::Fresh => body,
            WriteMode::Selective => merge_user_zones(&path, body)?,
        };
        std::fs::write(&path, final_body)?;
    }
    Ok(())
}

fn merge_user_zones(path: &Path, fresh_model_body: String) -> Result<String> {
    let Ok(existing) = std::fs::read_to_string(path) else {
        return Ok(fresh_model_body);
    };
    let preserved = history::extract_user_zones(&existing, KNOWN_MODEL_HEADINGS);
    if preserved.trim().is_empty() {
        return Ok(fresh_model_body);
    }
    let mut out = fresh_model_body;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(&preserved);
    Ok(out)
}

fn dv_parents(model: &fiftyone_folds::ModelResponse) -> Vec<String> {
    let mut v: Vec<String> = model
        .edges
        .iter()
        .filter(|e| e.child == "DV")
        .map(|e| e.parent.clone())
        .collect();
    v.sort();
    v.dedup();
    v
}

fn top_outcome_id(model: &fiftyone_folds::ModelResponse) -> Option<i64> {
    model
        .current
        .outcomes
        .iter()
        .max_by(|a, b| {
            a.probability
                .unwrap_or(0.0)
                .partial_cmp(&b.probability.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|o| o.id)
}

fn render_outcome(
    o: &fiftyone_folds::Outcome,
    is_top: bool,
    dv_parents: &[String],
    by_code: &BTreeMap<&str, &fiftyone_folds::Driver>,
    names: &Names,
    history_rows: &[history::OutcomeHistoryRow],
) -> String {
    let pct = o.probability.unwrap_or(0.0) * 100.0;
    let mut s = String::new();
    s.push_str("---\n");
    s.push_str(&format!("id: {}\n", o.id));
    s.push_str(&format!(
        "probability: {:.4}\n",
        o.probability.unwrap_or(0.0)
    ));
    s.push_str(&format!("probability_pct: {pct:.2}\n"));
    s.push_str(&format!("is_top: {is_top}\n"));
    s.push_str("tags: [outcome]\n");
    s.push_str("---\n\n");

    s.push_str(&format!("# Outcome {}\n\n", o.id));
    s.push_str(&format!(
        "> [!quote]\n> {}\n\n",
        escape_callout_lines(&o.label)
    ));
    let callout_kind = if is_top { "success" } else { "info" };
    s.push_str(&format!(
        "> [!{callout_kind}] Probability: **{pct:.2}%**{}\n\n",
        if is_top { " *(top outcome)*" } else { "" }
    ));

    if !dv_parents.is_empty() {
        s.push_str("## Direct Drivers\n\n");
        s.push_str(
            "_These drivers connect directly to the dependent variable in the causal map._\n\n",
        );
        for code in dv_parents {
            if let Some(l) = names.driver_link(code) {
                s.push_str(&format!("- {l}\n"));
            }
        }
        s.push('\n');

        s.push_str("## Causal Path\n\n");
        let lookup = |c: &str| by_code.get(c).map(|d| d.name.clone());
        s.push_str(&mermaid::outcome_funnel(&o.label, dv_parents, lookup));
        s.push('\n');
    }

    s.push_str(&history::render_outcome_section(history_rows));
    s.push_str(
        "## In The Model\n\nReturn to the model [[Overview]] or the [[Model.canvas|causal map]].\n",
    );
    s
}

fn escape_callout_lines(text: &str) -> String {
    text.replace('\n', "\n> ")
}
