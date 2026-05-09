# ADR 0015: Dark Theme Hardening and 51Folds Model Explorer UI Polish

**Date:** 2026-04-11
**Status:** Accepted
**Extends:** ADR 0014 (model explorer navigation stack)

## Context

ADR 0014 shipped the navigation-stack model explorer with the correct structure but a handful of theming, contrast, typography, and alignment bugs that made several screens unreadable on macOS. Real-world testing surfaced the following problems, each with a distinct root cause:

### 1. Light background in the central panel on macOS Light mode

egui 0.31's `Context::set_visuals(visuals)` only writes to the *current* theme slot:

```rust
pub fn set_visuals(&self, visuals: crate::Visuals) {
    self.style_mut_of(self.theme(), |style| style.visuals = visuals);
}
```

On a Mac running the system in Light mode, `ctx.theme()` returns `Theme::Light`, so our dark palette was stored under the Light slot. When eframe later re-resolved the theme against the system preference, it fell back to egui's default `Visuals::light()` for anything touching the Dark slot — which in our app was the `CentralPanel` frame. The side panels kept their fill because they were rendered first with cached state, but the central area flashed to off-white. Result: white-on-white text in the 51Folds Outcome and Drivers tabs.

### 2. egui_commonmark rendered dark grey text on dark navy

`egui`'s default text colour is read from `Visuals::text_color()`, which returns `visuals.widgets.noninteractive.fg_stroke.color`. The dark preset ships with `Color32::from_gray(140)` — a mid grey that is fine on egui's stock dark background but unreadable on our navy `PANEL_BG (17, 24, 39)`. Our app hadn't overridden this because every `RichText` call in `app.rs` sets `.color()` explicitly. `egui_commonmark::CommonMarkViewer`, however, does not — its `Style::to_richtext` method (in `egui_commonmark_backend::misc`) emits plain `RichText::new(text)` and relies on the ambient `override_text_color`. The Summary Report window therefore rendered body text in egui's default mid-grey.

### 3. `.weak()` text resolved to near-black

`egui_commonmark` renders blockquotes with `self.text_style.quote = true`, which adds `.weak()` to the `RichText`. `Visuals::weak_text_color()` is computed as:

```rust
gray_out(text_color()) = tint_color_towards(text_color, widgets.noninteractive.weak_bg_fill)
```

The dark preset leaves `weak_bg_fill = from_gray(27)` — near black. Blending `TEXT_PRIMARY` towards `gray(27)` produced a dim navy-grey essentially indistinguishable from the background. So even after fixing (2), any report that contained a blockquote would ship an unreadable passage.

### 4. Typography scale was too flat

ADR 0014 documented the target scale as 20 / 15 / 14 / 13 / 11, and the implementation matched it. In use, the hierarchy read as uniform grey: driver names at 15 px sat one step above pill buttons at 13 px, the "Details ›" action was 12 px, and timestamps sank to 11 px — almost invisible. The 3-point ladder failed the "readable at a glance" test. Industry baselines (Material 3, Apple HIG, IBM Carbon, Refactoring UI) put desktop body text at 14 px minimum and favour ~1.25× jumps between levels for skimmability.

### 5. Outcome labels centred inside their columns

The Outcome tab laid each row out as `[label] [bar] [percentage]` using `ui.add_sized` to reserve a fixed-width label column. `add_sized` centres its inner content, so multi-word outcome titles floated towards the middle of the column and each row had a different left edge — visually incoherent next to the perfectly aligned progress bars.

### 6. Sidebar outcome list wrapped with inconsistent indent

The right-hand 51Folds summary rendered each outcome as `"  {label}   {pct}%"` — two leading spaces for visual indent. When `Label::wrap()` broke a long outcome onto a second line, the continuation started at column 0 (no leading spaces), so wrapped lines stuck out to the left of the indented first line. Also rendered at 10 px, which was independently too small.

### 7. Hardcoded `APP_BG` used as text colour

The Summary Report's "Loaded Inferences" list rendered each row's label with `.color(APP_BG)`. `APP_BG = rgb(10, 14, 26)` is the *darkest* colour in the palette — near black. Text was invisible on the dark window background. This was a leftover from an earlier light-theme iteration that was never caught because the feature was rarely opened.

### 8. Unicode glyphs rendering as tofu boxes

