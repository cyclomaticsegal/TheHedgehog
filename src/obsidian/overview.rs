//! `Overview.md` — the vault's landing page. Mirrors what the Hedgehog
//! Outcome view shows: model question, background, short summary,
//! outcome probabilities, and entry points to the drivers / canvas /
//! sources.

use crate::obsidian::{mermaid, names::Names};
use anyhow::Result;
use std::path::Path;

pub(crate) fn write(
    model: &fiftyone_folds::ModelResponse,
    names: &Names,
    root: &Path,
) -> Result<()> {
    let body = render(model, names);
    std::fs::write(root.join("Overview.md"), body)?;
    Ok(())
}

fn render(model: &fiftyone_folds::ModelResponse, names: &Names) -> String {
    let mut s = String::new();

    s.push_str("---\n");
    s.push_str(&format!("model_id: {}\n", yaml_str(&model.model_id)));
    s.push_str(&format!("status: {}\n", yaml_str(&model.status)));
    s.push_str(&format!("question: {}\n", yaml_str(&model.question)));
    if !model.created_at.is_empty() {
        s.push_str(&format!("created_at: {}\n", yaml_str(&model.created_at)));
    }
    if !model.updated_at.is_empty() {
        s.push_str(&format!("updated_at: {}\n", yaml_str(&model.updated_at)));
    }
    s.push_str(&format!("num_drivers: {}\n", model.drivers.len()));
    s.push_str(&format!("num_edges: {}\n", model.edges.len()));
    s.push_str(&format!("num_outcomes: {}\n", model.current.outcomes.len()));
    s.push_str("tags: [overview]\n");
    s.push_str("---\n\n");

    s.push_str("# Overview\n\n");
    s.push_str(&format!(
        "> [!quote] Model Question\n> {}\n\n",
        model.question.replace('\n', "\n> ")
    ));

    if !model.short_summary.trim().is_empty() {
        s.push_str("> [!summary] Short Summary\n");
        for line in model.short_summary.lines() {
            s.push_str(&format!("> {line}\n"));
        }
        s.push('\n');
    }

    if !model.context.trim().is_empty() {
        s.push_str("## Background\n\n");
        s.push_str(model.context.trim_end());
        s.push_str("\n\n");
    }

    s.push_str("## Outcomes\n\n");
    if !model.current.outcomes.is_empty() {
        s.push_str(&mermaid::outcomes_pie(&model.current.outcomes));
        s.push('\n');
        s.push_str("| Outcome | Probability |\n|---|---:|\n");
        let mut sorted: Vec<&fiftyone_folds::Outcome> = model.current.outcomes.iter().collect();
        sorted.sort_by(|a, b| {
            b.probability
                .unwrap_or(0.0)
                .partial_cmp(&a.probability.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for o in sorted {
            let pct = o.probability.unwrap_or(0.0) * 100.0;
            let link = names
                .outcome_link(o.id)
                .unwrap_or_else(|| format!("Outcome {}", o.id));
            s.push_str(&format!("| {link} | {pct:.2}% |\n"));
        }
        s.push('\n');
    }

    s.push_str("## Drivers\n\n");
    s.push_str("![[Drivers]]\n\n");
    s.push_str("_Open `Drivers.base` for the sortable / filterable table view._\n\n");

    s.push_str("## Causal Map\n\n");
    s.push_str("![[Model.canvas]]\n\n");
    s.push_str("_Open `Model.canvas` for the laid-out DAG. Driver → DV edges are coloured green; driver → driver edges cyan._\n\n");

    s.push_str("## Sources\n\n");
    if names.sources.is_empty() {
        s.push_str("_No citations recorded for this model._\n");
    } else {
        s.push_str(&format!(
            "{} unique source{} cited across drivers. See [[Sources Index]] for the catalogue, or open any [[Sources/]] note and use the Backlinks pane to see citing drivers.\n",
            names.sources.len(),
            if names.sources.len() == 1 { "" } else { "s" }
        ));
    }

    s
}

fn yaml_str(value: &str) -> String {
    let needs_quote = value.is_empty()
        || value
            .chars()
            .any(|c| matches!(c, ':' | '#' | '"' | '\'' | '\n'))
        || value.starts_with(['-', '[', '{', '"', '\'']);
    if needs_quote {
        let escaped = value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', " ");
        format!("\"{escaped}\"")
    } else {
        value.to_owned()
    }
}
