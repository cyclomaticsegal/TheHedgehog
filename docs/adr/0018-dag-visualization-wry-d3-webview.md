# ADR 0018: DAG Visualization via D3.js in an Embedded wry WebView

**Date:** 2026-04-14
**Status:** Accepted
**Extends:** ADR 0013 (SDK integration, model explorer)

## Context

The 51Folds model response includes a full directed acyclic graph: 15 driver nodes + 1 outcome node ("DV") connected by ~60 directed edges. This graph data (`ModelResponse.edges`) was fetched, persisted, and loaded into memory but never visualized. The 51Folds native web UI renders this as an interactive "Visual Map" using D3.js and dagre — we wanted to match that quality inside the desktop app.

Three approaches were considered:

1. **Pure egui custom painting** — `petgraph` for layout, egui `Painter` for rendering. Stays in one window but produces a visually inferior result for graph visualization, which egui is not designed for.
2. **D3.js in a separate wry window** — full D3 quality but breaks the single-window design.
3. **D3.js in a wry child WebView** — WKWebView as a native subview of the existing eframe window. Full D3 rendering, same window, bidirectional IPC.

Option 3 was chosen.

## Decisions

### 1. wry child WebView embedded in the eframe window

`wry::WebViewBuilder::build_as_child()` creates a WKWebView as an NSView subview of the existing eframe window. On macOS, this calls `ns_view.addSubview(&webview)` — the webview is a native view within the same window, not a separate window.

The `RawWindowHandle` is captured from eframe's `CreationContext` at construction time (it implements `HasWindowHandle` in eframe 0.31) and stored in a `StoredWindowHandle` wrapper for lazy webview creation.

### 2. Positioning via `set_bounds()` each frame

Every frame that the Visual Map tab is active, `render_dag_view()` gets the egui panel's available rect, converts to logical coordinates, and calls `webview.set_bounds()`. This keeps the webview aligned with the egui panel as the window resizes or panels change size.

### 3. Visibility management

The webview is a native OS view on top of the GPU-rendered egui surface. It cannot participate in egui's z-ordering — egui popups, tooltips, and windows render behind it. The solution: hide the webview whenever the user is not on the Visual Map tab, or when any modal is open (Help window, splash screen, driver detail page).

A visibility check runs at the top of every `update()` frame.

### 4. Transparent background to prevent white flash

The webview starts with `with_transparent(true)` so its native backing is transparent. Before the HTML renders, the user sees through to the dark egui panel behind it. Once the HTML loads, `body { background: #0a0e1a }` paints the matching dark colour. A "ready" IPC signal from the JS side gates the first `set_visible(true)` call as an additional safeguard.

### 5. Self-contained HTML with inlined D3 + dagre

The HTML page (`assets/dag.html`) bundles D3.js v7 (~280KB) and dagre (~284KB) as inline `<script>` tags. A build step assembles `dag-bundle.html` (~561KB total) which is embedded in the binary via `include_str!`. No CDN dependency, no network access required.

dagre-d3 was evaluated (~725KB) but rejected in favour of using dagre for layout and D3 for rendering directly, giving full control over the visual style.

### 6. Visual design matching 51Folds

- Gold/tan circular nodes (#d4a574) with abbreviated code labels
- Red outcome node at the bottom
- Straight-line edges (not curves), thin (#4a5568, 1px, 40% opacity)
- Tight dagre layout (`nodesep: 40`, `ranksep: 55`)
- Hover: connected edges highlight blue, path-to-DV highlights red, unconnected nodes dim
- Click: sends driver code to Rust via IPC → navigates to driver detail page

### 7. Bidirectional IPC

- **JS → Rust**: `window.ipc.postMessage(JSON.stringify({type: 'click', code: 'CBP'}))` → parsed in the `with_ipc_handler` closure, forwarded via `mpsc::channel` to `DagIpcMessage::NodeClicked`
- **Rust → JS**: `webview.evaluate_script("window.updateDAG('...')")` sends serialized model data; `window.updateStates('...')` sends lightweight colour-only updates when driver states change

### 8. Back navigation

Clicking a node in the Visual Map navigates to `ModelView::DriverDetail(idx)`. A `model_view_back` field tracks the origin — the back button shows "Visual Map" (returning to the graph) rather than "Drivers" (which is the default back target when arriving from the driver list).

## Consequences

- `wry` is a new dependency (~adds WebKit bridging on macOS, WebView2 on Windows)
- The DAG visualization is limited to platforms where wry can create a child webview (macOS confirmed; Windows and Linux likely work but untested)
- The HTML bundle adds ~561KB to the binary size
- The webview is `!Send` — only usable on the main thread, which is where `DashboardApp` runs
- Future work: state-aware node colouring (currently all nodes are gold; could map to driver state colours), animated transitions on hover
