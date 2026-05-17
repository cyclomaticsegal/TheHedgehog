//! Versioned per-merge snapshots — the diff baseline that anchors
//! [`crate::obsidian::merge::read_vault_diff`].
//!
//! Each successful export (initial or post-merge) writes one
//! `Data/snapshots/v{NNN}.json`. The original export writes `v001.json`;
//! every selective re-export after a merge appends the next number. The
//! highest-numbered file is always the baseline for the *next* merge.
//!
//! The schema is intentionally narrow — just what the merge reader needs
//! to compute a diff. The full `ModelResponse` lives in `Data/model.json`.
//!
//! Format (`schema_version: 2`):
//!
//! ```json
//! {
//!   "schema_version": 2,
//!   "model_id": "...",
//!   "exported_at": "2026-05-15T10:42:00Z",
//!   "driver_states": { "UEBSP": "Hawkish", ... },
//!   "outcome_probabilities": { "1": 0.32, ... }
//! }
//! ```
//!
//! `schema_version` was bumped from `1` (the single-file
//! `Data/snapshot.json` shape proposed in the original ADR) to `2` when
//! we moved to the versioned-directory layout. The merge reader fails
//! fast if neither shape is present.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) const SCHEMA_VERSION: u32 = 2;
pub(crate) const SNAPSHOTS_DIRNAME: &str = "snapshots";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Snapshot {
    pub schema_version: u32,
    pub model_id: String,
    pub exported_at: String,
    /// Driver code → state name. Sorted via BTreeMap so JSON output is
    /// stable across runs — useful for `git diff` on vaults that the
    /// user has under version control.
    pub driver_states: BTreeMap<String, String>,
    /// Outcome id → probability (0.0–1.0).
    pub outcome_probabilities: BTreeMap<i64, f64>,
}

impl Snapshot {
    pub(crate) fn from_model(model: &fiftyone_folds::ModelResponse) -> Self {
        let driver_states = model
            .current
            .drivers
            .iter()
            .map(|ds| (ds.code.clone(), ds.state.clone()))
            .collect();
        let outcome_probabilities = model
            .current
            .outcomes
            .iter()
            .map(|o| (o.id, o.probability.unwrap_or(0.0)))
            .collect();
        Self {
            schema_version: SCHEMA_VERSION,
            model_id: model.model_id.clone(),
            exported_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            driver_states,
            outcome_probabilities,
        }
    }
}

/// Write the *next* numbered snapshot under `vault_root/Data/snapshots/`.
///
/// Finds the highest existing `vNNN.json` (or starts at `v001.json` if the
/// directory is empty) and writes the new file as the next number.
/// Returns the absolute path of the written file.
pub(crate) fn write_next(vault_root: &Path, snapshot: &Snapshot) -> Result<PathBuf> {
    let dir = vault_root.join("Data").join(SNAPSHOTS_DIRNAME);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating snapshots dir {}", dir.display()))?;

    let next = next_version_in(&dir)?;
    let path = dir.join(format!("v{next:03}.json"));
    let pretty = serde_json::to_string_pretty(snapshot)?;
    std::fs::write(&path, pretty)
        .with_context(|| format!("writing snapshot {}", path.display()))?;
    Ok(path)
}

/// Load the highest-numbered snapshot. Returns `Ok(None)` if the
/// directory is missing or empty — the merge reader uses this to surface
/// "re-export to enable merge" rather than guessing at a baseline.
#[allow(dead_code)] // used by merge.rs (slice 3) and tests
pub(crate) fn read_latest(vault_root: &Path) -> Result<Option<(PathBuf, Snapshot)>> {
    let dir = vault_root.join("Data").join(SNAPSHOTS_DIRNAME);
    if !dir.exists() {
        return Ok(None);
    }
    let Some(highest) = highest_version_in(&dir)? else {
        return Ok(None);
    };
    let path = dir.join(format!("v{highest:03}.json"));
    let bytes = std::fs::read(&path)
        .with_context(|| format!("reading snapshot {}", path.display()))?;
    let snap: Snapshot = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing snapshot {}", path.display()))?;
    Ok(Some((path, snap)))
}

fn next_version_in(dir: &Path) -> Result<u32> {
    Ok(highest_version_in(dir)?.map(|n| n + 1).unwrap_or(1))
}

fn highest_version_in(dir: &Path) -> Result<Option<u32>> {
    let mut highest: Option<u32> = None;
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("listing {}", dir.display()))?
        .flatten()
    {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(num) = parse_version_filename(name) else {
            continue;
        };
        highest = Some(match highest {
            Some(h) if h >= num => h,
            _ => num,
        });
    }
    Ok(highest)
}

fn parse_version_filename(name: &str) -> Option<u32> {
    let stem = name.strip_suffix(".json")?;
    let digits = stem.strip_prefix('v')?;
    digits.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tempdir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "hedgehog-snapshot-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn empty_snapshot() -> Snapshot {
        Snapshot {
            schema_version: SCHEMA_VERSION,
            model_id: "m_test".into(),
            exported_at: "2026-05-15T10:42:00Z".into(),
            driver_states: BTreeMap::new(),
            outcome_probabilities: BTreeMap::new(),
        }
    }

    #[test]
    fn first_write_creates_v001() {
        let root = tempdir();
        let snap = empty_snapshot();
        let path = write_next(&root, &snap).expect("write");
        assert!(path.ends_with("v001.json"), "{}", path.display());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn second_write_appends_v002() {
        let root = tempdir();
        let snap = empty_snapshot();
        let _ = write_next(&root, &snap).unwrap();
        let p2 = write_next(&root, &snap).unwrap();
        assert!(p2.ends_with("v002.json"), "{}", p2.display());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn read_latest_returns_highest() {
        let root = tempdir();
        let snap = empty_snapshot();
        write_next(&root, &snap).unwrap();
        let mut later = snap.clone();
        later.model_id = "m_later".into();
        write_next(&root, &later).unwrap();
        let (path, loaded) = read_latest(&root).unwrap().expect("snapshot present");
        assert!(path.ends_with("v002.json"));
        assert_eq!(loaded.model_id, "m_later");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn read_latest_returns_none_when_missing() {
        let root = tempdir();
        assert!(read_latest(&root).unwrap().is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn read_latest_skips_non_version_files() {
        let root = tempdir();
        let snap = empty_snapshot();
        write_next(&root, &snap).unwrap();
        // Drop a stray file in the directory — should be ignored.
        let dir = root.join("Data").join(SNAPSHOTS_DIRNAME);
        std::fs::write(dir.join("scratch.txt"), "noise").unwrap();
        let (path, _) = read_latest(&root).unwrap().expect("snapshot");
        assert!(path.ends_with("v001.json"));
        std::fs::remove_dir_all(&root).ok();
    }
}
