# Rolling Decision Log

## Purpose

This file preserves the working reasoning behind the v1 recommendations so later implementation decisions can be traced back to their source and date.

## 2026-03-31

### Decision 1: Use pure Rust with `egui`

User direction:

- build a Rust application
- prefer pure Rust with `egui`

Reasoning:

- Rust is a strong fit for long-running data ingestion, local caching, and deterministic alert logic.
- `egui` is a practical way to stay pure Rust and avoid splitting the stack.
- The tradeoff is UI polish versus a web-charting frontend, but the user accepted the pure Rust route.

Outcome:

- accepted

### Decision 2: Constrain v1 to free data only

User direction:

- all data sources must be free

Reasoning:

- this changes the shape of the product more than the language choice does
- free live VIX access is materially harder than free daily-close access

Outcome:

- accepted

### Decision 3: Do not build v1 around commercial live VIX feeds

Evidence:

- Cboe describes the official VIX-capable index feed as a real-time streaming service and directs users to request pricing: <https://www.cboe.com/us/indices/accessing-index-data/>

Inference:

- live official VIX access is a commercial market-data product, not a free app dependency

Outcome:

- rejected for v1

### Decision 4: Do not scrape Cboe delayed quote pages

Evidence:

- Cboe's delayed quote tooling includes a prohibition on downloading delayed quote table data by auto-extraction software: <https://res.cboe.com/delayed_quotes/api/quote_table/>

Inference:

- even if technically possible, this is the wrong foundation for a desktop app

Outcome:

- rejected for v1

### Decision 5: Use free daily VIX data instead

Evidence:

- FRED exposes the `VIXCLS` daily-close series: <https://fred.stlouisfed.org/series/VIXCLS>
- FRED observation API docs are stable and explicit: <https://fred.stlouisfed.org/docs/api/fred/series_observations.html>
- Cboe also publishes historical VIX CSV files updated daily: <https://ww2.cboe.com/tradable_products/vix/vix_historical_data/>

Reasoning:

- daily-close VIX is sufficient for a first dashboard that focuses on extreme detection and historical comparison
- it avoids licensing ambiguity and prohibited scraping

Outcome:

- accepted

### Decision 6: Use Alpha Vantage for silver in v1

Evidence:

- Alpha Vantage documents silver spot and silver history endpoints: <https://www.alphavantage.co/documentation/>
- Alpha Vantage support states the free plan covers up to 25 requests per day: <https://www.alphavantage.co/support/>

Reasoning:

- the silver history endpoint is a practical free source for a daily dashboard
- the free-tier cap means the app must sync sparingly and cache locally

Outcome:

- accepted for v1 with conservative polling

### Decision 7: Do not use LBMA benchmark data as the primary free application feed

Evidence:

- LBMA states that a licence is required to obtain and use real-time or historical LBMA Gold and Silver Price data: <https://www.lbma.org.uk/prices-and-data/precious-metal-prices>
- LBMA states public publication is delayed to midnight London time: <https://www.lbma.org.uk/prices-and-data/about-lbma-daily-auction-prices>
- LBMA announced in November 2025 that historic benchmark tables moved to the Members' Portal with licensing and self-certification requirements: <https://www.lbma.org.uk/articles/lbma-benchmark-prices-data-tables-are-moving>

Reasoning:

- LBMA remains useful as a benchmark reference, but it is not the cleanest free operational source for this application

Outcome:

- rejected as primary v1 feed

### Decision 8: Make the dashboard daily-close, not intraday live

Reasoning:

- This is the cleanest product that satisfies all accepted constraints:
  - pure Rust
  - `egui`
  - free data sources only
  - side-by-side and overlay comparison

Inference:

- "approaching extremes" in v1 means approaching daily-close extremes computed from a rolling window, not intraday threshold crossing

Outcome:

- accepted

### Decision 9: Use rolling percentile thresholds by default

Reasoning:

