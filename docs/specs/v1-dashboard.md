# V1 Product Spec: VIX, Metals, and Bitcoin Dashboard

## Status

Drafted on 2026-03-31 after selecting a pure Rust desktop application using `egui`, with free data sources only.

## Goal

Build a desktop dashboard that helps the user monitor the VIX against silver, gold, and Bitcoin, with emphasis on identifying VIX extremes and comparing risk and commodity behavior at the same time.

## Product Summary

The application is a pure Rust desktop app with an `egui` UI. It monitors VIX, silver, gold, and Bitcoin on a shared daily time axis, highlights when VIX is approaching or has reached extreme levels, and presents the tracked assets:

- independently in a four-chart dashboard
- together in an optional normalized overlay
- as a derived composite index built from gold, silver, and Bitcoin

Because the user requires free data sources only, v1 is designed around daily-close data rather than intraday real-time monitoring.

## V1 Scope

### Included

- Desktop application written in Rust
- UI built with `egui`
- Daily-close VIX monitoring
- Daily silver price monitoring
- Daily gold price monitoring
- Daily Bitcoin price monitoring
- Four-chart comparison view
- Optional normalized overlay chart
- Composite commodity index chart
- Visual threshold bands for "approaching extreme" and "extreme"
- Local cached history
- Local alert state inside the app
- Configurable thresholds and lookback window

### Excluded

- Real-time or 15-second VIX streaming
- Paid market data feeds
- Browser-based frontend
- Mobile app
- Broker integration
- Trade execution
- SMS/email/push notification delivery
- Tick, minute, or exchange-level market data

## User Stories

- As a user, I can see the latest VIX state at a glance.
- As a user, I can see silver, gold, and Bitcoin beside VIX on the same date range.
- As a user, I can switch to an overlay view to compare directional behavior.
- As a user, I can inspect a derived composite index that aggregates gold, silver, and Bitcoin movement against VIX.
- As a user, I can tell whether VIX is normal, approaching extreme, or extreme.
- As a user, I can inspect recent history over a configurable rolling window.
- As a user, I can run the app without paying for market data.

## Success Criteria

- The app refreshes daily-close data reliably from free sources.
- The app stores historical data locally and starts quickly after first sync.
- The dashboard clearly separates normal, approaching-extreme, and extreme states.
- The overlay view helps compare relative movement without confusing raw price scales.
- The app remains usable when one provider is temporarily unavailable.

## Data Policy

### Design Constraint

All data sources used by v1 must be free to access for this use case.

### Resulting Product Decision

V1 will use daily-close data, not live intraday data.

This is a direct consequence of the available source landscape as of 2026-03-31:

- official live VIX feeds are commercial
- some public delayed quote pages forbid automated extraction
- free metals and crypto APIs exist, but not with the volume or licensing clarity needed for continuous live monitoring

## Selected Data Sources

### Primary VIX Source

FRED series `VIXCLS`

- Series page: <https://fred.stlouisfed.org/series/VIXCLS>
- API docs: <https://fred.stlouisfed.org/docs/api/fred/series_observations.html>

Rationale:

- free API
- stable and well-documented
- daily close aligns with v1 scope
- avoids scraping restrictions

### Fallback VIX Source

Cboe historical VIX CSV download

- Historical page: <https://ww2.cboe.com/tradable_products/vix/vix_historical_data/>

Rationale:

- official source
- useful for bootstrap or validation
- daily historical data is publicly published

### Primary Metals Source

Alpha Vantage gold and silver history / spot APIs

- Docs: <https://www.alphavantage.co/documentation/>
- Support and free-tier limits: <https://www.alphavantage.co/support/>

Rationale:

- official API for metals historical data
- free access exists
- daily data works within v1

Constraint:

- free tier is limited to 25 requests per day, so the app must poll conservatively

### Primary Bitcoin Source

Alpha Vantage digital currency daily API

- Docs: <https://www.alphavantage.co/documentation/>
- Support and free-tier limits: <https://www.alphavantage.co/support/>

Rationale:

- official free daily Bitcoin history endpoint
- consistent provider family with the metals source
- keeps the provider layer simple in a pure Rust desktop app

## Rejected Data Approaches for V1

### Rejected: Cboe delayed quote page scraping

- Delayed quote page: <https://res.cboe.com/delayed_quotes/vix/>
- Cboe delayed quote API page: <https://res.cboe.com/delayed_quotes/api/quote_table/>

Reason:

- Cboe explicitly warns against automated extraction from the delayed quote site

### Rejected: Paid Cboe streaming VIX feed

- Cboe index feed: <https://www.cboe.com/us/indices/accessing-index-data/>

Reason:

- not free
- outside the user's stated requirement

### Rejected: LBMA benchmark feed as primary silver source

- Precious metal prices: <https://www.lbma.org.uk/prices-and-data/precious-metal-prices>
- Auction timing and delayed publication notes: <https://www.lbma.org.uk/prices-and-data/about-lbma-daily-auction-prices>
- November 12, 2025 access-policy update: <https://www.lbma.org.uk/articles/lbma-benchmark-prices-data-tables-are-moving>

Reason:

