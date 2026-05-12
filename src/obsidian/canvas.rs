//! `Model.canvas` — JSON Canvas 1.0 export with a pure-Rust layered
//! layout.
//!
//! We don't pull in a graph-layout dependency. The model size (~10–20
//! drivers, ~50 edges in practice) is small enough that a hand-rolled
//! Kahn-longest-path layer assignment plus a single barycenter sweep
//! produces a perfectly readable map. The output file conforms to the
//! JSON Canvas 1.0 spec — see <https://jsoncanvas.org/spec/1.0>.

use crate::obsidian::names::Names;
use anyhow::Result;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const X_SPACING: i32 = 360;
const Y_SPACING: i32 = 130;
const DRIVER_W: i32 = 260;
const DRIVER_H: i32 = 80;
const OUTCOME_W: i32 = 320;
const OUTCOME_H: i32 = 110;
const DV_W: i32 = 220;
const DV_H: i32 = 110;

pub(crate) fn write(
    model: &fiftyone_folds::ModelResponse,
    names: &Names,
    root: &Path,
) -> Result<()> {
    let canvas = build_canvas(model, names);
    let json = serde_json::to_string_pretty(&canvas)?;
    std::fs::write(root.join("Model.canvas"), json)?;
    Ok(())
}

#[derive(Serialize)]
struct Canvas {
    nodes: Vec<Node>,
    edges: Vec<CanvasEdge>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Node {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    background: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanvasEdge {
    id: String,
    from_node: String,
    to_node: String,
    from_side: &'static str,
    to_side: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
}

fn build_canvas(model: &fiftyone_folds::ModelResponse, names: &Names) -> Canvas {
    let layout = compute_layout(model);

    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // Driver nodes
    for d in &model.drivers {
        let pos = layout
            .positions
            .get(d.code.as_str())
            .copied()
            .unwrap_or((0, 0));
        let file = names
            .drivers
            .get(&d.code)
            .cloned()
            .unwrap_or_else(|| format!("Drivers/{}.md", d.code));
        nodes.push(Node {
            id: node_id_for_driver(&d.code),
            kind: "file",
            x: pos.0,
            y: pos.1,
            width: DRIVER_W,
            height: DRIVER_H,
            color: None,
            file: Some(file),
            text: None,
            label: None,
            background: None,
        });
    }

    // DV node — a text card carrying the model question for context.
    let dv_pos = layout.positions.get("DV").copied().unwrap_or((0, 0));
    nodes.push(Node {
        id: "DV".to_owned(),
        kind: "text",
        x: dv_pos.0,
        y: dv_pos.1,
        width: DV_W,
        height: DV_H,
        color: Some("4".to_owned()),
        file: None,
        text: Some(format!(
            "## Dependent Variable\n\n{}",
            truncate_for_card(&model.question, 200)
        )),
        label: None,
        background: None,
    });

    // Outcome nodes — one per outcome, each linked to the matching .md.
    for o in &model.current.outcomes {
        let key = outcome_layout_key(o.id);
        let pos = layout
            .positions
            .get(key.as_str())
            .copied()
            .unwrap_or((0, 0));
        let file = names
            .outcomes
            .get(&o.id)
            .cloned()
            .unwrap_or_else(|| format!("Outcomes/{}.md", o.id));
        nodes.push(Node {
            id: format!("out-{}", o.id),
            kind: "file",
            x: pos.0,
            y: pos.1,
            width: OUTCOME_W,
            height: OUTCOME_H,
            color: Some("6".to_owned()),
            file: Some(file),
            text: None,
            label: None,
            background: None,
        });
    }

    // Model edges, plus synthetic DV→outcome edges so the funnel is visible.
    let mut edge_counter = 0usize;
    for e in &model.edges {
        let from = node_id_for_driver(&e.parent);
        let to = if e.child == "DV" {
            "DV".to_owned()
        } else {
            node_id_for_driver(&e.child)
        };
        let color = if e.child == "DV" {
            Some("4".to_owned())
        } else {
            Some("5".to_owned())
        };
        edge_counter += 1;
        edges.push(CanvasEdge {
            id: format!("e{edge_counter}"),
            from_node: from,
            to_node: to,
            from_side: "right",
            to_side: "left",
            color,
        });
    }
    for o in &model.current.outcomes {
        edge_counter += 1;
        edges.push(CanvasEdge {
            id: format!("e{edge_counter}"),
            from_node: "DV".to_owned(),
            to_node: format!("out-{}", o.id),
            from_side: "right",
            to_side: "left",
            color: Some("6".to_owned()),
        });
    }

    Canvas { nodes, edges }
}

fn node_id_for_driver(code: &str) -> String {
    format!("d-{code}")
}

fn outcome_layout_key(id: i64) -> String {
    format!("__out__{id}")
}

fn truncate_for_card(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_owned();
    }
    let mut out: String = trimmed.chars().take(max_chars).collect();
    out.push('…');
    out
}

/// Result of the layered layout: an absolute (x, y) for each node key.
/// Keys are the driver code, the literal `"DV"`, or `__out__<id>` for an
/// outcome.
struct Layout {
    positions: BTreeMap<String, (i32, i32)>,
}

fn compute_layout(model: &fiftyone_folds::ModelResponse) -> Layout {
    let driver_codes: Vec<&str> = model.drivers.iter().map(|d| d.code.as_str()).collect();
    let mut node_keys: BTreeSet<String> = driver_codes.iter().map(|c| c.to_string()).collect();
    node_keys.insert("DV".to_owned());
    let outcome_keys: Vec<String> = model
        .current
        .outcomes
        .iter()
        .map(|o| outcome_layout_key(o.id))
        .collect();
    for k in &outcome_keys {
        node_keys.insert(k.clone());
    }
    // Also defensively register any driver code referenced in edges but
    // missing from `model.drivers` — keeps the layout total.
    for e in &model.edges {
        if e.parent != "DV" {
            node_keys.insert(e.parent.clone());
        }
        if e.child != "DV" {
            node_keys.insert(e.child.clone());
        }
    }

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
    // Synthetic DV → outcome edges.
    for k in &outcome_keys {
        parents.entry(k.clone()).or_default().push("DV".to_owned());
        children.entry("DV".to_owned()).or_default().push(k.clone());
    }

    // Longest-path layer assignment: layer(v) = max(layer(p) for p in parents(v)) + 1.
    let mut layer: BTreeMap<String, usize> = BTreeMap::new();
    let mut changed = true;
    let mut guard = 0;
    while changed && guard < 1_000 {
        changed = false;
        guard += 1;
        for k in &node_keys {
            let new_layer = parents
                .get(k)
                .map(|ps| {
                    ps.iter()
                        .map(|p| layer.get(p).copied().unwrap_or(0))
                        .max()
                        .unwrap_or(0)
                })
                .map(|m| {
                    if parents.get(k).map(|v| !v.is_empty()).unwrap_or(false) {
                        m + 1
                    } else {
                        0
                    }
                })
                .unwrap_or(0);
            let prev = layer.get(k).copied().unwrap_or(usize::MAX);
            if prev == usize::MAX || new_layer > prev {
                layer.insert(k.clone(), new_layer);
                changed = true;
            }
        }
    }

    // Force DV after every driver, and outcomes after DV — gives a clean
    // left-to-right reading order even if the underlying DAG would
    // otherwise pack DV earlier.
    let max_driver_layer = driver_codes
        .iter()
        .filter_map(|c| layer.get(*c).copied())
        .max()
        .unwrap_or(0);
    layer.insert("DV".to_owned(), max_driver_layer + 1);
    for k in &outcome_keys {
        layer.insert(k.clone(), max_driver_layer + 2);
    }

    // Bucket nodes by layer (stable, ascending key order).
    let mut layers: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (k, l) in &layer {
        layers.entry(*l).or_default().push(k.clone());
    }
    for v in layers.values_mut() {
        v.sort();
    }

    // Provisional y assignment by alphabetical order within each layer,
    // then one barycenter sweep to reduce crossings.
    let mut y_index: BTreeMap<String, f64> = BTreeMap::new();
    for ks in layers.values() {
        for (i, k) in ks.iter().enumerate() {
            y_index.insert(k.clone(), i as f64);
        }
    }
    for ks in layers.values_mut() {
        // Compute barycenter from each node's parent y-positions.
        let mut by_score: Vec<(f64, String)> = ks
            .iter()
            .map(|k| {
                let score = parents
                    .get(k)
                    .map(|ps| {
                        if ps.is_empty() {
                            y_index.get(k).copied().unwrap_or(0.0)
                        } else {
                            ps.iter()
                                .map(|p| y_index.get(p).copied().unwrap_or(0.0))
                                .sum::<f64>()
                                / (ps.len() as f64)
                        }
                    })
                    .unwrap_or(0.0);
                (score, k.clone())
            })
            .collect();
        by_score.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        ks.clear();
        for (i, (_, k)) in by_score.into_iter().enumerate() {
            y_index.insert(k.clone(), i as f64);
            ks.push(k);
        }
    }

    // Translate (layer, y_index) → absolute (x, y).
    let mut positions = BTreeMap::new();
    for (layer_idx, ks) in &layers {
        let count = ks.len() as i32;
        for (i, k) in ks.iter().enumerate() {
            let x = (*layer_idx as i32) * X_SPACING;
            let y = (i as i32 - count / 2) * Y_SPACING;
            positions.insert(k.clone(), (x, y));
        }
    }
    Layout { positions }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fiftyone_folds::{CurrentState, Driver, Edge, ModelResponse, Outcome};

    fn simple_model() -> ModelResponse {
        let mut m = ModelResponse {
            model_id: "M1".into(),
            ownership: String::new(),
            status: "Successed".into(),
            created_at: String::new(),
            updated_at: String::new(),
            question: "Will it rain?".into(),
            context: String::new(),
            short_summary: String::new(),
            drivers: vec![
                Driver {
                    code: "A".into(),
                    name: "A".into(),
                    state_descriptors: vec![],
                    context: None,
                },
                Driver {
                    code: "B".into(),
                    name: "B".into(),
                    state_descriptors: vec![],
                    context: None,
                },
            ],
            edges: vec![
                Edge {
                    parent: "A".into(),
                    child: "B".into(),
                },
                Edge {
                    parent: "B".into(),
                    child: "DV".into(),
                },
            ],
            current: CurrentState::default(),
        };
        m.current.outcomes.push(Outcome {
            id: 1,
            label: "Yes".into(),
            probability: Some(0.5),
        });
        m
    }

    #[test]
    fn layers_are_strictly_left_to_right() {
        let m = simple_model();
        let layout = compute_layout(&m);
        let xa = layout.positions["A"].0;
        let xb = layout.positions["B"].0;
        let xdv = layout.positions["DV"].0;
        let xout = layout.positions[&outcome_layout_key(1)].0;
        assert!(xa < xb, "A should be left of B");
        assert!(xb < xdv, "B should be left of DV");
        assert!(xdv < xout, "DV should be left of outcome");
    }

    #[test]
    fn canvas_serialises_to_jsoncanvas_shape() {
        let m = simple_model();
        let names = Names::compute(&m);
        let canvas = build_canvas(&m, &names);
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&canvas).unwrap()).unwrap();
        assert!(v["nodes"].is_array());
        assert!(v["edges"].is_array());
        // Drivers + DV + outcomes = 2 + 1 + 1 = 4 nodes.
        assert_eq!(v["nodes"].as_array().unwrap().len(), 4);
        // Model edges + synthetic DV→outcome = 2 + 1 = 3 edges.
        assert_eq!(v["edges"].as_array().unwrap().len(), 3);
    }
}
