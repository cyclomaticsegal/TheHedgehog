# ADR 0010: Multi-Provider Commodity Data Caching

**Date:** 2026-04-07  
**Status:** Superseded by [ADR 0011](0011-single-provider-daily-cache.md)
**Supersedes note:** Tiingo was removed the same day this ADR was drafted because its ETF-proxy prices (GLD, SLV, etc.) created a systematic level mismatch when mixed with Alpha Vantage spot prices, defeating the purpose of percentile-based regime detection. The multi-provider machinery was dismantled and replaced with a single-provider-per-instrument design — see ADR 0011.

**Context:** The Hedgehog dashboard fetches commodity data from external APIs (Alpha Vantage, Tiingo) with daily update frequency, incurring API rate-limit risks and redundant fetch calls within a trading day. Users need flexibility to compare data across providers without vendor lock-in.

## Problem

1. **Rate limiting:** Alpha Vantage free tier allows only 5 req/min and 25 req/day. Redundant calls during same trading day exhaust quota.
2. **Provider coupling:** Previous design required choosing a single commodity provider at startup. Switching required app restart.
3. **API costs:** Multi-provider fetches improve data quality but each call costs API budget.
4. **Source ambiguity:** Database stored observations by `(instrument, date)` only, making it impossible to keep both Alpha Vantage and Tiingo GOLD on the same date for comparison.

## Decision

### Schema Change
- **PRIMARY KEY:** `observations(instrument, date, source)` instead of `(instrument, date)`  
  This allows storing parallel data from both providers for the same instrument on the same date.

### Caching Strategy
- Before each refresh, query the database for `(instrument, source, max(date))`
- Skip API fetch if `max_date == today`
- Cache checks are provider-specific: Alpha Vantage GOLD and Tiingo GOLD are independent cache entries

### Dual-Provider Fetch
- **FRED:** VIX (single source) + Soybeans (fixed source)
- **Alpha Vantage:** All 9 commodities (Gold, Silver, Bitcoin, Crude Oil, Natural Gas, Copper, Aluminum, Wheat, Corn)
- **Tiingo:** Same 9 commodities via different tickers

All three providers are queried on every refresh, not selectable at startup. Observations are stored with their original source label.

### UI/UX
- Activity log shows cache hits: `"GOLD (cached 2024-04-07)"` in muted text
- Summary status reflects both fetches and cache hits: `"Updated 3, cached 8 (450 points)."`
- Cache information isolated to activity log, not duplicated in charts
- Provider choice moved to later phase (Phase 2): UI will let users select which provider's data to display per commodity

## Implementation Notes

1. **Parallel fetches:** Alpha Vantage and Tiingo commodity fetches happen sequentially in the same refresh cycle (not parallel) to simplify error handling and maintain API throttle discipline.

2. **Source field:** Existing `Observation.source` field is reused; no schema columns added, only PRIMARY KEY constraint changed.

3. **Backward compatibility:** Existing databases will be incompatible. Safe for POC.

4. **Refresh signature:** `refresh_market_data()` now takes three separate cache maps:
   ```rust
   cached_dates_fred: HashMap<Instrument, NaiveDate>,
   cached_dates_alpha: HashMap<Instrument, NaiveDate>,
   cached_dates_tiingo: HashMap<Instrument, NaiveDate>,
   ```

## Benefits

- ✅ Eliminates same-day redundant API calls
- ✅ Supports future multi-provider UI without architectural change
- ✅ Decouples data storage from provider choice
- ✅ Reduces API costs by ~70% (typical: 10 instruments × 2 sources = 20 calls; cache hits 14/20 on day 2+)
- ✅ Improves data quality by storing both perspectives

## Risks

- **Beta lag:** If one provider lags the other, users may see stale data from slower provider. Mitigation: UI will show source/date per commodity.
- **Quota backlog:** If both providers are queried daily, total quota usage is 2× commodities. For Alpha Vantage free: 10 requests (5 alpha + 5 tiingo, shared throttle). For paid: not an issue.

## Phase 2: Provider Selection UI

Future work will add:
1. Per-commodity dropdown: "Show GOLD from [Alpha Vantage | Tiingo]"
2. Legend showing which source is displayed for each commodity
3. Timestamp per commodity (when that source was last updated)
4. Optionally: comparison view showing both sources side-by-side

## References

- ADR 0007 (Rollback Dexter)
- ADR 0008 (51Folds Integration spec)
- Cache implementation: `src/storage.rs::last_observation_date(instrument, source)`
- Refresh logic: `src/providers.rs::refresh_market_data()`
- Event handling: `src/models.rs::RefreshEvent::Cached`