- licensing constraints around real-time and historical benchmark usage
- delayed public publication
- not a clean fit for an always-refreshing free app feed

## Product Behavior

### Default View

- top-left: VIX chart
- top-right: silver chart
- bottom-left: gold chart
- bottom-right: Bitcoin chart
- shared rolling window selector
- latest status summary for all tracked assets

### Overlay View

- toggle enables a combined chart
- all tracked assets are normalized to a common base, for example 100 at window start
- raw-value labels remain available outside the chart so normalization does not obscure actual levels

### Composite Commodity Index

- equal-weight normalized basket of silver, gold, and Bitcoin
- rebased to 100 at the start of the selected window
- shown in a dedicated chart against normalized VIX

### VIX State Model

V1 should support two threshold systems:

1. Fixed thresholds
2. Rolling percentile thresholds

The default mode should be rolling percentiles because "extreme" is context-sensitive across regimes.

#### Proposed Default Logic

- lookback window: 252 trading days
- approaching extreme: VIX at or above 85th percentile of the rolling window
- extreme: VIX at or above 95th percentile of the rolling window

Inference:

These values are a reasonable v1 default for a comparative dashboard, but they are product defaults rather than market truth. They should be user-configurable.

### Alerts

V1 alerts are in-app only:

- status banner
- colored indicator
- optional audible cue

Alert conditions:

- entering approaching-extreme state
- entering extreme state
- leaving extreme state

No external notification transport is included in v1.

## Functional Requirements

### Data Ingestion

- Fetch VIX daily observations from FRED.
- Fetch silver daily observations from Alpha Vantage.
- Fetch gold daily observations from Alpha Vantage.
- Fetch Bitcoin daily observations from Alpha Vantage.
- Persist normalized internal records locally.
- Avoid duplicate inserts.
- Backfill missing dates on startup.

### Caching

- Store history locally in SQLite.
- Keep the latest successful fetch timestamp per provider.
- Support offline startup from cached data.

### Analysis Engine

- Compute rolling percentiles for VIX.
- Support fixed-threshold mode as a config option.
- Derive state transitions: `normal`, `approaching_extreme`, `extreme`.
- Align VIX, metals, and Bitcoin by trading date for display.
- Compute a composite index from normalized gold, silver, and Bitcoin series.

### UI

- Four-chart dashboard
- Overlay toggle
- Composite index chart
- Window presets: `1M`, `3M`, `6M`, `1Y`, `All`
- Status summary panel
- Threshold configuration panel
- Data freshness indicator

## Non-Functional Requirements

- Pure Rust implementation
- Startup under 2 seconds after cache exists
- Graceful handling of provider failures
- No dependence on scraping prohibited pages
- Conservative API usage suitable for free tiers

## Proposed Architecture

```text
egui App
  |
  +-- ViewModel / App State
        |
        +-- Analysis Engine
        |     +-- rolling percentile calculator
        |     +-- threshold evaluator
        |     +-- date alignment / normalization
        |     +-- composite index calculation
        |
        +-- Data Service
        |     +-- VixProvider (FRED primary, Cboe bootstrap optional)
        |     +-- MetalsProvider (Alpha Vantage)
        |     +-- BitcoinProvider (Alpha Vantage)
        |     +-- Sync Scheduler
        |
        +-- Storage
              +-- SQLite cache
```

## Component Notes

### UI Layer

- `eframe` + `egui`
- charting via `egui_plot` or equivalent
- stateful controls for window and threshold mode

### Data Service

- provider trait per instrument/source
- provider responses converted into one internal `Observation` model
- sync runs on startup and on a low-frequency timer

### Storage

Tables:

- `observations`
- `provider_sync_state`
- `app_settings`
- `alert_events`

## Refresh Strategy

Because data is daily in v1:

- sync on startup
- sync on manual refresh
- scheduled background refresh once every few hours at most

Inference:

This keeps a full refresh, which now includes gold, silver, and Bitcoin in addition to VIX, inside the free-tier request budget while still making the app feel current for daily-close use.

## Configuration

User-configurable settings:

- lookback window
- threshold mode
- fixed threshold values
- percentile cutoffs
- default chart window
- optional sound on alert
- API keys where required

## Risks and Mitigations

### Risk: Free-source limits or policy changes

Mitigation:

- keep provider abstraction clean
- allow swapping providers without rewriting UI or engine

### Risk: Date mismatches between VIX and silver

Mitigation:

- use a shared aligned calendar in the analysis layer
- surface missing-data gaps explicitly

### Risk: Users expect live monitoring

Mitigation:

- clearly label v1 as daily-close monitoring
- show data freshness and last-update timestamps

## Future Versions

### Candidate V2

- paid live VIX provider
- higher-frequency silver source
- desktop notifications outside the app
- annotation support
- exportable screenshots and CSV

## Open Questions

- Should gold and silver be modeled as spot-only instruments, or should the app later support alternate proxies such as futures or ETFs?
- Should the composite remain equal-weighted, or should later versions support configurable weights?
- Should v1 thresholds default to percentile mode only, or expose both percentile and fixed modes in the first release?
- Should the app maintain separate windows for display and percentile calculation?
