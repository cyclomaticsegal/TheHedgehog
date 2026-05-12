//! Centralised filename computation. Every other generator consults
//! [`Names`] so that wikilinks and file paths agree across the vault.

use std::collections::BTreeMap;

/// Precomputed filesystem names for every entity in a model.
pub(crate) struct Names {
    pub vault_dir: String,
    /// Driver code → `"Drivers/UEBSP — US Executive Branch ...md"` (relative to vault root).
    pub drivers: BTreeMap<String, String>,
    /// Outcome id → `"Outcomes/1 — Full pivot ....md"`.
    pub outcomes: BTreeMap<i64, String>,
    /// Unique citation URL → `"Sources/3 — example.com.md"`.
    /// Order is the canonical numbering used for `[^N]` footnotes.
    pub sources: BTreeMap<String, (usize, String)>,
}

impl Names {
    pub fn compute(model: &fiftyone_folds::ModelResponse) -> Self {
        let vault_dir = vault_dir_from_question(&model.question, &model.model_id);

        let mut drivers = BTreeMap::new();
        for d in &model.drivers {
            let path = format!("Drivers/{} — {}.md", sanitise(&d.code), sanitise(&d.name));
            drivers.insert(d.code.clone(), path);
        }

        let mut outcomes = BTreeMap::new();
        for o in &model.current.outcomes {
            let label = truncate(&o.label, 60);
            let path = format!("Outcomes/{} — {}.md", o.id, sanitise(&label));
            outcomes.insert(o.id, path);
        }

        // Collect unique citation URLs in the order they're encountered.
        // Numbering is 1-based and stable across the vault.
        let mut sources: BTreeMap<String, (usize, String)> = BTreeMap::new();
        let mut counter: usize = 0;
        for ds in &model.current.drivers {
            let Some(just) = ds.justification.as_ref() else {
                continue;
            };
            for cit in &just.citations {
                if sources.contains_key(&cit.source) {
                    continue;
                }
                counter += 1;
                let path = format!(
                    "Sources/{} — {}.md",
                    counter,
                    sanitise(&domain_of(&cit.source))
                );
                sources.insert(cit.source.clone(), (counter, path));
            }
        }

        Self {
            vault_dir,
            drivers,
            outcomes,
            sources,
        }
    }

    /// `[[Drivers/CODE — Name]]` (the wikilink form, no `.md`).
    pub fn driver_link(&self, code: &str) -> Option<String> {
        self.drivers
            .get(code)
            .map(|p| format!("[[{}]]", strip_md(p)))
    }

    pub fn outcome_link(&self, id: i64) -> Option<String> {
        self.outcomes
            .get(&id)
            .map(|p| format!("[[{}]]", strip_md(p)))
    }

    #[allow(dead_code)]
    pub fn source_link(&self, url: &str) -> Option<String> {
        self.sources
            .get(url)
            .map(|(_, p)| format!("[[{}]]", strip_md(p)))
    }
}

fn strip_md(path: &str) -> &str {
    path.strip_suffix(".md").unwrap_or(path)
}

/// Trim a string and replace characters that confuse filesystems (and
/// Obsidian's wikilink parser) with safe substitutes. We keep Unicode
/// letters but drop the structural ones.
pub(crate) fn sanitise(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            // These break wikilinks (`[[...]]`) or file paths.
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '[' | ']' | '#' | '^' => {
                out.push(' ');
            }
            '\n' | '\r' | '\t' => out.push(' '),
            c => out.push(c),
        }
    }
    // Collapse double spaces and trim.
    let collapsed: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(&collapsed, 120)
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    s.chars().take(max_chars).collect()
}

fn vault_dir_from_question(question: &str, model_id: &str) -> String {
    let base = if question.trim().is_empty() {
        format!("51Folds Model {model_id}")
    } else {
        truncate(question.trim(), 80)
    };
    let safe = sanitise(&base);
    if safe.is_empty() {
        format!("51Folds Model {model_id}")
    } else {
        safe
    }
}

/// Extract the host from a URL, falling back to the trimmed URL itself
/// (or "source") if parsing fails. We deliberately don't pull in the
/// `url` crate just for this — a hand-rolled extract is enough.
pub(crate) fn domain_of(url: &str) -> String {
    // Strip scheme.
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    // Domain is up to the next /, ?, or # — strip auth too just in case.
    let host_part = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    let host = host_part.rsplit('@').next().unwrap_or(host_part);
    let host = host.trim_start_matches("www.");
    if host.is_empty() {
        "source".to_owned()
    } else {
        host.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_of_strips_scheme_and_www() {
        assert_eq!(domain_of("https://www.example.com/path?a=1"), "example.com");
        assert_eq!(domain_of("http://example.com"), "example.com");
        assert_eq!(domain_of("example.com/x"), "example.com");
        assert_eq!(domain_of(""), "source");
    }

    #[test]
    fn sanitise_strips_structural_chars() {
        assert_eq!(
            sanitise("a/b:c*d?e\"f<g>h|i[j]k#l^m"),
            "a b c d e f g h i j k l m"
        );
    }

    #[test]
    fn sanitise_collapses_whitespace() {
        assert_eq!(sanitise("  hello   world  \n   foo  "), "hello world foo");
    }
}
