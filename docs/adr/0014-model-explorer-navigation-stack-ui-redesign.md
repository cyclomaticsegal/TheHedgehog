# ADR 0014: Model Explorer Navigation Stack UI Redesign

**Date:** 2026-04-10
**Status:** Accepted
**Supersedes:** The flat tabbed model explorer from ADR 0013

## Context

ADR 0013 moved the 51Folds model results from the AI side panel into a tabbed central panel with Outcome and Drivers sub-tabs. While this gave the content more space than the 360px sidebar, the implementation had fundamental UX problems:

1. **The driver list tried to show everything at once.** Each driver was a card containing the name, a code badge, 5 state buttons, AND an inline-expanding detail section. When expanded, hundreds of words of justification, citations, importance, shifts, and monitoring content dumped into the card. The result was a wall of dense, uneven cards with no visual rhythm.

2. **Typography was too small.** Driver names at 10-12px, state buttons at 11px, detail content at 10-11px. The 51Folds native UI uses 14-16px for equivalent elements. The text was hard to read and uninviting.

3. **No visual hierarchy.** Cards varied in height based on name length and state count. Expanded cards were dramatically taller than collapsed ones, destroying the list's scanability. The 51Folds native UI maintains uniform row height in its driver list.

4. **Colour coding was poor.** TEXT_SECONDARY (rgb 148,163,184) for detail content against the SURFACE (rgb 26,34,54) background lacked contrast. Section headings blended into body text.

5. **Wrong interaction model.** The accordion/expand pattern forces the user to manage open/closed state across 15 drivers, scrolling past walls of text to find the next driver. The 51Folds native UI uses **navigation** instead: clicking a driver takes you to a dedicated detail page, and each detail section (Why selected? / Why does this matter? / What could shift? / What should we monitor?) is a separate navigable page.

Study of the 51Folds native platform UI (13 reference screenshots in `learning-images/`) confirmed the navigation-stack approach as the clear design direction.

## Decision

### Replace flat tabs with a navigation stack

The `ModelTab` enum (Outcome / Drivers) was replaced with a `ModelView` navigation stack:

```rust
enum ModelView {
    Outcome,                                    // Level 0
    DriverList,                                 // Level 0
    DriverDetail(usize),                        // Level 1 — one driver
    DriverSection(usize, DriverDetailSection),  // Level 2 — one content section
}

enum DriverDetailSection {
    WhySelected,
    WhyMatters,
    WhatShift,
    WhatMonitor,
}
```

Each variant renders as a full page in the central panel. Back buttons navigate up the stack. The toolbar still shows Outcome / Drivers as top-level selectors, but clicking a driver row's chevron navigates deeper.

### Outcome view redesign

Matches the 51Folds native Outcome tab (reference image 1):

- Question as 20px bold white heading
- Timestamps in 11px muted text
- Outcome rows: label left (15px white), coral/red proportional bar (right-aligned fill), percentage right (15px white)
- Before/after delta annotations in 12px green/red
- Take Away section: 18px heading, 14px body prose
- "Show me the drivers" prompt and button at bottom (matching the native "Would you like to fine-tune the drivers?" pattern)

### Driver list redesign

Matches the 51Folds native Drivers tab (reference images 2, 10):

- Each driver is a clean, consistent row: name + code (14px white), pill-style state selectors (13px, rounded, selected state filled), blue chevron `>` for navigation
- Rows separated by thin dividers with consistent spacing
- No inline expansion — clicking the chevron navigates to the detail page
- Re-evaluate button (coral fill when active) and Reset button at bottom

### Driver detail page (new)

Matches the 51Folds native driver detail screen (reference image 5):

- Back button `< Drivers` in blue
- Driver name as 20px bold heading
- Current state and its description prominently displayed at top
- All state descriptions listed with the current one highlighted
- "Related:" section with 4 navigable rows, each with a label and chevron:
  - Why was [state] selected?
  - Why does this matter?
  - What could shift?
  - What should we monitor?

### Section content pages (new)

Matches the 51Folds native content screens (reference images 6, 7, 8, 9):

- Back button `< [Driver Name]` in blue
- Section title as 20px bold heading
- Full prose content at 14px with paragraph spacing
- Bold sub-headings rendered from markdown-style `**heading**` markers
- Citations listed at bottom under a "Sources" separator
- Each section gets the full central panel width and scroll — no cramming

### Typography scale

| Element | Size | Weight | Colour |
|---------|------|--------|--------|
| Page headings | 20px | Bold | White |
| Section headings | 16-18px | Bold | White |
| Body text | 14px | Normal | TEXT_PRIMARY |
| Row labels | 14-15px | Bold | White |
| State selectors | 13px | Normal | White (selected) / TEXT_MUTED (unselected) |
| Timestamps/badges | 11-12px | Normal | TEXT_MUTED |

Nothing below 12px. The previous implementation had elements at 9-10px.

### Spacing

- 16-24px between major sections
- 12px between items within a section
- 8px between separators and content
- 16px after back buttons

The previous implementation used 3-6px gaps throughout.

## Files Changed

| File | Summary |
|---|---|
| `src/app.rs` | Replaced `ModelTab` with `ModelView` + `DriverDetailSection` enums. Rewrote `render_central_outcome_tab`, `render_central_drivers_tab`. Added `render_driver_detail_page`, `render_driver_section_page`, `render_driver_context_section`. Updated toolbar, auto-switch, and all `model_tab` references. |

## Consequences

- The model explorer now follows the same navigation-stack pattern as the 51Folds native platform — one thing at a time, with room to breathe.
- Driver detail content that was previously crammed into inline accordions now has full-page space with readable typography.
- The driver list is scannable — uniform rows, consistent height, no surprise expansions.
- Typography is 40-60% larger across the board, bringing it in line with the native platform.
- The `DraftDriverState.expanded` field is now unused (drivers don't inline-expand). It remains in the struct but could be removed in a future cleanup.
- The old `render_folds_model_results` method in the side panel (the original dense rendering) is still present but marked `#[allow(dead_code)]` — it can be removed once the redesign is confirmed.
