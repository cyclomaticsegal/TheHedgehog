# ADR-0004: Design System — Colour Palette and Global Dark Theme

**Status:** Accepted  
**Date:** 2026-04-02  
**Branch:** `perf-imp`

---

## Context

After the collapsible chart panels and price panel were implemented (ADR-0002, ADR-0003), a visual review of the running application revealed three problems:

1. **Broken chevron glyphs** — The Unicode characters `▶` (U+25B6) and `▼` (U+25BC) rendered as `□` (replacement boxes) on the test system. egui bundles a subset of the Noto Sans font; geometric shape blocks are not included. This was visually confusing — the affordance for collapse was broken.

2. **White panel backgrounds** — egui's default `Visuals::light()` was in effect, producing white/light-grey panel backgrounds. The chart rendering painted dark backgrounds directly, creating jarring contrast: bright sidebar and header areas surrounding dark chart canvases.

3. **Incoherent colour palette** — Dozens of ad-hoc `Color32::from_rgb(...)` literals were scattered across the file with no shared semantic meaning. Alert-level colours (Normal / Approaching / Extreme) had three independent definitions. Hover states, backgrounds, borders, and text all used arbitrary grey values with no relationship to each other.

---

## Decision

### 1. Drawn chevrons — no Unicode glyphs

The Unicode chevron characters are replaced with painter-drawn line segments. Two `painter.line_segment()` calls draw a `>` (right-pointing, collapsed) or `∨` (down-pointing, expanded) using the established direction psychology:

- **Right-pointing (`>`)** means "there is content to the right / hidden behind this row" — universally used for collapsed tree nodes (VS Code, macOS Finder, file explorers).
- **Down-pointing (`∨`)** means "content is below / currently shown" — universally used for expanded tree nodes.

This approach is robust to font availability and gives precise control over stroke weight and colour.

### 2. Global dark theme via `set_visuals`

`DashboardApp::new()` calls `_cc.egui_ctx.set_visuals(...)` with a customised `egui::Visuals::dark()` baseline. The key overrides are:

| Property | Value |
|---|---|
| `panel_fill` | `PANEL_BG` (#111827) |
| `window_fill` | `PANEL_BG` (#111827) |
| `extreme_bg_color` | `APP_BG` (#0A0E1A) |
| `faint_bg_color` | `SURFACE` (#1A2236) |
| `widgets.*.bg_fill` | `SURFACE` / `SURFACE_HOVER` |
| `widgets.*.bg_stroke` | `BORDER` |

This eliminates the light/dark mismatch: egui-rendered widgets (sidebar panels, buttons, separators, text inputs) now use the same dark palette as the custom-painted chart areas.

### 3. Semantic colour constants

A block of `const Color32` values is defined at the top of `app.rs`, before any structs. Constants are grouped semantically:

**Backgrounds (darkest → lightest)**

| Name | Hex | Use |
|---|---|---|
| `APP_BG` | #0A0E1A | Chart canvas fill; extreme background |
| `PANEL_BG` | #111827 | egui panel fill; price picker frame |
| `SURFACE` | #1A2236 | Collapsible headers (resting); widget bg |
| `SURFACE_HOVER` | #222D42 | Hover state; picker row cursor |

**Borders**

| Name | Hex | Use |
|---|---|---|
| `BORDER` | #2D3748 | Chart outline; widget stroke; grid lines |

**Text**

| Name | Hex | Use |
|---|---|---|
| `TEXT_PRIMARY` | #E2E8F0 | Titles, active values |
| `TEXT_SECONDARY` | #94A3B8 | Secondary info, date labels, thresholds |
| `TEXT_MUTED` | #4A5568 | Hints, grid labels, chevron icons |

**Alert levels (foreground / background)**

| Name | Hex | Use |
|---|---|---|
| `ALERT_NORMAL_FG` | #38A169 | Normal-state text, icons, threshold line |
| `ALERT_APPROACHING_FG` | #D69E2E | Approaching-state text, icons, threshold |
| `ALERT_EXTREME_FG` | #E53E3E | Extreme-state text, icons, threshold |
| `ALERT_NORMAL_BG` | #143424 | Status banner Normal background |
| `ALERT_APPROACHING_BG` | #4E3A0C | Status banner Approaching background |
| `ALERT_EXTREME_BG` | #4E1414 | Status banner Extreme background |

### Collapsible header background

Headers now paint `SURFACE` as a permanent background (not only on hover), with `SURFACE_HOVER` on pointer entry. This creates a consistent visual separation between the header row and the chart body beneath it, and fixes the "white bar" appearance under the light theme.

### What is NOT addressed

**Typography** — font sizes and families are unchanged. A dedicated typography pass (heading scale, monospace vs proportional consistency) is deferred.

**Icon set** — the Unicode check `✓` and cross `✗` in the activity log remain as-is. These are in the Basic Multilingual Plane and present in egui's embedded font. Only the geometric-block characters (`▶`, `▼`) were missing.

**Per-instrument hue system** — instrument colours (`instrument_color` function) are unchanged. They were already intentionally distinct and visually effective; no regression was observed.

---

## Implementation

All changes are in `src/app.rs`:

- 22 named `const Color32` values at module top.
- `DashboardApp::new()`: 10-line `set_visuals` block immediately before `let db_path`.
- `collapsible_chart_header`: permanent `SURFACE`/`SURFACE_HOVER` background; Unicode chevrons replaced with two `painter.line_segment()` calls each.
- All `Color32::from_rgb(...)` / `Color32::from_gray(...)` literals for semantic colours replaced with the named constants. Chart-internal ad-hoc rgba values (e.g. transparent zone fills) are updated to use the palette's RGB components at the same alpha.

---

## Consequences

### Positive
- The UI is visually coherent end-to-end; no more light/dark clash.
- Alert colours (Normal / Approaching / Extreme) are defined exactly once and used consistently in the status banner, sidebar, chart threshold bands, chart threshold labels, spike episode list, and collapsible header summaries.
- Chevrons render correctly on all systems regardless of font availability.
- Future contributors have a named vocabulary for colours; adding a new widget does not require inventing a new arbitrary grey value.

### Neutral
- `APP_BG` and `SURFACE` are also referenced inline (as `Color32::from_rgb(...)` with the same values) in two places where a const would have required restructuring a closure — these literals remain but are byte-identical to the constants.
- The dark theme is applied globally and unconditionally. A light-mode preference is not supported and is not planned.
