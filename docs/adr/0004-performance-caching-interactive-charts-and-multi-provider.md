# ADR 0004: Performance Caching, Interactive Charts, and Multi-Provider Architecture

## Status

Accepted on 2026-03-31

## Context

The handoff from the previous session identified several blockers:

- **the app was unresponsive** — button presses, checkbox toggles, and sidebar drags were sluggish, which is unusual for a native Rust application
- **the charting layer lacked interactivity** — no hover inspection, no crosshair, no value readouts
- **y-axis labels on the normalized comparison chart were unintelligible** — raw base-100 values without context
- **the UI lacked visual feedback during data refresh** — a single bulk status message appeared only after all 11 instruments had finished fetching
- **Alpha Vantage free-tier rate limits (25 requests/day, 1 request/second)** made the app fragile during refresh and constrained daily usage to 2–3 refreshes
- **API keys were hardcoded in the UI** — no `.env` support, no persistence across launches without manual Save
- **legend and indicator affordances were inconsistent** — text glyphs rendered as outlines in some fonts, unselected instruments showed as black rather than dimmed color, spike indicators used the same visual language as commodity selectors

## Decision

We address these as a single coordinated set of changes across five areas.

### 1. Analysis caching

The root cause of UI sluggishness was that egui is an immediate-mode GUI — `update()` runs every frame (~60fps). Every frame was recomputing:

- `compute_vix_status()` three times (sidebar, banner, chart), each sorting a 252-element window
- `recent_spike_episodes()` once, but at O(n²) cost: for each of ~600 observations, a window was cloned, sorted, and percentile-interpolated

We add a cache layer to `DashboardApp`. Expensive analysis results (`cached_vix_status`, `cached_spike_episodes`) are recomputed only when `data_generation` changes (data reload) or `threshold_config` changes (user adjusts settings). All UI code reads from cache.

### 2. Incremental refresh with activity log

We replace the bulk `RefreshOutput` return type with a streaming `RefreshEvent` enum sent per-instrument through the channel:

- `Fetching(Instrument)` — before the HTTP call
- `Fetched(ObservationBatch)` — batch saved to storage immediately on receipt
- `FetchFailed { instrument, error }` — error recorded inline
- `Done` — triggers final reload and alert evaluation

A bottom panel displays real-time progress: timestamp, instrument color swatch, status icon (spinner/checkmark/cross), instrument name, and point count or error message.

### 3. Interactive charts

We add hover crosshair interaction to `paint_chart`:

- vertical crosshair line at cursor position
- date label at crosshair base
- interpolated value readout for each visible series at the hover x-coordinate
- colored dots on each line at the intersection point

The normalized comparison chart y-axis switches from raw base-100 values to percentage-change labels (`+20%`, `-10%`, `0%`).

Spike episodes in the sidebar become clickable: clicking a spike draws a translucent highlight band over that date range on the VIX chart. Clicking again deselects.

### 4. Multi-provider architecture with Tiingo

We add Tiingo as a third data provider alongside FRED and Alpha Vantage. The user selects between Alpha Vantage and Tiingo in the sidebar; FRED always provides VIX.

Tiingo maps commodities to liquid ETFs:

| Instrument | ETF |
|-----------|-----|
| Gold | GLD |
| Silver | SLV |
| Bitcoin | IBIT |
| Crude Oil | USO |
| Natural Gas | UNG |
| Copper | CPER |
| Aluminum | DBB |
| Wheat | WEAT |
| Corn | CORN |
| Soybeans | SOYB |

Tiingo's free tier provides 1,000 requests/day with no per-second throttle, effectively removing the rate-limit constraint.

The provider spec system is restructured from a static array to `build_specs(provider)` which returns the appropriate set based on the user's selection. Alpha Vantage calls retain the 1.5-second inter-request throttle.

### 5. Environment-based API key loading

API keys are loaded from `.env` via `dotenvy` on startup, with saved settings taking precedence. The `.env` file is gitignored.

## Why

### Performance

