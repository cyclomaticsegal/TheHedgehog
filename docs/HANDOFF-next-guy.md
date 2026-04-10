# Handoff Note for the Next Engineer

## Status

The project compiles and runs as a functional monitoring dashboard. Major UX and performance issues from the previous handoff have been resolved (see ADR 0004).

Verification at handoff:

- `cargo fmt`
- `cargo check`
- `cargo build` — clean, no warnings

## What the user actually wants

The user wants a Rust desktop dashboard for comparing **VIX spikes/extremes** against commodities and selected risk assets using **real data**.

The main analytical goal is:

- detect when VIX enters or approaches extreme regimes
- visually compare what happens to commodities at the same time
- judge the **shape and duration** of VIX spikes
- flexibly compare:
  - VIX vs one asset
  - VIX vs a small subset like gold + silver
  - VIX vs a group composite such as energy or metals

The user explicitly cares about:

- real data over dummy data
- visual clarity on short windows like `1M`, `3M`, `6M`
- clear overlays and correlation reading

## Resolved in ADR 0004 session

The following issues from the previous handoff have been addressed:

- **UI responsiveness** — analysis caching eliminates per-frame O(n²) computation; interactions are now instant
- **Charting interactivity** — hover crosshair with interpolated values and date readout on both charts
- **Y-axis clarity** — comparison chart now shows percentage change (`+20%`, `-10%`) instead of raw base-100 values
- **Refresh visibility** — Bloomberg-style activity log shows per-instrument progress in real time
- **Rate limiting** — Tiingo provider added (1,000 req/day free) as alternative to Alpha Vantage (25 req/day)
- **API key management** — keys loaded from `.env` on startup, gitignored
- **Visual consistency** — solid filled squares for commodity selectors (dimmed when off), solid circles for spike indicators, instrument colors in activity log
- **Spike interaction** — clicking a spike episode highlights its date range on the VIX chart

## Remaining user concerns

These may still need attention:

## Known functional/UX problems

### 1. The current charting approach is too primitive

The app draws its own charts in [`src/app.rs`](src/app.rs) using custom painter logic.

This has several consequences:

- labels overlap
- legends are crude
- axis labeling is weak
- the rendering does not provide strong interaction affordances
- overlay readability degrades quickly as more lines are added

This is the main reason the UI feels clunky.

### 2. The overlay model is technically present but not ergonomically good

There is now selector-driven overlay state in:

- [`src/models.rs`](src/models.rs)
- [`src/app.rs`](src/app.rs)

But the control surface is still not good enough:

- too much state is exposed without strong visual feedback
- it is not obvious what lines are currently active
- the relationship between selected group, selected instrument, and dynamic overlay is confusing
- the app still does not feel like a clean “correlation workbench”

### 3. Real-data-first behavior was partially fixed, but this area needs review

The app now tries to avoid silent example-data fallback and tries to clear sample-only caches on startup.

Relevant files:

- [`src/app.rs`](src/app.rs)
- [`src/storage.rs`](src/storage.rs)

This needs to be re-verified end to end with a fresh database and real API keys.

### 4. The app currently compiles, but the product logic is ahead of the UX

Architecturally the code has moved toward:

- grouped instruments
- group composites
- macro composite
- selector-driven overlays

But the user is telling us clearly that the current presentation is worse than the simpler earlier version.

That should be taken seriously. The product has become more general while becoming less legible.

## Likely root cause of the user frustration

The app is trying to do too many things in one screen with a charting layer that is not strong enough.

The result is:

- too many charts
- too many controls
- too many overlays
- not enough hierarchy
- not enough emphasis on the core use case:
  - VIX spike
  - what happened to gold/silver/Bitcoin/selected commodities during the same interval

## Recommended next steps

### 1. Simplify the UI around the primary workflow

Start with the user’s actual workflow, not the generic grouped model.

A better layout is probably:

- left sidebar:
  - data/source status
  - VIX threshold settings
  - overlay selection
- main top:
  - large primary chart for VIX
  - visible spike bands / threshold shading
- main middle:
  - user-selected comparison overlay chart against VIX
- main lower:
  - optional detail cards / group summaries / selected instrument raw charts

The current multi-panel grouped layout is likely too busy.

### 2. Rebuild the chart UX

Do not keep polishing the current custom chart painter unless absolutely necessary.

At minimum, improve:

- legend placement
- axis labels
- line visibility
- series toggling feedback
- spacing and typography

The current chart rendering lives entirely in [`src/app.rs`](src/app.rs).

### 3. Make overlay selection obviously reactive

When a user toggles lines on or off, the app should give immediate clear feedback:

- clear checked state
- visible active-series summary
- less ambiguity about whether a group composite is currently included

The current overlay selector exists, but the user still found it confusing.

### 4. Preserve and prioritize the “core correlation” use case

The earlier app made it easier to see:

- VIX spike
- silver selloff
- Bitcoin selloff
- gold strength

That relationship should have a dedicated first-class view.

Do not hide that behind grouping abstractions.

### 5. Re-verify real provider behavior

The app currently depends on:

- FRED
- Alpha Vantage

Relevant provider code:

- [`src/providers.rs`](src/providers.rs)

This needs a proper live-data verification pass with real keys and a clean cache.

## Important files

- App UI and custom chart rendering:
  - [`src/app.rs`](src/app.rs)
- Instrument/group model:
  - [`src/models.rs`](src/models.rs)
- Analysis helpers and spike episodes:
  - [`src/analysis.rs`](src/analysis.rs)
- Providers:
  - [`src/providers.rs`](src/providers.rs)
- Storage and cache behavior:
  - [`src/storage.rs`](src/storage.rs)
- Example data generator:
  - [`src/sample_data.rs`](src/sample_data.rs)

## ADR / docs trail

- Original free-data ADR:
  - [`docs/adr/0001-pure-rust-egui-free-daily-data.md`](docs/adr/0001-pure-rust-egui-free-daily-data.md)
- Grouped macro monitor ADR:
  - [`docs/adr/0002-grouped-macro-monitor.md`](docs/adr/0002-grouped-macro-monitor.md)
- Real-data-first and dynamic overlays ADR:
  - [`docs/adr/0003-real-data-first-and-dynamic-overlays.md`](docs/adr/0003-real-data-first-and-dynamic-overlays.md)
- Performance, interactivity, and multi-provider ADR:
  - [`docs/adr/0004-performance-caching-interactive-charts-and-multi-provider.md`](docs/adr/0004-performance-caching-interactive-charts-and-multi-provider.md)
- Rolling decision log:
  - [`docs/research/rolling-decision-log.md`](docs/research/rolling-decision-log.md)

## Bottom line

The codebase is no longer blocked at the architecture level.

The blocker is now product quality:

- make real data trustworthy
- make the visual comparison obvious
- reduce confusion
- optimize around the user’s actual correlation workflow, especially on short time windows
