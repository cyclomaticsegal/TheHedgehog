# V2 Product Spec: Grouped Macro Monitor

## Status

Drafted on 2026-03-31 after expanding the dashboard from a small fixed watchlist into a grouped macro monitor.

## Goal

Build a grouped desktop dashboard that lets the user compare VIX behavior against the most systemically important commodity and risk buckets using free official sources.

## Product Shape

The application remains a pure Rust desktop app using `egui`, but the interaction model changes from a small fixed dashboard to a grouped monitor with:

- group navigation
- group-level derived indexes
- per-instrument drill-down
- macro composite comparison against VIX

## First-Wave Groups

### Volatility

- VIX

### Monetary / Store of Value

- Gold
- Silver

### Energy

- Crude oil
- Natural gas

### Industrial Metals

- Copper
- Aluminum

### Agriculture

- Wheat
- Corn
- Soybeans

### Risk / Alternative

- Bitcoin

## Why This Cut

This first-wave set captures the highest-signal macro groups that are supportable with free official feeds:

- `VIX`: risk/volatility state
- `Monetary`: trust hedge and store-of-value assets
- `Energy`: inflation and industrial cost base
- `Industrial`: growth and infrastructure signal
- `Agriculture`: food security and political stability
- `Risk`: speculative liquidity and alternative beta

## Data Sources

### FRED

- VIX daily close `VIXCLS`
- Soybeans monthly `PSOYBUSDM`
- FRED API docs: <https://fred.stlouisfed.org/docs/api/fred/series_observations.html>

### Alpha Vantage

- `GOLD_SILVER_HISTORY` for gold and silver
- `DIGITAL_CURRENCY_DAILY` for Bitcoin
- commodity endpoints including `WTI`, `NATURAL_GAS`, `COPPER`, `ALUMINUM`, `WHEAT`, `CORN`
- Documentation: <https://www.alphavantage.co/documentation/>
- Support/free-plan limits: <https://www.alphavantage.co/support/>

## Important Constraint

The grouped expansion introduces mixed-frequency data:

- daily: VIX, gold, silver, Bitcoin, crude oil, natural gas
- monthly: copper, aluminum, wheat, corn, soybeans

This is acceptable because the product is explicitly a macro monitor, not an intraday execution terminal.

## Derived Indexes

### Group Index

For any non-volatility group, compute an equal-weight normalized basket of the instruments in the selected group.

### Macro Composite

Compute an equal-weight normalized basket of all non-VIX first-wave assets.

### Interpretation

These are convenience comparison indexes for dashboard analysis. They are not benchmark indexes, investable products, or economically weighted baskets.

## UI Model

### Sidebar

- API key settings
- threshold settings
- group selector
- detail instrument selector
- status summary
- recent alert log

### Main Area

- VIX status banner
- chart grid for the selected group
- selected group index vs VIX
- optional group overlay
- selected instrument vs VIX
- macro composite vs VIX

## Functional Requirements

- Group navigation must not require code changes per chart.
- Adding a new instrument should be primarily a metadata/provider registration task.
- Each instrument must declare group, storage key, and expected data frequency.
- The dashboard must support mixed frequencies on a shared date axis.
- Derived group indexes must ignore incomplete dates rather than invent aligned values.

## Architectural Direction

The application should use a registry-driven instrument model:

- instrument metadata
- provider binding
- storage mapping
- analysis functions that accept arbitrary slices of observations
- UI rendering that iterates over selected group members

## Risks

### Free-Tier Budget

A full refresh now uses one FRED request plus multiple Alpha Vantage requests. This reinforces the need for low-frequency sync behavior.

### Frequency Mismatch

Monthly agricultural or industrial series will look visually sparse next to daily energy or monetary series.

Mitigation:

- label frequency explicitly in the UI
- normalize only for comparative overlays
- keep raw instrument charts visible

### Coverage Gaps for Later Phases

Coal, LNG as a distinct series, iron ore, steel, lithium, uranium, cobalt, and rare earths are not yet included because free official coverage is less straightforward.

## Next Expansion Candidates

- Brent crude
- Cotton, sugar, coffee
- Uranium
- Lithium proxy
- broader industrial basket
