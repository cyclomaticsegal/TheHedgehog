# ADR 0003: Real-Data-First Mode and Selector-Driven Overlays

## Status

Accepted on 2026-03-31

## Context

The grouped monitor introduced two usability regressions:

- automatic example-data seeding could overwrite previously cached real data
- the overlay model became too rigid to support the user's actual comparison workflow

The user wants:

- real data as the primary operating mode
- example data only for explicit development/testing use
- flexible overlays such as:
  - VIX + group composite
  - VIX + Bitcoin
  - VIX + gold
  - VIX + gold + silver
  - other user-chosen subsets

## Decision

We will make the dashboard real-data-first and selector-driven.

Specifically:

- the app must not automatically seed example data on startup when real data is absent or incomplete
- example data becomes an explicit user action only
- live refreshes replace an instrument's cached history rather than merging into potentially stale example rows
- overlays will always include VIX and optionally include:
  - any user-selected subset of instruments
  - the currently selected group composite

## Why

- silent fallback to example data undermines trust in the dashboard
- correlation work requires direct control over which assets are visible together
- the user's actual workflow is comparative and exploratory, not limited to one hardcoded overlay

## Consequences

### Positive

- the app becomes much more trustworthy as a monitoring tool
- overlays now match the user's actual analysis style
- real and example modes are clearly separated

### Negative

- the overlay UI gains more state and control surface
- first-run UX is less visually impressive when no real data is loaded yet

## Alternatives Considered

### Alternative 1: Keep auto-seeding as a convenience fallback

Rejected because:

- it can overwrite cached real data and mislead the user about data provenance

### Alternative 2: Keep only hardcoded overlays

Rejected because:

- it does not support the user's requested comparison patterns

## Implementation Notes

- source state should be visible in the UI
- live refresh should replace instrument histories atomically
- example data should remain useful for development and should better mimic typical VIX spike behavior, including short V-shaped shock patterns

## Review Trigger

Review this ADR if:

- the app adds a multi-select chart builder beyond the current overlay selector
- the project introduces a dedicated live-vs-example mode switch
- the storage layer gains per-series provenance/versioning
