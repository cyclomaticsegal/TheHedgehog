# ADR 0021: Instrument Set Reduction and Bitcoin Regrouping

**Date:** 2026-04-16
**Status:** Accepted
**Extends:** ADR 0008 (51Folds integration), ADR 0010 (multi-provider commodity caching), ADR 0011 (single-provider daily cache)

## Context

The Hedgehog shipped with eleven instruments across six asset groups: VIX, Gold, Silver, Bitcoin, Crude Oil, Natural Gas, Copper, Aluminum, Wheat, Corn, Soybeans. Every instrument was expected to produce a daily close so the VIX overlay and AI analysis could reason about current moves.

In practice, five of those instruments did not. Copper, Aluminum, Wheat, Corn, and Soybeans were chronically stale — latest closes weeks to months behind the app's running date. An investigation on 2026-04-12 suspected Alpha Vantage data lag; after switching the series to `interval=daily` and rebuilding, the problem persisted. Copper reached 46 days stale by 2026-04-16; Soybeans had no data at all.

The cause is not our fetcher. **Alpha Vantage's free Commodity API does not support `interval=daily` for `COPPER`, `ALUMINUM`, `WHEAT`, `CORN`, or `SOYBEANS`.** Those series are published at monthly cadence (derived from World Bank / monthly benchmarks and USDA WASDE reports). When we request `daily` the endpoint either falls back to the monthly print — so the latest "close" is whenever the last monthly report posted — or returns nothing. `WTI` and `NATURAL_GAS` are the only Commodity-endpoint series that genuinely support daily. `GOLD` and `SILVER` are served by a different endpoint (spot metals) that does support daily, and `BTC` comes from the digital-currency endpoint.

Five instruments producing stale-or-absent data makes them unusable for a tool whose central premise is "what's happening in markets right now." A 6-month-old copper print cannot inform an AI hypothesis about a VIX spike that started yesterday.

## Decisions

### 1. Drop the five stale instruments

`Copper`, `Aluminum`, `Wheat`, `Corn`, and `Soybeans` are removed from the `Instrument` enum and every surface that references them — provider specs, overlay picker, chart colour map, knowledge base, help text, AI prompt attribution string, evaluation tests, sort-order match arms. The historical rows already in the SQLite database are left in place (the `observations` table accepts any `storage_key`) but no longer fetched, displayed, or referenced.

Rationale: better to remove an instrument than to silently present stale data as if it were current. The AI prompt ground-truth rule says "the user message values override anything web search returns" — but the user message needs to be trustworthy. A 46-day-old copper close poisons that contract.

### 2. Collapse two asset groups

`AssetGroup::Industrial` (Copper + Aluminum) and `AssetGroup::Agriculture` (Wheat + Corn + Soybeans) become empty with no members, so the variants are deleted entirely. `AssetGroup::ALL` drops from 6 to 3.

### 3. Fold Bitcoin into the Monetary / Store of Value group

`AssetGroup::Risk` (which contained only Bitcoin) is also removed. Bitcoin moves into `AssetGroup::Monetary` alongside Gold and Silver.

Rationale: the "Risk / Alternative" label was forced by Bitcoin's original positioning as uncorrelated-to-equities. Empirically that framing failed during the 2020 and 2022 drawdowns — Bitcoin traded as a high-beta risk asset against the Nasdaq. The current application narrative treats it as a monetary store-of-value candidate evaluated alongside Gold and Silver, so the group membership should reflect that.

### 4. Final instrument set

Six instruments across three groups:

- **Volatility**: VIX
- **Monetary / Store of Value**: Gold, Silver, Bitcoin
- **Energy**: Crude Oil, Natural Gas

### 5. Settings compatibility strategy

User-saved settings (stored as JSON in `app_settings`) may contain stale enum values — `"selected_group": "Risk"` or `"overlay_instruments": ["Copper", ...]`. Serde will fail to deserialize those fields, which `load_settings` wraps in a fallback to `AppSettings::default()` (at `src/app.rs` where `load_settings` is called with `unwrap_or_else`).

Net effect: users upgrading from a pre-0021 build lose their persisted settings (threshold config, selected group, overlay choice) and get defaults. API keys are unaffected (they live in `.env`, not settings). Historical inferences and 51Folds models in the DB are unaffected.

Acceptable for a preview tool; a migration wasn't worth building.

## Consequences

- Users upgrading from an older build see their settings reset to defaults on first launch. A small note in the release documentation would soften this; none is shipped here.
- The AI analysis is now fed only data it can actually reason about. Stale prices never reach the prompt.
- The knowledge base lost ~8 chunks (Copper, Aluminum, Wheat, Corn, Soybeans instrument-specific plus two cross-asset "Industrial Metals" and "Agriculture" entries) and had historical-episode narratives trimmed to remove references to those commodities. No new chunks added.
- The default sidebar load now ticks three instruments (Gold, Silver, Bitcoin) out of five overlay-eligible — a much higher primary-to-tertiary ratio than before. The structural bias checks and LLM judge built on a "selected = subject, unselected = background" binary lose resolving power at this ratio. This triggered the companion ADR 0022 (primary/secondary/tertiary tier framework).
- Bringing Copper or Aluminum back would require switching to a different data provider (FRED has some daily industrial-metal series; Quandl/Nasdaq Data Link has more). Filed as a future consideration, not planned.
- Test coverage adjusted: one regression test (`soybeans_not_attributed_to_fred`) removed; `eval_metals_basket` narrowed to Gold+Silver; `eval_bitcoin_only` added in place of the removed `eval_soybeans_only`.
