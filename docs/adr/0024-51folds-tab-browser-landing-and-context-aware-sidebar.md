# ADR 0024: 51Folds Tab as Model Registry — Browser Landing, Context-Aware Sidebar

**Date:** 2026-05-14
**Status:** Accepted
**Extends:** ADR 0008 (51Folds integration), ADR 0013 (SDK integration, model explorer), ADR 0014 (model explorer navigation stack), ADR 0020 (concurrent builds, foreground/backlog)

## Context

The README opens with the app's stated *raison d'être*: "causal, probabilistic modelling of capital-markets regimes." Translated: **51Folds is the unifying artifact**; Charts and the embedded Research Agent are on-ramps that produce hypotheses feeding into it. The UI did not say any of this.

Three concrete misalignments:

1. **Default landing was Charts** (`src/app.rs:606`). A new user opening the app for the first time saw a VIX comparison chart, not a model registry. The app announced itself as "a volatility dashboard." Misleading.
2. **The 51Folds tab only rendered the *currently loaded* model.** There was no first-class list-of-models surface anywhere. Models could only be reached implicitly: through the Reports tab's inference history (clicking an inference auto-loaded its linked model), or transiently inside the build-tray popover (ADR 0020), which only showed in-flight builds. Users with five completed models had no way to *see* the five of them as a set.
3. **The left sidebar was static.** VIX Status, Overlay on VIX, and Recent Spikes — all chart-specific widgets — rendered on every tab, including Research, Reports, and 51Folds. Three out of four tabs, the sidebar lied.

A frank conversation with the user crystallized the fix: orient the app around the artifact it claims to be about. The 51Folds tab becomes the model browser (list → detail). The browser is the default landing surface. The sidebar's chart block is conditioned on the Charts tab.

A constraint shaped the design: **viewing must be non-destructive to foreground builds.** Browsing a completed model while another model is being built in the foreground (ADR 0020 tray chip showing `◌ 1 building`) must not interrupt the in-flight build's polling channels, must not swap it to backlog, must not visually replace the tray chip.

## Decisions

### 1. New `ModelView::Browse` variant, default

```rs
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum ModelView {
    #[default]
    Browse,
    Outcome,
    DriverList,
    VisualMap,
    DriverDetail(usize),
    DriverSection(usize, DriverDetailSection),
}
```

`Browse` is the empty/landing state of the 51Folds tab. The detail variants (`Outcome`, `DriverList`, `VisualMap`, plus the two driver-page variants) are reachable only by clicking a row in the browser or by loading an inference whose linked model auto-navigates to `Outcome` (ADR 0019 path, preserved).

### 2. Default landing flipped to `CentralView::Model` + `ModelView::Browse`

```rs
// src/app.rs constructor — was:
central_view: CentralView::Charts,
model_view: ModelView::Outcome,
// now:
central_view: CentralView::Model,
model_view: ModelView::Browse,
```

A new user's first frame is "this app builds Bayesian models, with two on-ramps." Returning users see their model history immediately. Charts is one click away (top-bar pill); nothing is hidden, only re-prioritized. Last-used-tab persistence was deliberately *not* added — even returning users see the registry first, which is the point.

### 3. Browser rendering as a stateless module returning `BrowserAction`

New file `src/app/model_browser.rs`. The browser is pure UI: it takes `&[FoldsModelRecord]` and `Option<&str>` (foreground model_id for the pin indicator), and returns a `BrowserAction` enum. Mutations happen in the caller.

```rs
#[must_use]
pub(super) enum BrowserAction {
    None,
    OpenModel(String),
    StartAnalyze,    // empty-state CTA → Charts
    OpenResearch,    // empty-state CTA → Dexter
}
```

This mirrors the `sidebar.rs` pattern (no `DashboardApp` coupling). The caller `handle_browser_action()` is the single funnel for all browser side effects. Future actions (Delete, Fork, Export, Compare) extend the enum.

