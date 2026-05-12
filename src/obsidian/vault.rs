//! Vault scaffolding: directory tree plus a seeded `.obsidian/` config so
//! the folder opens fully wired (Canvas + Bases + Graph) on first use.

use anyhow::Result;
use std::path::Path;

/// Create the vault root and the canonical sub-directories. Idempotent.
pub(crate) fn scaffold(root: &Path) -> Result<()> {
    std::fs::create_dir_all(root)?;
    for sub in ["Drivers", "Outcomes", "Sources", "Data", ".obsidian"] {
        std::fs::create_dir_all(root.join(sub))?;
    }

    std::fs::write(root.join(".obsidian/app.json"), APP_JSON)?;
    std::fs::write(root.join(".obsidian/core-plugins.json"), CORE_PLUGINS_JSON)?;
    std::fs::write(root.join(".obsidian/graph.json"), GRAPH_JSON)?;
    Ok(())
}

/// Minimal app config. `showUnsupportedFiles` makes the provenance
/// `Data/model.json` visible in the file explorer — Obsidian hides
/// non-markdown files by default, which left the `Data/` folder
/// looking empty in the tree.
const APP_JSON: &str = r#"{
  "newFileLocation": "root",
  "alwaysUpdateLinks": true,
  "useMarkdownLinks": false,
  "showLineNumber": false,
  "showUnsupportedFiles": true,
  "readableLineLength": false
}
"#;

/// The export relies on Canvas (`Model.canvas`), Bases (`Drivers.base`,
/// `Sources Index.base`), Properties (typed frontmatter), and Graph (the
/// auto-derived cross-driver map). Pre-enable all of them so the user
/// doesn't have to.
const CORE_PLUGINS_JSON: &str = r#"[
  "file-explorer",
  "global-search",
  "switcher",
  "graph",
  "backlink",
  "outgoing-link",
  "tag-pane",
  "properties",
  "page-preview",
  "note-composer",
  "command-palette",
  "outline",
  "word-count",
  "canvas",
  "bases"
]
"#;

/// Pre-coloured graph groups make the three clusters (drivers / outcomes /
/// sources) immediately distinguishable when the user opens Graph View.
const GRAPH_JSON: &str = r#"{
  "collapse-filter": false,
  "search": "",
  "showTags": false,
  "showAttachments": false,
  "hideUnresolved": false,
  "showOrphans": true,
  "collapse-color-groups": false,
  "colorGroups": [
    { "query": "path:Drivers/",  "color": { "a": 1, "rgb": 5816530 } },
    { "query": "path:Outcomes/", "color": { "a": 1, "rgb": 14079702 } },
    { "query": "path:Sources/",  "color": { "a": 1, "rgb": 10708548 } }
  ],
  "collapse-display": false,
  "showArrow": true,
  "textFadeMultiplier": 0,
  "nodeSizeMultiplier": 1.1,
  "lineSizeMultiplier": 1,
  "collapse-forces": false,
  "centerStrength": 0.5,
  "repelStrength": 12,
  "linkStrength": 1,
  "linkDistance": 250,
  "scale": 1,
  "close": true
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffold_creates_expected_layout() {
        let dir = tempdir();
        scaffold(&dir).unwrap();
        for p in [
            "Drivers",
            "Outcomes",
            "Sources",
            "Data",
            ".obsidian/app.json",
            ".obsidian/core-plugins.json",
            ".obsidian/graph.json",
        ] {
            assert!(dir.join(p).exists(), "missing {p}");
        }
    }

    /// Lightweight temp dir helper — we keep the dependency footprint small
    /// rather than pulling in `tempfile` for one test path.
    pub(crate) fn tempdir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let n = format!(
            "hedgehog-obsidian-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        p.push(n);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
