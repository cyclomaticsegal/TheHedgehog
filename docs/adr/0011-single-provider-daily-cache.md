# ADR 0011: Single-Provider Commodity Pipeline with Daily Cache

**Date:** 2026-04-07
**Status:** Accepted
**Supersedes:** [ADR 0010](0010-multi-provider-commodity-caching.md)

## Context

ADR 0010 introduced a multi-provider commodity pipeline (Alpha Vantage spot + Tiingo ETF proxies) with parallel storage so users could compare or fall back between providers. Within hours of landing, two problems were obvious:

1. **Tiingo data is ETF prices, not spot.** GLD, SLV, USO, IBIT, etc. are share prices for funds that hold (or synthesise exposure to) the underlying. They diverge from spot via tracking error, contango, expense ratios, and ETF-share dynamics. Mixing them with Alpha Vantage spot prices for percentile-based regime detection produces nonsense — the percentile windows shift depending on which source the user happens to be looking at.
2. **Multi-provider state was complexity for no real user benefit.** The user does not want to compare providers; they want one trustworthy commodity series per instrument. The branching logic for provider selection, dual cache maps, the source-tagged filter on every chart load, and the per-commodity provider toggle in the sidebar were all paying for an option that was never going to be exercised.

A separate cache bug surfaced while auditing: `storage::last_observation_date` did an exact source match, but the populated cache map passed `"Alpha Vantage"` while the actual stored sources were `"Alpha Vantage GOLD"`, `"Alpha Vantage SILVER"`, etc. The Alpha Vantage cache lookup never matched, so the daily-cache layer ADR 0010 was supposed to provide was silently a no-op.

## Decision

### Single provider per instrument

| Instrument | Source |
|---|---|
| VIX | FRED `VIXCLS` |
| Soybeans | FRED `PSOYBUSDM` |
| Gold, Silver, Bitcoin, Crude Oil (WTI), Natural Gas, Copper, Aluminum, Wheat, Corn | Alpha Vantage |

No selection UI. No fallback. If the user wants different data, they edit `providers.rs`.

### Daily cache

- Storage exposes `last_observation_date_for_provider(instrument, source_prefix) -> Option<NaiveDate>` using a `LIKE` match so callers can pass a provider prefix without knowing per-symbol suffixes.
- On startup (or manual refresh), `start_refresh` builds two cache maps: `cached_dates_fred` (VIX + Soybeans) and `cached_dates_alpha` (everything else).
- `providers::refresh_market_data` checks `cached_date == today` per spec and emits a `RefreshEvent::Cached { instrument, source, date }` instead of issuing an API request.
- Result: opening the app any number of times in the same trading day burns the API quota at most once. The fetch only fires once Alpha Vantage publishes a new daily close.

### Auto-refresh on startup is the default

`AppSettings::auto_refresh_on_startup` now defaults to `true` and is wired into `App::new`. The checkbox in the sidebar lets users opt out, but for the default workflow ("open app → see today's data") no manual refresh is needed.

### API key visibility in the sidebar

Each key field renders a `● set` (green) / `○ not set` (red) badge alongside the label so the user can see at a glance which providers are configured without un-masking the key.

### Activity log surfaces cache hits

`LogStatus::Cached(date)` is rendered in muted text alongside the OK / Failed entries, so the user can confirm at a glance that the cache layer skipped an API call.

## Consequences

### Positive
- One source of truth per instrument — percentile thresholds are consistent.
- Free Alpha Vantage tier (25 req/day) is no longer at risk: 9 commodity calls + 2 FRED calls on the first launch of the day, then zero.
- ~350 lines of multi-provider branching deleted across `models.rs`, `storage.rs`, `providers.rs`, `app.rs`.
- The `(instrument, date, source)` PRIMARY KEY introduced by ADR 0010 stays, because it cleanly supports the per-source date queries the cache uses.

### Neutral
- Users who want ETF data have to fork. Acceptable for a POC.
- The `source` column is now effectively a single value per `(instrument, date)`, so the composite key is overkill in practice. Kept anyway because dropping it would require a migration with no functional benefit.

### Negative
- If Alpha Vantage is rate-limited or down on the first launch of the day, the user sees stale-by-one-day data until the next launch. Mitigation: the activity log clearly shows which instruments fetched, which were cached, and which failed.

## References

- Cache lookup: `src/storage.rs::last_observation_date_for_provider`
- Cache build + refresh entry point: `src/app.rs::start_refresh`
- Refresh loop with cache short-circuit: `src/providers.rs::refresh_market_data`
- Auto-refresh wiring: `src/app.rs::App::new`
- API key badges: `src/app.rs::api_key_field`
