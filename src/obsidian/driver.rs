//! Per-driver pages — the rich-narrative core of the export.
//!
//! Each driver gets typed frontmatter so it's queryable in Bases, plus a
//! body that mirrors how Hedgehog presents a driver in the UI (current
//! state callout, the three context sections, justification rationale)
//! with the inline `[N]` citation markers rewritten to `[^N]` footnotes
//! that link to the de-duplicated `Sources/` notes.

use crate::obsidian::{
    mermaid,
    names::Names,
    source::{SourceIndex, citation_key},
};
use anyhow::Result;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

pub(crate) fn write_all(
    model: &fiftyone_folds::ModelResponse,
    names: &Names,
    sources: &SourceIndex,
    root: &Path,
) -> Result<()> {
    std::fs::create_dir_all(root.join("Drivers"))?;

    let by_code = drivers_by_code(model);
    let state_by_code = state_by_code(model);
    let justification_by_code = justification_by_code(model);
    let (parents_of, children_of) = adjacency(model);

    for d in &model.drivers {
        let body = render_driver(
            d,
            state_by_code.get(d.code.as_str()).copied(),
            justification_by_code.get(d.code.as_str()).copied(),
            parents_of
                .get(d.code.as_str())
                .map(|v| v.as_slice())
                .unwrap_or(&[]),
            children_of
                .get(d.code.as_str())
                .map(|v| v.as_slice())
                .unwrap_or(&[]),
            &by_code,
            names,
            sources,
        );
        let path = root.join(names.drivers.get(&d.code).expect("driver name precomputed"));
        std::fs::write(&path, body)?;
    }
    Ok(())
}

fn drivers_by_code(
    model: &fiftyone_folds::ModelResponse,
) -> BTreeMap<&str, &fiftyone_folds::Driver> {
    model.drivers.iter().map(|d| (d.code.as_str(), d)).collect()
}

fn state_by_code(model: &fiftyone_folds::ModelResponse) -> BTreeMap<&str, &str> {
    model
        .current
        .drivers
        .iter()
        .map(|ds| (ds.code.as_str(), ds.state.as_str()))
        .collect()
}

fn justification_by_code(
    model: &fiftyone_folds::ModelResponse,
) -> BTreeMap<&str, &fiftyone_folds::DriverJustification> {
    model
        .current
        .drivers
        .iter()
        .filter_map(|ds| ds.justification.as_ref().map(|j| (ds.code.as_str(), j)))
        .collect()
}

/// Returns (parents_of[code], children_of[code]). The synthetic `DV`
/// node only ever appears as a child; we keep it in `children_of`
/// entries so each driver page can report "Reaches DV".
fn adjacency(
    model: &fiftyone_folds::ModelResponse,
) -> (BTreeMap<String, Vec<String>>, BTreeMap<String, Vec<String>>) {
    let mut parents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in &model.edges {
        children
            .entry(e.parent.clone())
            .or_default()
            .push(e.child.clone());
        parents
            .entry(e.child.clone())
            .or_default()
            .push(e.parent.clone());
    }
    for v in parents.values_mut() {
        v.sort();
        v.dedup();
    }
    for v in children.values_mut() {
        v.sort();
        v.dedup();
    }
    (parents, children)
}

#[allow(clippy::too_many_arguments)]
fn render_driver(
    driver: &fiftyone_folds::Driver,
    current_state: Option<&str>,
    justification: Option<&fiftyone_folds::DriverJustification>,
    parents: &[String],
    children: &[String],
    by_code: &BTreeMap<&str, &fiftyone_folds::Driver>,
    names: &Names,
    sources: &SourceIndex,
) -> String {
    let mut s = String::new();
    write_frontmatter(&mut s, driver, current_state, parents, children, names);

    s.push_str(&format!("# {} ({})\n\n", driver.name, driver.code));
    if let Some(state) = current_state {
        s.push_str(&format!("> [!info] Current State: **{state}**\n\n"));
    }

    write_possible_states(&mut s, driver, current_state);
    write_context(&mut s, driver);
    if let Some(j) = justification {
        write_rationale(&mut s, &driver.code, j, sources);
    }
    write_local_map(&mut s, driver, parents, children, by_code);
    write_in_the_model(&mut s, parents, children, names);

    s
}

