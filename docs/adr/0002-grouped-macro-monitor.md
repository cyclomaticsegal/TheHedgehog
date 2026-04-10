# ADR 0002: Grouped Macro Monitor and Registry-Driven Instrument Model

## Status

Accepted on 2026-03-31

## Context

The original dashboard started as a small fixed comparison set. The user's later direction broadened the problem into grouped macro monitoring, with emphasis on:

- monetary / store-of-value assets
- energy
- industrial metals
- agriculture
- risk assets

This expansion creates two pressures:

1. the UI must scale beyond hardcoded chart blocks
2. the data layer must handle a larger set of instruments with mixed frequencies

## Decision

We will refactor the app around a registry-driven instrument model and grouped navigation.

The first implementation wave will support:

- VIX
- Gold
- Silver
- Bitcoin
- Crude oil
- Natural gas
- Copper
- Aluminum
- Wheat
- Corn
- Soybeans

The UI will pivot to:

- group selector
- per-group instrument grid
- selected group index vs VIX
- selected instrument vs VIX
- macro composite vs VIX

## Why

- The user's grouping is fundamentally stronger than adding more hardcoded charts.
- A metadata-driven model is the clean boundary for continued expansion.
- Free official data exists for a meaningful first wave, even if some series are monthly.

Primary source references:

- FRED observation API: <https://fred.stlouisfed.org/docs/api/fred/series_observations.html>
- FRED VIX `VIXCLS`: <https://fred.stlouisfed.org/series/VIXCLS>
- FRED soybeans `PSOYBUSDM`: <https://fred.stlouisfed.org/series/PSOYBUSDM>
- Alpha Vantage docs: <https://www.alphavantage.co/documentation/>
- Alpha Vantage support: <https://www.alphavantage.co/support/>

## Consequences

### Positive

- new assets can be added with much less UI churn
- grouped comparisons are clearer than a flat watchlist
- derived indexes allow higher-level macro comparison against VIX

### Negative

- mixed-frequency data complicates presentation
- full refreshes now consume a larger share of the free request budget
- some strategically important later assets still lack clean free coverage

## Alternatives Considered

### Alternative 1: Continue adding assets one by one to the existing app

Rejected because:

- it would make both the UI and state model brittle very quickly

### Alternative 2: Delay grouped design until more assets are added

Rejected because:

- the refactor cost only rises as more hardcoded logic accumulates

### Alternative 3: Require uniform daily data for every instrument

Rejected because:

- it would exclude too many economically important series from the free-data design

## Implementation Notes

- instrument metadata should define storage key and expected frequency
- provider binding should be externalized from the UI layer
- group and macro composites should be equal-weight normalized baskets in v2
- incomplete dates should be dropped from derived basket calculations

## Review Trigger

Review this ADR if:

- the project adds a second provider family beyond FRED and Alpha Vantage for broad commodity coverage
- the project moves to weighted composite indexes
- the project needs intraday group monitoring
