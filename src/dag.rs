//! DAG visualization for 51Folds causal models via an embedded wry WebView.
//!
//! The WebView hosts a self-contained HTML page with D3.js + dagre for
//! interactive graph layout. It is created as a native child view inside
//! the eframe window (WKWebView on macOS) and repositioned each frame to
//! track the egui panel rect.

use raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use serde::Serialize;
use std::sync::mpsc;

/// The bundled HTML with D3 + dagre inlined at build time.
const DAG_HTML: &str = include_str!("../assets/dag-bundle.html");

// ---------------------------------------------------------------------------
// Window handle wrapper
// ---------------------------------------------------------------------------

/// Stores the raw window + display handles captured from `CreationContext`
/// so we can create the wry WebView lazily (outside the `new()` call).
///
/// # Safety
/// The raw pointers inside the handles point to the NSView / NSWindow
/// owned by eframe. They remain valid for the entire app lifetime because
/// eframe does not destroy the window until `run_native` returns.
pub struct StoredWindowHandle {
    window: RawWindowHandle,
    display: RawDisplayHandle,
}

unsafe impl Send for StoredWindowHandle {}

impl StoredWindowHandle {
    pub fn new(window: RawWindowHandle, display: RawDisplayHandle) -> Self {
        Self { window, display }
    }
}

impl HasWindowHandle for StoredWindowHandle {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        // Safety: the raw handle is valid for the eframe window lifetime.
        unsafe { Ok(raw_window_handle::WindowHandle::borrow_raw(self.window)) }
    }
}

impl HasDisplayHandle for StoredWindowHandle {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        unsafe { Ok(raw_window_handle::DisplayHandle::borrow_raw(self.display)) }
    }
}

// ---------------------------------------------------------------------------
// IPC messages (JS → Rust)
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum DagIpcMessage {
    NodeClicked { code: String },
    Ready,
}

// ---------------------------------------------------------------------------
// Data serialization (Rust → JS)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct DagNode {
    code: String,
    name: String,
    state: String,
}

#[derive(Serialize)]
struct DagEdge {
    parent: String,
    child: String,
}

#[derive(Serialize)]
struct DagPayload {
    drivers: Vec<DagNode>,
    edges: Vec<DagEdge>,
    outcomes: Vec<(String, f64)>,
}

// ---------------------------------------------------------------------------
// DagWebView
// ---------------------------------------------------------------------------

pub struct DagWebView {
    webview: Option<wry::WebView>,
    visible: bool,
    /// Whether the JS side has signalled that the page is rendered.
    ready: bool,
    ipc_rx: mpsc::Receiver<DagIpcMessage>,
    ipc_tx: mpsc::Sender<DagIpcMessage>,
    /// Generation of model data last sent to JS.
    data_generation: u64,
    /// Error from webview creation.
    pub error: Option<String>,
}

impl DagWebView {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            webview: None,
            visible: false,
            ready: false,
            ipc_rx: rx,
            ipc_tx: tx,
            data_generation: 0,
            error: None,
        }
    }

    /// Create the wry WebView as a child view of the eframe window.
    pub fn create(
        &mut self,
        handle: &StoredWindowHandle,
        bounds: wry::Rect,
    ) {
        let tx = self.ipc_tx.clone();
        let result = wry::WebViewBuilder::new()
            .with_html(DAG_HTML)
            .with_bounds(bounds)
            .with_transparent(true)
            .with_background_color((10, 14, 26, 0))
            .with_ipc_handler(move |req| {
                let body = req.body();
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
                    match val["type"].as_str() {
                        Some("click") => {
                            if let Some(code) = val["code"].as_str() {
                                let _ = tx.send(DagIpcMessage::NodeClicked {
                                    code: code.to_owned(),
                                });
                            }
                        }
                        Some("ready") => {
                            let _ = tx.send(DagIpcMessage::Ready);
                        }
                        _ => {}
                    }
                }
            })
            .with_devtools(cfg!(debug_assertions))
            .build_as_child(handle);

        match result {
            Ok(wv) => {
                self.webview = Some(wv);
                self.error = None;
            }
            Err(e) => {
                self.error = Some(format!("Failed to create DAG webview: {e}"));
                eprintln!("dag: {}", self.error.as_ref().unwrap());
            }
        }
    }

    /// Reposition the webview to match the egui panel rect.
    pub fn set_bounds(&mut self, bounds: wry::Rect) {
        if let Some(ref wv) = self.webview {
            let _ = wv.set_bounds(bounds);
        }
    }

    /// Show or hide the native webview. The webview stays hidden until
    /// the JS side signals readiness to avoid a white flash on first load.
    pub fn set_visible(&mut self, visible: bool) {
        let effective = visible && self.ready;
        if self.visible == effective {
            return;
        }
        self.visible = effective;
        if let Some(ref wv) = self.webview {
            let _ = wv.set_visible(effective);
        }
    }

    /// Mark the webview as ready (called when the JS "ready" IPC arrives).
    pub fn mark_ready(&mut self) {
        self.ready = true;
    }

    /// Send the full model data to the JS side for a complete re-render.
    pub fn send_model_data(
        &mut self,
        model: &fiftyone_folds::ModelResponse,
        draft_drivers: &[(String, String, String)], // (code, name, selected_state)
        generation: u64,
    ) {
        if self.data_generation == generation {
            return;
        }
        self.data_generation = generation;

        let payload = DagPayload {
            drivers: draft_drivers
                .iter()
                .map(|(code, name, state)| DagNode {
                    code: code.clone(),
                    name: name.clone(),
                    state: state.clone(),
                })
                .collect(),
            edges: model
                .edges
                .iter()
                .map(|e| DagEdge {
                    parent: e.parent.clone(),
                    child: e.child.clone(),
                })
                .collect(),
            outcomes: model
                .current
                .outcomes
                .iter()
                .map(|o| (o.label.clone(), o.probability.unwrap_or(0.0)))
                .collect(),
        };

        if let Ok(json) = serde_json::to_string(&payload)
            && let Some(ref wv) = self.webview
        {
            let _ = wv.evaluate_script(&format!("window.updateDAG({})", js_string_literal(&json)));
        }
    }

    /// Lightweight state-only update (colour changes, no relayout).
    #[allow(dead_code)]
    pub fn update_driver_states(
        &self,
        draft_drivers: &[(String, String)], // (code, selected_state)
    ) {
        #[derive(Serialize)]
        struct StateUpdate {
            code: String,
            state: String,
        }
        let updates: Vec<StateUpdate> = draft_drivers
            .iter()
            .map(|(code, state)| StateUpdate {
                code: code.clone(),
                state: state.clone(),
            })
            .collect();
        if let Ok(json) = serde_json::to_string(&updates)
            && let Some(ref wv) = self.webview
        {
            let _ = wv.evaluate_script(&format!("window.updateStates({})", js_string_literal(&json)));
        }
    }

    /// Drain the IPC channel for messages from JavaScript.
    pub fn poll_ipc(&self) -> Option<DagIpcMessage> {
        self.ipc_rx.try_recv().ok()
    }

    pub fn is_created(&self) -> bool {
        self.webview.is_some()
    }
}

/// Encode an arbitrary string as a JS string literal safe to embed inside
/// a script passed to `evaluate_script`. Uses `serde_json::to_string` (a
/// JSON string literal is a valid JS string literal) and additionally
/// escapes U+2028 / U+2029, which JSON leaves raw but which terminate
/// string literals in pre-ES2019 engines.
fn js_string_literal(s: &str) -> String {
    let json = serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_owned());
    json.replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029")
}