fn write_frontmatter(
    s: &mut String,
    d: &fiftyone_folds::Driver,
    current_state: Option<&str>,
    parents: &[String],
    children: &[String],
    names: &Names,
) {
    s.push_str("---\n");
    s.push_str(&format!("code: {}\n", yaml_str(&d.code)));
    s.push_str(&format!("name: {}\n", yaml_str(&d.name)));
    if let Some(state) = current_state {
        s.push_str(&format!("current_state: {}\n", yaml_str(state)));
    }
    if !d.state_descriptors.is_empty() {
        s.push_str("possible_states:\n");
        for sd in &d.state_descriptors {
            s.push_str(&format!("  - {}\n", yaml_str(&sd.name)));
        }
    }

    let parent_links: Vec<String> = parents
        .iter()
        .filter_map(|p| names.driver_link(p))
        .collect();
    let child_links: Vec<String> = children
        .iter()
        .filter_map(|c| {
            if c == "DV" {
                Some("[[Overview#Outcomes]]".to_owned())
            } else {
                names.driver_link(c)
            }
        })
        .collect();

    if !parent_links.is_empty() {
        s.push_str("parents:\n");
        for l in &parent_links {
            s.push_str(&format!("  - \"{l}\"\n"));
        }
    }
    if !child_links.is_empty() {
        s.push_str("children:\n");
        for l in &child_links {
            s.push_str(&format!("  - \"{l}\"\n"));
        }
    }

    let tier = if parents.is_empty() && !children.is_empty() {
        "root"
    } else if children.iter().any(|c| c == "DV") {
        "terminal"
    } else if !children.is_empty() {
        "interior"
    } else {
        "leaf"
    };
    s.push_str(&format!("tier: {tier}\n"));
    // `entity_type` is the discriminator Drivers.base filters on. We
    // can't rely on `file.inFolder("Drivers")` because the user may
    // mount a parent directory as their vault, which makes folder
    // paths relative-to-something-unexpected.
    s.push_str("entity_type: driver\n");
    s.push_str("tags: [driver]\n");
    s.push_str("---\n\n");
}

fn write_possible_states(s: &mut String, d: &fiftyone_folds::Driver, current_state: Option<&str>) {
    if d.state_descriptors.is_empty() {
        return;
    }
    s.push_str("## Possible States\n\n");
    for sd in &d.state_descriptors {
        let is_current = current_state.map(|c| c == sd.name).unwrap_or(false);
        let kind = if is_current { "check" } else { "example" };
        let suffix = if is_current { " *(current)*" } else { "" };
        s.push_str(&format!("> [!{kind}] {}{suffix}\n", sd.name));
        for line in sd.description.lines() {
            s.push_str(&format!("> {line}\n"));
        }
        s.push('\n');
    }
}

fn write_context(s: &mut String, d: &fiftyone_folds::Driver) {
    let Some(ctx) = d.context.as_ref() else {
        return;
    };
    if !ctx.importance.trim().is_empty() {
        s.push_str("## Why This Matters\n\n");
        s.push_str(ctx.importance.trim_end());
        s.push_str("\n\n");
    }
    if !ctx.shifts.trim().is_empty() {
        s.push_str("## How It Shifts\n\n");
        s.push_str(ctx.shifts.trim_end());
        s.push_str("\n\n");
    }
    if !ctx.monitor.trim().is_empty() {
        s.push_str("## What To Monitor\n\n");
        s.push_str(ctx.monitor.trim_end());
        s.push_str("\n\n");
    }
}

fn write_rationale(
    s: &mut String,
    driver_code: &str,
    justification: &fiftyone_folds::DriverJustification,
    sources: &SourceIndex,
) {
    if justification.content.is_empty() {
        return;
    }
    s.push_str("## Current State Rationale\n\n");
    let mut used: HashSet<String> = HashSet::new();
    for para in &justification.content {
        let rewritten = rewrite_citations(para, driver_code, sources, &mut used);
        s.push_str(rewritten.trim_end());
        s.push_str("\n\n");
    }
    if !used.is_empty() {
        let mut nums: Vec<String> = used.into_iter().collect();
        nums.sort_by_key(|n| n.parse::<u64>().unwrap_or(u64::MAX));
        for num in nums {
            let key = citation_key(driver_code, &num);
            if let Some(rel) = sources.by_citation_num.get(&key) {
                let target = rel.strip_suffix(".md").unwrap_or(rel);
                s.push_str(&format!("[^{num}]: [[{target}]]\n"));
            }
        }
        s.push('\n');
    }
}

