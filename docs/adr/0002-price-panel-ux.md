# ADR-0002: Price Panel — Keyboard-Triggered Drill-Down from Correlation Chart

**Status:** Accepted  
**Date:** 2026-04-02  
**Branch:** `perf-imp`

---

## Context

The "Asset Performance vs VIX" chart normalises all instruments to a percentage change from the window start so they can be compared on a single axis. This is the right design for the correlation question the chart answers, but it sacrifices the raw price: once normalised, you cannot read off "what is Gold actually trading at?" from the chart or its hover tooltip.

A sidebar showing the latest close price for each instrument was considered and is still planned, but it only answers "what is the current price?" — a glance-level question. A user who is spending time with the correlation chart and notices something interesting (e.g. Gold significantly diverging from VIX) may want to investigate the raw price history for five minutes, without losing the comparison context.

The question was how to surface a raw price chart for any single instrument in a way that feels natural and non-disruptive.

### Options considered

**1. Click a line in the correlation chart**  
Rejected. The chart canvas already responds to hover and drag-to-zoom. A click gesture is ambiguous — the system cannot know at mouse-down whether a click or drag is intended. Double-click was considered but conflicts with the drag interaction rhythm and is harder to hit-test accurately on painted lines (no hit-box, only pixels).

**2. Click a legend item**  
Rejected. UX research on financial charting tools (Carbon Design System, Bloomberg, TradingView) is clear: legend clicks are universally expected to toggle series visibility, not trigger navigation. Hijacking that affordance for a different action would create confusion.

**3. Sidebar expansion / separate page**  
Rejected for the interactive investigation use case. Navigating away from the correlation chart destroys the context that prompted the investigation.

**4. Keyboard shortcut → immediate view swap**  
Partially rejected. A single key showing a price chart for a predetermined instrument (e.g. the one nearest the crosshair) is opaque — there is no visible affordance and the instrument selection is implicit and accident-prone.

**5. [Accepted] `P` key → picker dialog → price panel expands below**  
This is the two-step command model used by Bloomberg Terminal (`ticker → function → GO`), VS Code (`Ctrl+P → file`), and Linear (`Cmd+K → action`). Research (Retool, Superhuman, Smashing Magazine modal decision tree) consistently validates two-step keyboard interactions when:
- Step 1 is a search/filter reducing a known list
- Step 2 is Enter (single key)
- The picker is small and non-disruptive
- The transition is fast

---

## Decision

### Interaction model

- **`P` key** (when no text widget has keyboard focus) opens the instrument picker.
- If a price panel is already showing, `P` closes it (toggle).
- The picker is a floating overlay (`egui::Area` at `ORDER::Foreground`, centred on screen).
- It shows all 10 non-VIX instruments immediately on open, no typing required.
- Typing filters the list by name (case-insensitive substring).
- `↑↓` arrows navigate, `Enter` confirms, `Esc` cancels.
- On confirmation, the picker closes and a price chart panel slides in directly below the correlation chart, separated by a rule. The price chart uses the same time window and custom zoom as the other charts, and its crosshair syncs with the VIX and correlation charts.
- `P` closes the panel. The correlation chart is unaffected throughout.

### Discoverability

A quiet `[P] price view` hint sits in the top-right of the correlation chart header. It is always visible but intentionally muted (gray, monospace). The price panel's own header shows a matching `[P] close` hint.

This follows the "teach shortcuts passively" principle identified in the Superhuman and Retool command palette research: users who open the picker repeatedly will learn the shortcut; occasional users are never lost because the hint is always there.

### Why `P`, not `Cmd+K` or `/`

`Cmd+K` is the dominant convention in web apps. This is a native Rust/egui desktop application with no browser shortcut conflicts. `P` (for Price) is mnemonic, single-key, and consistent with Bloomberg's mnemonic function-key model. The character is available in this app's existing keyboard surface (no collision with other shortcuts).

### Key implementation details

- `price_picker_just_opened: bool` flag triggers `TextEdit::request_focus()` on the first frame the picker is rendered, giving it keyboard focus immediately.
- Navigation key events (`ArrowDown`, `ArrowUp`, `Enter`, `Escape`) are consumed via `ctx.input_mut(|i| i.consume_key(...))` before the picker Area is shown, preventing bleed-through to the scroll area.
- The `P` key global handler checks `ctx.memory(|m| m.focused().is_some())` before firing. This ensures the key is suppressed while the picker's text filter (or any other text field in the app) has focus.
- `price_panel_instrument: Option<Instrument>` drives chart rendering; `None` = panel hidden, `Some(x)` = panel showing instrument `x`. Since `Instrument` is `Copy`, it is extracted with a `let` binding before `self` is borrowed by `chart_price_panel`.

---

## Consequences

### Positive
- Users can drill into any instrument's raw price history without leaving the correlation view or losing zoom/time context.
- The interaction is keyboard-first and discoverable — suits both power users and occasional users.
- The price panel is time-synced and crosshair-synced with the existing charts; the three panels compose naturally.
- No chart signatures changed; `chart_price_panel` follows the same `(ui, app, synced_x, ...)` pattern as `chart_vix` and `chart_correlation`.

### Neutral
- The picker shows all 10 non-VIX instruments, not only those currently in the overlay. This is intentional — the price view is independent of the comparison selection — but means the picker list is always the same regardless of what is visible in the correlation chart.
- Only line charts are supported (raw close prices). Candle rendering is deferred; the data sources (FRED, Alpha Vantage, Tiingo) provide close-only or daily-close data, making OHLC candles impractical without a data source change.

### Deferred
- A sidebar "latest prices" panel (quick-glance close prices for all instruments without interaction).
- Mouse-based instrument selection (if the hit-testing complexity is later deemed worthwhile).
- Persistence of the selected price panel instrument across sessions (currently resets on restart).