Empty-state copy includes two CTAs that switch tabs (Analyze VIX → Charts, Open Research Agent → ResearchAgent). The on-ramps are explicit, not implicit.

### 4. Non-destructive viewing via swap-in-place

The pivotal design call. Two options were available:

**(A) Parameterize every detail renderer to read from `&FoldsTask`** instead of `self.folds_task`. Clean architecturally; ~dozens of read sites across `render_central_outcome_tab`, `render_central_drivers_tab`, `render_dag_view`, `render_driver_detail_page`, `render_driver_section_page`. Many of those reads are mutable (driver pill state, etc.) — non-trivial borrow-checker dance.

**(B) Swap `viewed_task` into `folds_task` for the duration of the render, swap back at exit.** Lets the existing detail code run unmodified. A new `in_viewed_render` flag tells foreground-mutating affordances (Refresh, Re-evaluate) to disable themselves so they cannot accidentally act on the viewed model.

(B) was chosen. The mechanism:

```rs
fn render_central_model_view(&mut self, ui: &mut egui::Ui) {
    if self.model_view == ModelView::Browse {
        self.refresh_model_list_if_stale();
        let action = model_browser::render_model_browser(
            ui, &self.cached_model_list,
            self.folds_task.model_id.as_deref(),
        );
        self.handle_browser_action(action);
        return;
    }

    let viewing = self.viewed_task.is_some();
    if viewing {
        let viewed = self.viewed_task.take().expect("checked Some above");
        let real_foreground = std::mem::replace(&mut self.folds_task, viewed);
        self.viewed_task = Some(real_foreground);
        self.in_viewed_render = true;
    }

    self.render_central_model_view_inner(ui);

    if viewing {
        let original_foreground = self.viewed_task.take()
            .expect("swap above guarantees Some");
        let mutated_view = std::mem::replace(&mut self.folds_task, original_foreground);
        self.viewed_task = Some(mutated_view);
        self.in_viewed_render = false;
    }
}
```

Local pill edits (`draft_drivers` mutations) land back in `viewed_task` because they were applied while it was swapped into `folds_task`. The foreground's in-flight channels (`rx`, `refresh_rx`) are preserved across the swap — they are never read inside the detail renderers, only by `update()` *before* the render call, so the swap window is safe.

Foreground-mutating affordances gated on `!self.in_viewed_render`:
- Refresh-from-51Folds button (`src/app.rs:~4263`) — replaced with a "Browsing — not the foreground build" label.
- Re-evaluate button (`src/app.rs:~4780`) — disabled (Reset and Revert stay enabled because they are purely local pill rollbacks).

### 5. Two new storage queries

```rs
pub fn load_all_folds_models(&self) -> Result<Vec<FoldsModelRecord>>;
pub fn load_folds_response_by_model_id(&self, model_id: &str) -> Result<Option<String>>;
```

Siblings of the existing `load_pending_folds_models` and `load_folds_response_for_inference`. The first drops the `WHERE status = 'pending'` clause and orders DESC; the second queries by `model_id` rather than `inference_id`. No schema changes — the `folds_models` table already has every column the browser needs (`status`, `created_at`, `question`, `model_id`).

### 6. Cached model list with a 30-second TTL

`cached_model_list: Vec<FoldsModelRecord>` + `model_list_loaded_at: Option<Instant>` on `DashboardApp`. The browser calls `refresh_model_list_if_stale()` on entry; the TTL amortizes DB hits during scroll/render. Stale data within a 30s window is acceptable — a model completing mid-window will appear at the next entry. (No explicit invalidation hook on build completion yet; a future change should add one if 30s feels laggy in practice.)

### 7. Sidebar wrapped on `central_view == CentralView::Charts`

```rs
if self.central_view == CentralView::Charts {
    if let Some(status) = &self.cached_vix_status {
        sidebar_vix_summary(ui, status);
        ui.separator();
    }
    sidebar_overlay_controls(ui, &mut self.settings);
    ui.separator();
    sidebar_spike_episodes(ui, &self.cached_spike_episodes, &mut self.highlighted_spike);
    ui.separator();
}
// Config block (Data Source / AI / Research / Thresholds / 51Folds key)
// always renders below — it is truly cross-cutting.
```