fn rewrite_citations(
    input: &str,
    driver_code: &str,
    sources: &SourceIndex,
    used: &mut HashSet<String>,
) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'['
            && let Some(end) = find_close_bracket(bytes, i)
        {
            let inner = &input[i + 1..end];
            let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();
            let all_numeric = !parts.is_empty()
                && parts
                    .iter()
                    .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
            if all_numeric {
                let mut any_known = false;
                let mut rendered = String::new();
                for (idx, n) in parts.iter().enumerate() {
                    let key = citation_key(driver_code, n);
                    if sources.by_citation_num.contains_key(&key) {
                        any_known = true;
                        if idx > 0 {
                            rendered.push(' ');
                        }
                        rendered.push_str(&format!("[^{n}]"));
                        used.insert((*n).to_owned());
                    }
                }
                if any_known {
                    out.push_str(&rendered);
                    i = end + 1;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn find_close_bracket(bytes: &[u8], open: usize) -> Option<usize> {
    let mut i = open + 1;
    while i < bytes.len() && i < open + 20 {
        match bytes[i] {
            b']' => return Some(i),
            b'[' | b'\n' => return None,
            _ => i += 1,
        }
    }
    None
}

fn write_local_map(
    s: &mut String,
    d: &fiftyone_folds::Driver,
    parents: &[String],
    children: &[String],
    by_code: &BTreeMap<&str, &fiftyone_folds::Driver>,
) {
    if parents.is_empty() && children.is_empty() {
        return;
    }
    s.push_str("## Local Causal Map\n\n");
    let lookup = |code: &str| by_code.get(code).map(|d| d.name.clone());
    s.push_str(&mermaid::local_dag(&d.code, parents, children, lookup));
    s.push('\n');
}

fn write_in_the_model(s: &mut String, parents: &[String], children: &[String], names: &Names) {
    s.push_str("## In The Model\n\n");
    if !parents.is_empty() {
        s.push_str("**Parents**\n\n");
        for p in parents {
            if let Some(l) = names.driver_link(p) {
                s.push_str(&format!("- {l}\n"));
            }
        }
        s.push('\n');
    }
    let non_dv_children: Vec<&String> = children.iter().filter(|c| c.as_str() != "DV").collect();
    if !non_dv_children.is_empty() {
        s.push_str("**Children**\n\n");
        for c in non_dv_children {
            if let Some(l) = names.driver_link(c) {
                s.push_str(&format!("- {l}\n"));
            }
        }
        s.push('\n');
    }
    if children.iter().any(|c| c == "DV") {
        s.push_str("**Reaches DV** — this driver feeds directly into the outcome distribution. See [[Overview]] for the outcome breakdown.\n\n");
    }
}

fn yaml_str(value: &str) -> String {
    let needs_quote = value.is_empty()
        || value.chars().any(|c| {
            matches!(
                c,
                ':' | '#' | '&' | '*' | '?' | '|' | '<' | '>' | '!' | '%' | '@' | '`' | '"' | '\''
            )
        })
        || value.starts_with(['-', '[', '{', '"', '\'']);
    if needs_quote {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sources_index() -> SourceIndex {
        let mut idx = SourceIndex {
            by_citation_num: BTreeMap::new(),
            by_url: BTreeMap::new(),
        };
        idx.by_citation_num.insert(
            citation_key("X", "1"),
            "Sources/1 — example.com.md".to_owned(),
        );
        idx.by_citation_num.insert(
            citation_key("X", "2"),
            "Sources/2 — example.org.md".to_owned(),
        );
        idx
    }

    #[test]
    fn rewrite_converts_known_markers_only() {
        let idx = make_sources_index();
        let mut used = HashSet::new();
        let out = rewrite_citations(
            "Trade rose [1] sharply but [the executive] disagreed [99].",
            "X",
            &idx,
            &mut used,
        );
        assert!(out.contains("[^1]"), "{out}");
        assert!(out.contains("[the executive]"), "{out}");
        assert!(
            out.contains("[99]"),
            "unknown number should pass through: {out}"
        );
        assert_eq!(used.len(), 1);
    }

    #[test]
    fn rewrite_handles_grouped_markers() {
        let idx = make_sources_index();
        let mut used = HashSet::new();
        let out = rewrite_citations("Two refs [1, 2].", "X", &idx, &mut used);
        assert!(out.contains("[^1]"), "{out}");
        assert!(out.contains("[^2]"), "{out}");
        assert_eq!(used.len(), 2);
    }
}
