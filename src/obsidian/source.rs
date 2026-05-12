//! Per-citation `Sources/<n> — <domain>.md` files.
//!
//! Every unique citation URL across all driver justifications becomes one
//! source note. Driver pages link to these notes via wikilinks in their
//! footnote definitions, so Obsidian's backlinks pane on a source note
//! becomes a "which drivers cite this evidence" view for free — the
//! single most valuable cross-cutting affordance the export creates.

use crate::obsidian::names::{Names, domain_of};
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::Path;

/// Lookup table built during source writing and consumed by `driver.rs`
/// to rewrite inline `[N]` references into Obsidian footnotes.
///
/// Maps the original 51Folds citation number (as a string, since the API
/// types it that way) to the source's vault-relative path.
pub(crate) struct SourceIndex {
    pub by_citation_num: BTreeMap<String, String>,
    pub by_url: BTreeMap<String, String>,
}

impl SourceIndex {
    fn empty() -> Self {
        Self {
            by_citation_num: BTreeMap::new(),
            by_url: BTreeMap::new(),
        }
    }
}

pub(crate) fn write_all(
    model: &fiftyone_folds::ModelResponse,
    names: &Names,
    root: &Path,
) -> Result<SourceIndex> {
    let mut index = SourceIndex::empty();

    // Map each citation number we've seen to its URL, so driver pages
    // can rewrite inline `[N]` markers even when N is a per-driver-local
    // number that happens to point to a deduped vault-global source.
    for ds in &model.current.drivers {
        let Some(just) = ds.justification.as_ref() else {
            continue;
        };
        for cit in &just.citations {
            if let Some((_, path)) = names.sources.get(&cit.source) {
                index
                    .by_citation_num
                    .entry(citation_key(&ds.code, &cit.num))
                    .or_insert_with(|| path.clone());
                index
                    .by_url
                    .entry(cit.source.clone())
                    .or_insert_with(|| path.clone());
            }
        }
    }

    // Write one note per unique URL.
    let sources_dir = root.join("Sources");
    std::fs::create_dir_all(&sources_dir)?;
    for (url, (num, rel_path)) in &names.sources {
        let abs = root.join(rel_path);
        let body = render(*num, url);
        std::fs::write(&abs, body)?;
    }

    Ok(index)
}

/// Key used to look up a citation by `(driver_code, num)` — the same
/// citation number can mean different URLs across different drivers in
/// the 51Folds response, so we have to scope the lookup.
pub(crate) fn citation_key(driver_code: &str, num: &str) -> String {
    format!("{driver_code}::{num}")
}

fn render(num: usize, url: &str) -> String {
    let domain = domain_of(url);
    let mut s = String::new();
    s.push_str("---\n");
    s.push_str(&format!("num: {num}\n"));
    s.push_str(&format!("url: \"{}\"\n", yaml_escape(url)));
    s.push_str(&format!("domain: \"{}\"\n", yaml_escape(&domain)));
    s.push_str("entity_type: source\n");
    s.push_str("tags: [source]\n");
    s.push_str("---\n\n");

    s.push_str(&format!("# Source {num} — {domain}\n\n"));
    s.push_str(&format!("> [!cite] Reference\n> <{url}>\n\n"));
    s.push_str("## Cited By\n\n");
    s.push_str(
        "_Open the **Backlinks** pane (right sidebar) to see every driver that cites this source._\n",
    );
    s
}

fn yaml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