- egui repaints on every interaction event; the O(n²) spike analysis was the dominant cost per frame
- caching reduces per-frame cost from hundreds of milliseconds to effectively zero for frames where data and settings are unchanged
- even during slider drags (which mutate threshold settings every frame), the recomputation now happens once per frame instead of four times

### Incremental refresh

- the user had no visibility into which instruments were loading, which had failed, or why
- bulk refresh masked individual failures behind a wall of concatenated error text
- per-instrument progress matches the user's mental model of "fetching Gold, then Silver, then..."

### Chart interactivity

- the handoff document explicitly flagged "no strong interaction affordances" as a charting weakness
- hover-to-inspect is the minimum viable interaction for a financial chart
- percentage-change labels on the y-axis are immediately meaningful; raw normalized values are not

### Multi-provider

- Alpha Vantage's 25-request/day free limit made the app impractical for iterative development and normal use
- Tiingo's 1,000-request/day free tier removes this constraint entirely
- ETF prices are perfectly adequate for normalized regime-shift comparison — the base-100 normalization eliminates absolute price differences

## Consequences

### Positive

- the app is now responsive on all interactions
- the user can see exactly what is happening during refresh
- charts provide value readouts on hover
- the comparison chart's y-axis is immediately legible
- the app can refresh ~90 times/day on Tiingo vs ~2 on Alpha Vantage
- spike episodes are interactive — clicking navigates to the relevant chart region
- visual language is now consistent: solid filled squares for commodities, solid filled circles for spikes

### Negative

- Tiingo provides ETF prices, not direct commodity prices — absolute values differ from the Alpha Vantage commodity series
- the cache introduces a small amount of additional state to maintain
- switching providers mid-session replaces stored observations for each instrument, which means the user loses their previous provider's data for those instruments
- the activity log panel consumes vertical screen space

### Neutral

- the `RefreshOutput` type was removed; all refresh communication now goes through `RefreshEvent`
- `ThresholdConfig` gained a `PartialEq` derive for cache invalidation comparison

## Alternatives Considered

### Alternative 1: Use egui_plot or plotters instead of custom chart painter

Not pursued because:

- the custom painter already exists and works
- adding interactivity to it was straightforward
- a library switch would require rewriting all chart rendering and threshold band logic

### Alternative 2: Debounce threshold slider changes to reduce recomputation during drags

Not pursued because:

- the cache already limits recomputation to once per frame at most
- debouncing would add UI latency and complexity for marginal gain

### Alternative 3: Use FRED for more instruments instead of adding Tiingo

Partially viable — FRED has some commodity price indices — but:

- FRED commodity data is mostly monthly IMF/World Bank prices
- FRED does not cover Bitcoin, natural gas, or several other instruments
- Tiingo covers all instruments at daily frequency through ETFs

### Alternative 4: Polygon.io as the alternative provider

Not selected because:

- Polygon's free tier has a 5-call/minute limit and provides only delayed data
- Tiingo's free tier is substantially more generous

## Implementation Notes

- cache invalidation keys: `data_generation` (incremented on every `reload_from_storage`) and `threshold_config` (compared via `PartialEq`)
- the `Receiver` is temporarily taken out of `self` during `poll_refresh` to avoid borrow-checker conflicts between the channel read and mutable log updates
- `interpolate_at()` uses linear interpolation between adjacent data points for hover value readouts
- Tiingo dates arrive as ISO 8601 with timezone (`2023-01-03T00:00:00+00:00`); we parse only the first 10 characters
- all three API keys support `.env` loading with `dotenvy`: `FRED_API_KEY`, `ALPHA_VANTAGE_API_KEY`, `TIINGO_API_KEY`

## Review Trigger

Review this ADR if:

- the app moves to a charting library that provides its own hover/crosshair interaction
- the cache invalidation model needs to extend beyond threshold config (e.g., per-instrument cache keys)
- Tiingo changes its free-tier terms or ETF coverage
- the app adds a provider that requires authentication beyond a simple API key
- the app needs to support simultaneous use of multiple commodity providers rather than switching between them
