# ADR-0001: Performance Improvements — Memory Allocation and Hot-Path Optimisations

**Status:** Accepted  
**Date:** 2026-04-02  
**Branch:** `perf-imp`

---

## Context

A full review of the codebase (`src/analysis.rs`, `src/app.rs`, `src/providers.rs`) was carried out before any production load testing. Five categories of issue were identified:

1. **O(n² log n) algorithm in spike-episode analysis** — `recent_spike_episodes` iterated over every observation and, on each iteration, allocated a fresh `Vec<f64>`, copied the growing window into it, then called `percentile()` *twice*, each time performing an O(k log k) sort of the same data. For the default 252-day lookback window on ~620 daily observations this amounted to ~1,240 allocations and ~1,240 sorts per analysis run.

2. **Unnecessary `Vec` clone inside the per-frame chart painter** — `paint_chart` builds a `Vec<Pos2>` of screen-space points, then clones the entire vector before passing it to `egui::Shape::line()` (which takes ownership). The clone existed solely to preserve a reference to the last point for the terminal dot. Cloning potentially hundreds of points per series, per chart, per frame was pure waste.

3. **Unnecessary `ThresholdSnapshot` clone in `chart_vix`** — `cached_vix_status.as_ref().map(|s| s.thresholds.clone())` produced an owned copy of a two-`f64` struct on every frame when `paint_chart` already accepted `Option<&ThresholdSnapshot>`.

4. **Per-frame heap allocation in `sanitize_overlay_selection`** — The function, called unconditionally on every `eframe::App::update`, built a new `Vec` to deduplicate the overlay instrument list via an O(n²) `contains` scan, then replaced the original `Vec`. With at most 11 instruments this is small in absolute terms, but the allocation pattern was incorrect and needless.

5. **Two-pass iteration with per-key `String` allocation in JSON key search** — `extract_close_value` in `providers.rs` iterated the same `serde_json::Map` twice to find a matching key, calling `key.to_ascii_lowercase()` (heap-allocating a `String`) on every key in each pass.

---

## Decision

All five issues were fixed on the `perf-imp` branch with targeted, minimal changes. No interfaces were altered and no new abstractions were introduced.

### 1. `analysis.rs` — Eliminate per-iteration allocations and redundant sorts

**`recent_spike_episodes`** was restructured as follows:

- For `ThresholdMode::Fixed`, thresholds are constant for the entire call. They are now computed once before the loop and reused, short-circuiting all per-iteration allocation and sorting.
- For `ThresholdMode::RollingPercentile`, a single `Vec<f64>` buffer (`closes_buf`) is pre-allocated outside the loop with `Vec::with_capacity(window_size)`. Each iteration clears and refills this buffer (no allocation after the first fill once capacity is reached) and sorts it **once**, then calls the new `percentile_of_sorted` helper for both threshold values.

**`percentile_of_sorted`** was extracted as a private helper that operates on a pre-sorted slice. The existing `percentile` function delegates to it after sorting, preserving the original API for `compute_vix_status`.

**Effect:** Allocations in the rolling-percentile path drop from O(2n) to O(1) (the one pre-allocated buffer). Sorts drop from O(2n) to O(n). For the default configuration this is roughly a 1,240× reduction in allocations and 2× reduction in sort work.

**Trade-off considered:** A sliding-window order-statistics structure (e.g., two heaps) would achieve O(n log k) total rather than O(n k log k), but adds significant complexity. For the current dataset sizes (~620 observations, 252-day window) the simpler buffer-reuse approach is sufficient and easier to audit.

### 2. `app.rs` — Eliminate `screen_points` clone in `paint_chart`

The last `Pos2` in the vector is saved with `.last().copied()` before the vector is moved into `Shape::line()`. `Pos2` is `Copy`, so this is a single 8-byte register copy rather than a heap allocation.

**Effect:** Eliminates one `Vec<Pos2>` clone per series per frame (up to ~10 series × 2 charts = ~20 clones/frame, each holding hundreds of points).

### 3. `app.rs` — Borrow `thresholds` instead of cloning in `chart_vix`

`paint_chart` already accepted `Option<&ThresholdSnapshot>`. Changing the call site from `.map(|s| s.thresholds.clone())` to `.map(|s| &s.thresholds)` removes a 16-byte struct copy per frame and eliminates the spurious `.as_ref()` double-borrow that the previous API required.

### 4. `app.rs` — Stack-allocated deduplication in `sanitize_overlay_selection`

The heap-allocated `deduped` Vec is replaced by a `[bool; 11]` array on the stack, indexed by a deterministic match over the 11-variant `Instrument` enum. A single `retain` pass performs deduplication with zero heap allocation.

**Effect:** Removes one `Vec<Instrument>` allocation and one `Vec` replacement per frame.

**Trade-off considered:** Giving `Instrument` an `index() -> usize` method or `#[repr(u8)]` discriminant would be cleaner but touches the public model type. The explicit match is safe, local, and will produce a compile error if a new `Instrument` variant is added without updating this function (exhaustive matching).

### 5. `providers.rs` — Single-pass, zero-allocation key search in `extract_close_value`

A private `contains_ci` helper was added that performs an ASCII-case-insensitive substring search using only byte slices — no `String` allocation. `extract_close_value` now makes a single pass, returning immediately on a `"close" + "usd"` match or falling back to a remembered `"close"`-only candidate.

**Effect:** Reduces iterations from 2× |keys| to 1× |keys| in the worst case; eliminates all per-key `String` allocations. This function runs once per instrument per API refresh (not in a hot rendering loop), so the improvement is modest but the pattern was incorrect regardless.

---

## Consequences

### Positive
- Spike-episode analysis no longer has quadratic allocation behaviour; it will not degrade noticeably as historical datasets grow.
- Per-frame memory churn in the rendering loop is substantially reduced, lowering pressure on the system allocator and improving frame-time consistency.
- All changes are strictly local — no public API surfaces, no data model changes, no new dependencies.

### Neutral
- `percentile_of_sorted` is private to `analysis.rs`. If a future caller needs it, it can be made `pub(crate)`.
- The `[bool; 11]` bitmask in `sanitize_overlay_selection` must be kept in sync with `Instrument::ALL`. Rust's exhaustive match will catch any mismatch at compile time.

### Not addressed in this ADR
- The `HashMap<Instrument, Vec<Observation>>` data store could be replaced with a fixed-size array for O(1) access without hashing. Deferred: requires adding an `index()` method to `Instrument` and changing `reload_from_storage`, `series()`, and all call sites — a larger refactor with no correctness impact at current scale.
- Repeated `filter_for_zoom` calls per overlay instrument per frame (no memoisation). Deferred: would require storing per-frame filtered slices in `DashboardApp`, increasing state complexity.
- The O(n log n) spike analysis algorithm could be further reduced to O(n log k) with a dual-heap sliding-window median. Deferred pending real performance profiling data.