The sidebar model summary used `"Model {id} — ✓ complete"`. egui's default font (Ubuntu Light) does not ship the "✓" (U+2713) glyph at the weights we use, so it rendered as a missing-glyph rectangle. The em-dash was fine mechanically but the user reported it read as "m-" on a tight line. Both were removed in favour of prose. A broader sweep of hardcoded label strings found three more em-dashes in rendered UI code.

## Decision

### Theme pinning

Replace the single `set_visuals` call with explicit writes to **both** theme slots plus a pinned preference:

```rust
_cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
_cc.egui_ctx.set_visuals_of(egui::Theme::Dark, visuals.clone());
_cc.egui_ctx.set_visuals_of(egui::Theme::Light, visuals);
```

Either slot now holds our palette, so a system-theme flip cannot strand us on egui's default light visuals.

### Explicit `CentralPanel` frame

Wrap the `CentralPanel` in an explicit `Frame` so its backdrop is pinned regardless of theme resolution:

```rust
let central_frame = egui::Frame::default()
    .fill(PANEL_BG)
    .inner_margin(egui::Margin::symmetric(16, 12));
egui::CentralPanel::default().frame(central_frame).show(ctx, |ui| { ... });
```

### Text-colour hardening

Four concurrent writes to `visuals` so that every resolution path in egui's text pipeline lands on a readable colour:

| Field | New value | What it controls |
|---|---|---|
| `widgets.noninteractive.fg_stroke.color` | `TEXT_PRIMARY` | Default `Visuals::text_color()` — the base for unstyled text and `weak_text_color()` |
| `override_text_color` | `Some(TEXT_PRIMARY)` | Fallback for non-`.strong()` / non-`.weak()` RichText in widgets that don't set `.color()` (including `egui_commonmark`) |
| `widgets.active.fg_stroke.color` | `Color32::WHITE` | Target for `.strong()` text (headings, bold) via `strong_text_color()` |
| `widgets.noninteractive.weak_bg_fill` | `TEXT_SECONDARY` | Fade-out target used by `gray_out` → controls how `.weak()` dims. Was `gray(27)` — now blends towards a mid-light grey so blockquotes stay legible |

I verified from `egui::widget_text::get_text_color` (style.rs:436-446) that an explicit `RichText::color()` still wins over `override_text_color`, so every `.color(Color32::WHITE)` / `.color(ALERT_NORMAL_FG)` / etc. call in the app continues to work unchanged.

### Typography rebalance

Applied a ~1.25× modular scale with clear gaps between levels. Minimum body text: 14 px. Minimum caption text: 12 px. Uppercase "eyebrow" labels at 12 px (standard pattern in Stripe / Linear / GitHub) are used for section headings inside cards — this frees the 13–15 px range for actual content and improves perceived hierarchy without needing a larger top end.

| Element | Before | After |
|---|---|---|
| Page heading (question, section page) | 20 | **22** |
| Driver card name | 15 | **17** |
| Driver detail heading | 20 | **22** |
| Outcome row label / percentage | 15 | **16** bold |
| Body paragraphs (justifications, state descriptions) | 14 | **15** |
| "Related" row labels | 14 | **15** |
| Back button | 13 | **14** |
| "Details ›" button | 12 | **13** |
| Current-state description | 13 | **14** |
| Citations / Sources list | 12 | **13** |
| Timestamps / meta | 11 | **12** + `TEXT_SECONDARY` |
| Section card titles | mixed 13 | **12 UPPERCASE bold** |

### Shared helpers and accent palette

Added two helper functions and two constants to `src/app.rs`:

```rust
const ACCENT_BLUE: Color32 = Color32::from_rgb(96, 165, 250);      // links, chevrons, highlights
const ACCENT_BLUE_DIM: Color32 = Color32::from_rgb(59, 130, 246);  // filled CTAs, pill selections

fn section_card<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R { ... }
fn back_button(ui: &mut egui::Ui, label: &str) -> egui::Response { ... }
```

`section_card` wraps content in a dark `SURFACE` card with a 1 px `BORDER` stroke, 8 px corner radius, and `(18, 16)` symmetric inner margin. Every grouped block in the model explorer (Outcome probabilities, Take away, Current state, All states, Related rows, Sources, driver rows) now uses this, so the look is consistent and one place can tune it.

### Layout fixes

