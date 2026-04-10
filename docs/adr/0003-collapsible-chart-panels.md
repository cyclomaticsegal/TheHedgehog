# ADR-0003: Collapsible Chart Panels

**Status:** Accepted  
**Date:** 2026-04-02  
**Branch:** `perf-imp`

---

## Context

With three chart panels now possible (VIX, Asset Performance vs VIX, and the Price panel), the vertical content height frequently exceeds what fits on screen, requiring the user to scroll to see all three. Scrolling is not a problem per se, but it breaks the side-by-side mental model: the user cannot see the correlation between a VIX spike and a commodity price move if those two charts are off-screen at the same time.

The goal is to let users reclaim screen space temporarily without losing context or navigating away.

---

## Decision

### Collapsible panels with summary headers

Each chart panel has a clickable full-width header row. Clicking anywhere on the header toggles the panel between expanded (full chart) and collapsed (header only, ~30px). The header always shows:

| Panel | Right-side summary when collapsed |
|---|---|
| VIX Index | `18.45 — Normal` in alert-level colour |
| Asset Performance vs VIX | `4 assets · [P] price view` in gray |
| Gold — Price | `2418.50 · [P] close` in instrument colour |

This means collapsing a chart does not make it invisible — the most important single-line summary remains readable. The user can scan all three headers and know the state of the dashboard at a glance even with two panels collapsed.

### Affordance: chevron + full-width click target

A small chevron (`▶` / `▼`) on the left of each header indicates collapsed/expanded state. The entire header row — not just the chevron — is the click target. Research and convention (VS Code sidebar sections, Bloomberg panel headers, browser developer tools) is clear that small chevron-only click targets are a usability failure on data-dense screens. Full-row click targets are the correct pattern.

A subtle background highlight on hover signals that the header is interactive without adding visual noise when the user is not interacting with it.

### Scroll area auto-adjustment

egui's `ScrollArea` automatically removes the scrollbar when content fits within the viewport. No explicit scrollbar management is required. Collapsing any panel may cause the scrollbar to disappear; expanding any panel may cause it to reappear. This is the expected behaviour.

### What is NOT implemented

**Drag-to-resize between panels** — rejected. Adds significant implementation complexity and creates an easy way to accidentally make a chart too small to use. Not needed when collapse/expand covers the space-reclamation use case.

**Keyboard shortcuts for per-chart collapse** — rejected at this stage. Collapse is a low-frequency action (once per session). Keyboard shortcuts earn their complexity only for frequent actions. The full-row click target is fast enough.

**Persistence of collapse state** — rejected. Collapse state is session-only (resets when the app restarts). Persisting it would require adding fields to `AppSettings` (which is saved to SQLite) and handling schema migration for existing installs. The benefit is minor: users who collapse VIX every session will click it once per session. The cost (schema migration, default handling) is disproportionate.

---

## Implementation

`collapsible_chart_header(ui, id_salt, collapsed, title, right_text, right_color) -> bool`

A standalone function using `ui.allocate_exact_size` inside `ui.push_id` for stable widget IDs. Returns `true` when clicked; callers toggle their `collapsed` flag in response. The painter renders directly onto the allocated rect, giving precise control over text placement and hover highlight z-ordering.

Collapse state is held in three `bool` fields on `DashboardApp` (`vix_collapsed`, `correlation_collapsed`, `price_panel_collapsed`). Since `bool` is `Copy`, they are extracted before `self` is passed (as `&DashboardApp`) to each chart function, preventing borrow conflicts. The pattern mirrors the existing `custom_zoom` extraction.

Each chart function received a `collapsed: &mut bool` parameter. The function renders its header first (data-free, using cached analysis results), applies the early-return if collapsed, then computes windowed data and renders the chart body. This avoids the filtering/normalisation work when collapsed.

---

## Consequences

### Positive
- Three panels fit comfortably on typical laptop screens when two are collapsed.
- Summary info in collapsed headers means the user loses no critical context.
- The `[P] price view` and `[P] close` hints remain visible in collapsed state, preserving keyboard affordance discoverability.
- Scroll area management is automatic; no code needed.

### Neutral
- The title in the collapsible header uses `FontId::proportional(14.0)` (painter-rendered) rather than egui's `RichText::strong()`. In practice this renders identically since egui's default font set does not include a separate bold variant.
- `price_panel_collapsed` persists while the price panel instrument is `None` (panel hidden). This is harmless; the flag is simply ignored until a panel is opened.