- fixed VIX numbers are simple, but they can misstate unusualness across changing volatility regimes
- percentile thresholds adapt better for a comparative dashboard

Proposed defaults:

- lookback: 252 trading days
- approaching extreme: 85th percentile
- extreme: 95th percentile

Inference:

- these defaults are product defaults for usability, not objective market truth

Outcome:

- accepted as v1 default, with configuration support

### Decision 10: Expand the comparison set to gold and Bitcoin

User direction:

- add gold
- add Bitcoin
- include both in the overlay

Evidence:

- Alpha Vantage documents the `GOLD_SILVER_HISTORY` endpoint for precious metals and `DIGITAL_CURRENCY_DAILY` for Bitcoin daily history: <https://www.alphavantage.co/documentation/>
- Alpha Vantage support still frames the free plan as capped at 25 requests per day: <https://www.alphavantage.co/support/>

Reasoning:

- gold strengthens the metals comparison beside silver
- Bitcoin adds a risk-sensitive non-metal comparator that the user explicitly wants
- the same provider family can support all three non-VIX series

Inference:

- a full refresh now means one FRED request plus three Alpha Vantage requests, so the daily-refresh design constraint becomes even more important

Outcome:

- accepted

### Decision 11: Define the composite commodity index as an equal-weight normalized basket

User direction:

- add a general indicator aggregating gold, silver, and Bitcoin movement against VIX

Reasoning:

- the tracked assets have very different absolute price scales, so raw averaging is misleading
- normalization to a common base allows comparison of direction and relative magnitude
- equal weighting is the simplest transparent v1 rule

Inference:

- the "composite commodity index" in v1 is a convenience comparison indicator, not a benchmark index or investable basket

Outcome:

- accepted for v1

### Decision 12: Refactor from a fixed watchlist into grouped macro monitoring

User direction:

- expand greatly the commodities
- organize them in macro-relevant groupings

Reasoning:

- the user's grouping is based on economic leverage, not simple popularity
- continuing to add hardcoded series would make the application structurally weak
- the grouped model is the right abstraction boundary for future expansion

Outcome:

- accepted

### Decision 13: Support a first wave of grouped assets with mixed frequencies

Accepted first-wave groups:

- Volatility: VIX
- Monetary: gold, silver
- Energy: crude oil, natural gas
- Industrial: copper, aluminum
- Agriculture: wheat, corn, soybeans
- Risk: Bitcoin

Evidence:

- FRED provides `VIXCLS` and `PSOYBUSDM` through its official observation API: <https://fred.stlouisfed.org/docs/api/fred/series_observations.html>
- Alpha Vantage documents official endpoints for precious metals, Bitcoin daily history, and commodity series such as `WTI`, `NATURAL_GAS`, `COPPER`, `ALUMINUM`, `WHEAT`, and `CORN`: <https://www.alphavantage.co/documentation/>
- Alpha Vantage support still states the free plan is limited: <https://www.alphavantage.co/support/>

Inference:

- the grouped macro monitor should explicitly tolerate mixed frequencies rather than pretend everything is daily

Outcome:

- accepted

### Decision 14: Make the dashboard real-data-first

User direction:

- work with real data
- example data is acceptable only for development purposes

Reasoning:

- automatic example-data fallback can mislead the user about what they are actually observing
- the dashboard is meant for correlation work, so source trust matters directly

Outcome:

- accepted

### Decision 15: Move overlays to a selector-driven model

User direction:

- overlay group composites with VIX
- overlay single assets with VIX
- overlay small subsets such as VIX + gold + silver

Reasoning:

- the user is doing exploratory correlation analysis rather than consuming a single fixed dashboard view
- the correct abstraction is not "one overlay", but "VIX plus a user-selected set of comparison lines"

Outcome:

- accepted

## Maintenance Rule

When a later implementation or source decision changes this direction, append a new dated entry instead of rewriting history in place.