- **Outcome labels**: replaced `ui.add_sized(..., Label)` with `allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), ...)` so labels are flush-left inside their column. Percentages use `Layout::right_to_left(Align::Center)` for symmetric right alignment.
- **Driver state pills**: scoped `ui.spacing_mut().button_padding = Vec2::new(14.0, 8.0)` and added `.min_size(Vec2::new(0.0, 30.0))` so each pill has real interior breathing room. Corner radius bumped 14 → 16 to match the taller pill.
- **Reset / Re-evaluate / CTA buttons**: scoped `button_padding = (16, 9/10)` so they don't read as cramped link-buttons.
- **Section page back button**: uses `"Driver ({code})"` (short, always ≤12 chars) instead of truncating the full driver name with `chars().take(30)`. The full name is already the page's heading one row below.

### Sidebar 51Folds summary rewrite

Bumped the Model ID label to 13 px + bold + green, and the outcome rows to 12 px. Restructured the row layout: the percentage is now on its own line above the label, at the same left edge, so a long outcome label wraps cleanly with no leading-indent artifact. Stripped the `✓` glyph (rendered as tofu in the default font) and the em-dash from the "Model {id} — ✓ complete" label.

### Em-dash sweep on hardcoded labels

Four call sites in `src/app.rs` had hardcoded em-dashes in rendered text:

| Line | Before | After |
|---|---|---|
| ~1304 | `" — {suspect} suspect (>25 min pending)"` | `" ({suspect} suspect, >25 min pending)"` |
| ~1566 | `"  {} — {:.1}%"` (sidebar outcome row) | replaced by two-line layout (see above) |
| ~1601 | `"Model {model_id} — building…"` | `"Model ID: {model_id}   building…"` |
| ~4941 | `"—"` (VIX N/A placeholder) | `"n/a"` |

Long-form prose in `src/help.rs`, `src/knowledge.rs`, and `src/ai.rs` kept its em-dashes — those are grammatically correct in flowing English and the user's complaint was specifically about label strings.

### Inference list text colour fix

The Summary Report's "Loaded Inferences" row label:

```rust
RichText::new(label).size(11.0).color(APP_BG)  // ← near black
```

Changed to:

```rust
RichText::new(label).size(13.0).color(TEXT_PRIMARY)
```

The adjacent status dot was also bumped from 8 px → 10 px and given a 4 px right gap so the row reads as a proper list item.

## Files Changed

| File | Summary |
|---|---|
| `src/app.rs` | Visuals hardening (theme pinning + text-colour quadrant), explicit `CentralPanel` frame, `section_card` / `back_button` helpers, `ACCENT_BLUE` / `ACCENT_BLUE_DIM` constants, full redesign of `render_central_model_view` / `render_central_outcome_tab` / `render_central_drivers_tab` / `render_driver_detail_page` / `render_driver_section_page` / `render_driver_context_section`, sidebar 51Folds summary rewrite, inference list text-colour bugfix, em-dash sweep |
| `src/help.rs` | Added "51Folds Model Explorer" section (navigation stack, Outcome / Drivers / driver detail / section pages, driver state editing and Re-evaluate, sidebar summary, Charts vs 51Folds tabs) — previous help text had no coverage of the 51Folds features added in ADRs 0008, 0013, 0014 |
| `docs/adr/README.md` | Added ADR 0015 row |
| `docs/adr/0015-dark-theme-hardening-and-51folds-ui-polish.md` | This document |

## Consequences

- The 51Folds central-panel tabs match the charts' dark aesthetic, eliminating the jarring flash when switching views.
- Typography hierarchy is visibly stepped: page headings, card headings, body, labels, and captions each read as distinct levels.
- Any future use of `egui_commonmark` (or any other widget that inherits the default text colour) will render readably on our backdrop without needing a per-site override.
- Em-dashes, checkmarks, and other glyphs that egui's default font lacks are no longer used in hardcoded labels — we rely on the font's guaranteed ASCII + Latin-1 coverage for chrome, reserving Unicode for data coming back from 51Folds and LLM responses.
- `section_card` and the accent constants give the next UI iteration one place to turn knobs. A future dark-theme tweak (say, raising BORDER contrast, or switching to a different accent hue) can be a two-line change.
- Help documentation is current through ADR 0014. Future ADRs that touch user-visible features should also update `src/help.rs` in the same pass to prevent the gap re-opening.
