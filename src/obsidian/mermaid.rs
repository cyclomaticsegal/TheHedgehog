//! Mermaid snippet helpers. We don't need a full Mermaid library — every
//! diagram we emit is one of three shapes (pie chart, local 2-hop DAG,
//! outcome funnel), so a couple of focused fns are cheaper than a
//! general builder.

/// Pie chart sized to the model's outcomes. Probabilities are rendered as
/// percentages to one decimal place — matches the Hedgehog UI.
pub(crate) fn outcomes_pie(outcomes: &[fiftyone_folds::Outcome]) -> String {
    let mut s = String::from("```mermaid\npie showData title Outcome probabilities\n");
    for o in outcomes {
        let pct = o.probability.unwrap_or(0.0) * 100.0;
        // Mermaid pie labels must be quoted and free of newlines.
        let label = o.label.replace('"', "'").replace('\n', " ");
        let label = label.chars().take(60).collect::<String>();
        s.push_str(&format!("    \"{label}\" : {pct:.1}\n"));
    }
    s.push_str("```\n");
    s
}

/// Local 2-hop neighbourhood for a driver: direct parents → driver → direct
/// children, plus an edge to `DV` if present. Used on each driver page so
/// the reader can orient without leaving the note.
///
/// Names are looked up via `driver_name`; the special code "DV" renders as
/// "Outcome (DV)".
pub(crate) fn local_dag(
    code: &str,
    parents: &[String],
    children: &[String],
    driver_name: impl Fn(&str) -> Option<String>,
) -> String {
    // Peripheral nodes show just the code — the body of the note has
    // the full names as wikilinks, and short labels keep the diagram
    // narrow enough to fit inside Obsidian's editor pane without
    // clipping when the user has a high fan-out driver like DV's parents.
    let mut s = String::from("```mermaid\nflowchart LR\n");
    s.push_str(&format!(
        "    {0}([\"{1}\"]):::self\n",
        node_id(code),
        full_label(code, &driver_name)
    ));
    for p in parents {
        s.push_str(&format!(
            "    {0}[\"{1}\"]\n",
            node_id(p),
            short_label(p)
        ));
        s.push_str(&format!("    {0} --> {1}\n", node_id(p), node_id(code)));
    }
    for c in children {
        s.push_str(&format!(
            "    {0}[\"{1}\"]\n",
            node_id(c),
            short_label(c)
        ));
        s.push_str(&format!("    {0} --> {1}\n", node_id(code), node_id(c)));
    }
    s.push_str("    classDef self fill:#1f3b5b,stroke:#7fb2ff,color:#fff;\n");
    s.push_str("```\n");
    s
}

/// Outcome-side mini DAG: direct DV-parents → DV → this outcome.
pub(crate) fn outcome_funnel(
    outcome_label: &str,
    dv_parents: &[String],
    _driver_name: impl Fn(&str) -> Option<String>,
) -> String {
    let mut s = String::from("```mermaid\nflowchart LR\n");
    let label = outcome_label
        .chars()
        .take(60)
        .collect::<String>()
        .replace('"', "'");
    s.push_str(&format!("    OUT([\"{label}\"]):::out\n"));
    s.push_str("    DV{{DV}}\n");
    s.push_str("    DV --> OUT\n");
    for p in dv_parents {
        s.push_str(&format!("    {0}[\"{1}\"]\n", node_id(p), short_label(p)));
        s.push_str(&format!("    {0} --> DV\n", node_id(p)));
    }
    s.push_str("    classDef out fill:#3a1f5b,stroke:#c89bff,color:#fff;\n");
    s.push_str("```\n");
    s
}

/// Mermaid node IDs must match `[A-Za-z][A-Za-z0-9_]*`. Driver codes
/// already do, but `DV` and synthetic IDs need a guard.
fn node_id(code: &str) -> String {
    if code == "DV" {
        return "DV".to_owned();
    }
    let mut out = String::with_capacity(code.len());
    for (i, ch) in code.chars().enumerate() {
        if i == 0 && !ch.is_ascii_alphabetic() {
            out.push('N');
        }
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() { "N".to_owned() } else { out }
}

/// Compact label for peripheral nodes — just the driver code, or
/// "DV" for the dependent variable. Keeps local maps narrow.
fn short_label(code: &str) -> String {
    if code == "DV" {
        return "DV".to_owned();
    }
    code.replace('"', "'")
}

/// Verbose label for the *centre* node only — code + name so the
/// reader can identify the focal driver without context.
fn full_label(code: &str, driver_name: &impl Fn(&str) -> Option<String>) -> String {
    if code == "DV" {
        return "Outcome (DV)".to_owned();
    }
    let name = driver_name(code).unwrap_or_default();
    let label = if name.is_empty() {
        code.to_owned()
    } else {
        format!("{code}: {name}")
    };
    let trimmed = label.chars().take(40).collect::<String>();
    trimmed.replace('"', "'")
}