The three chart-context sections appear only on Charts. The config block stays. The sidebar stops lying.

### 8. Top-bar sub-pill restructure

A "Models" pill is always present when the 51Folds tab is active, and acts as back-to-list. Outcome / Drivers / Visual Map sub-pills appear only when the user is in detail mode (`model_view != Browse`) AND there is a model available to view (`viewed_task.is_some() || folds_task.model.is_some()`). Clicking Models from detail clears `viewed_task` and navigates to Browse — foreground state is preserved.

### 9. Additive, not replacing

`load_historical_inference` (the Reports → inference → linked-model path) was deliberately left untouched. It still swaps the inference's model into `folds_task` foreground and auto-navigates to `ModelView::Outcome`. The browser flow is **additive**: the two paths coexist. Critically, this preserves the build-recovery behavior — loading a pending inference still re-attaches its in-flight build to the foreground.

## Consequences

- The app's identity matches its stated raison d'être the moment it opens. Cold-launch UX is "browse / build 51Folds models" rather than "view a VIX chart." Returning users see their model history first; new users see a two-CTA empty state pointing at the on-ramps.

- The swap-in-place mechanism is a load-bearing assumption: **no code inside the detail renderers reads polling channels** (`rx`, `refresh_rx`). All polling happens in `update()` before render. If a future renderer starts polling channels during render, the swap window would silently break. The compile time invariant is weak; the runtime contract is implicit. A test or comment-pin near `render_central_model_view` is a worthwhile follow-up.

- `in_viewed_render` is now a load-bearing flag for any foreground-mutating action. Two sites use it (Refresh, Re-evaluate). Future affordances that mutate the server (Delete, Archive, Fork, Export-with-side-effect) must check it. Rust does not enforce this; reviewers do.

- The browser is stateless w.r.t. side effects via the `BrowserAction` return enum. Extending the browser with new row actions is mechanical: add a variant, handle it in `handle_browser_action`. Mutating the model list (e.g. delete) requires also bumping `model_list_loaded_at` to `None` so the next render reloads.

- Sidebar widgets that are tab-specific should declare themselves explicitly via the same conditional pattern. If a future widget straddles Charts and another tab, it needs its own predicate — not a re-loosening of the chart block. The principle: the sidebar should never lie.

- Default-tab persistence was *not* added. This is intentional — the identity goal requires every launch to land on the model registry. If users complain, the right escape hatch is a "remember last tab" *setting*, not a behavior change.

- The browser today renders one flat list ordered by `created_at DESC`. Filtering, search, status grouping, deletion, and per-model drift metrics are deferred. The list virtualizes (egui `ScrollArea::show_rows`), so the implementation scales to thousands of rows; the UX of finding a specific model among many is a separate problem.

- The 30s cache TTL is the cheapest correct value, not the best one. A build completing mid-window appears at the next entry — acceptable, but if a future build-completion hook fires `self.model_list_loaded_at = None`, the cache becomes precisely fresh at the moments that matter. Worth wiring when the completion-event plumbing is next touched.

- `viewed_task` shares the full `FoldsTask` shape with `folds_task`, which carries many fields irrelevant to viewing (`rx`, `refresh_rx`, `in_flight`, `error`, `refresh_found_failed_id`). They sit unused in viewed_task. Splitting into a lighter `ViewedModel` struct was considered and rejected: the shared shape is what lets the swap mechanism work without parameterizing renderers. The dead fields are cheap and the simplicity dominates.

- `ModelView::Browse | ModelView::VisualMap => {}` in the inner-renderer match is unreachable in practice (both are handled in the outer dispatcher), but is kept explicit so future variants don't silently fall through. The match is exhaustive, not defensive.
