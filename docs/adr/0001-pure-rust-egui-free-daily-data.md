# ADR 0001: Pure Rust + egui with Free Daily Data

## Status

Accepted on 2026-03-31

## Context

The product goal is a desktop dashboard for monitoring the VIX against silver, gold, and Bitcoin, with:

- four individual charts
- an optional overlay
- a composite comparison index
- alerts for VIX extremes or approach to extremes

The user explicitly selected:

- pure Rust
- `egui`
- free data sources only

Those constraints force a decision about whether v1 should be built around live intraday monitoring or daily-close monitoring.

## Decision

We will build v1 as a pure Rust desktop application using `egui`, backed by free daily-close data sources and a local cache.

Selected sources:

- VIX primary: FRED `VIXCLS`
- VIX fallback/bootstrap: Cboe historical VIX CSV
- Silver primary: Alpha Vantage metals history
- Gold primary: Alpha Vantage metals history
- Bitcoin primary: Alpha Vantage digital currency daily history

The app will:

- show VIX, silver, gold, and Bitcoin independently
- provide an optional normalized overlay
- compute a composite index as an equal-weight normalized basket of silver, gold, and Bitcoin
- compute VIX alert states from rolling percentile thresholds by default

## Why

### Rust and egui

- The user asked for pure Rust.
- Rust is a good fit for data ingestion, caching, analysis, and alert evaluation.
- `egui` is the most direct path to a pure Rust desktop UI.

### Daily-close instead of live intraday

- Cboe's official streaming VIX data is commercial: <https://www.cboe.com/us/indices/accessing-index-data/>
- Cboe delayed quote pages are not appropriate for automated extraction: <https://res.cboe.com/delayed_quotes/api/quote_table/>
- FRED provides free daily-close VIX data: <https://fred.stlouisfed.org/series/VIXCLS>
- Alpha Vantage provides metals and digital-currency APIs, but the free tier is limited to 25 requests per day: <https://www.alphavantage.co/support/>

Inference:

- a clean, policy-aligned, free v1 is daily-close

## Consequences

### Positive

- stays within the user's free-data constraint
- keeps the application pure Rust
- avoids scraping prohibited sources
- gives a credible v1 with manageable technical risk
- keeps future provider upgrades isolated behind interfaces

### Negative

- v1 is not a live volatility terminal
- alerts are based on daily-close updates rather than intraday threshold crossing
- metals and Bitcoin freshness are constrained by free-tier API limits
- a full refresh budget is tighter because the app now needs one FRED request plus three Alpha Vantage requests

### Neutral / Follow-on Effects

- if later requirements demand true live alerts, v2 will likely require a paid VIX feed
- the provider abstraction should be treated as a stable boundary from the start

## Alternatives Considered

### Alternative 1: Rust backend plus web frontend

Rejected because:

- the user selected pure Rust with `egui`

### Alternative 2: Paid live VIX feed in v1

Rejected because:

- violates the free-data requirement

### Alternative 3: Scrape delayed quote pages

Rejected because:

- the source explicitly prohibits automated extraction for the delayed quote tables

### Alternative 4: LBMA as primary silver source

Rejected because:

- licensing and delayed-publication model make it a poor default operational feed for this app
- references:
  - <https://www.lbma.org.uk/prices-and-data/precious-metal-prices>
  - <https://www.lbma.org.uk/prices-and-data/about-lbma-daily-auction-prices>
  - <https://www.lbma.org.uk/articles/lbma-benchmark-prices-data-tables-are-moving>

## Implementation Notes

- define provider traits so data sources can be swapped later
- persist data locally in SQLite
- compute rolling percentile thresholds in the analysis layer
- compute the composite index in the analysis layer from normalized component series
- expose threshold mode and lookback as settings
- make data freshness visible in the UI

## Review Trigger

Review this ADR if any of the following changes:

- the user approves paid data
- a clearly licensed free live VIX source becomes available
- the UI stack changes away from pure Rust
