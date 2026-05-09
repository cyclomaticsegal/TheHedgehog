# Security & Performance Audit — 2026-04-03

**Branch:** `perf-imp`  
**Scope:** Full source scan of `src/` (ai.rs, app.rs, analysis.rs, knowledge.rs, models.rs, providers.rs, storage.rs, help.rs, main.rs)  
**Context:** Post-implementation of ADR-0005 (AI analysis panel) and ADR-0006 (inference persistence + reports)

---

## Security Findings

### CRITICAL

#### 1. SQL Injection in `load_knowledge_chunks`
**File:** `src/storage.rs:199-215`

The `load_knowledge_chunks` method builds SQL with `format!()` string interpolation:

```rust
conditions.push(format!("tags LIKE '%{tag}%'"));
```

Currently safe because callers only pass `Instrument::storage_key()` values (compile-time enum strings). However, the function signature accepts `&[&str]`, creating a latent injection risk if any future caller passes user-controlled input.

**Remediation:** Use parameterized queries with `?` placeholders. Build the parameter list dynamically alongside the SQL conditions.

**Risk today:** Low (callers are safe). Risk if function is reused: Critical.

---

### HIGH

#### 2. API Key Leakage in Error Messages
**File:** `src/ai.rs:62-69`

`send_and_parse()` surfaces the full HTTP response body in error messages. If an API endpoint echoes back request metadata (headers, keys), the error message displayed in the UI could contain the API key.

```rust
return Err(anyhow::anyhow!("HTTP {status}: {body}"));
```

**Remediation:** Truncate or redact response bodies before surfacing in errors. The existing `providers.rs` has a `redact_url_key()` pattern that could be adapted.

#### 3. Non-Atomic `.env` Write
**File:** `src/app.rs:330-343`

`save_keys_to_env()` reads the file, modifies content in memory, then writes back with `fs::write()`. A crash between read and write loses all keys. No backup is created.

**Remediation:** Write to a `.env.tmp` file first, then atomically rename over the original.

#### 4. No Response Size Limit on LLM Calls
**File:** `src/ai.rs:47-58`

The 60-second timeout exists but no maximum response body size is enforced. A compromised or misbehaving API could return a multi-MB response, exhausting memory.

**Remediation:** Read the response body into a size-limited buffer (e.g. 100KB max) before parsing.

---

### MEDIUM

#### 5. Missing Model Name Validation
**File:** `src/models.rs:293-294`, `src/app.rs` (sidebar text field)

The `ai_model` field is a free-text `String` with no length limit or character validation. Newlines, control characters, or very long strings could cause unexpected API behavior.

**Remediation:** Validate: 1-128 characters, alphanumeric + `-` + `.` only.

#### 6. Missing Date Range Validation in Report Window
**File:** `src/app.rs` (report window)

No check that `from <= to`. No maximum range limit. A very large range could load thousands of inferences into memory.

**Remediation:** Validate `from <= to` and cap range at a reasonable maximum (e.g. 5 years).

#### 7. Unvalidated API Response Structure
**File:** `src/ai.rs:88-94, 114-119`

Array index access on API responses (`response["content"][0]["text"]`) returns `Null` on missing keys rather than panicking (serde_json behavior), but the error message is generic. Not a crash risk, but makes debugging harder.

**Remediation:** Use explicit `.get()` / `.as_array()` / `.first()` chain for clearer error messages.

---

### LOW / INFO

#### 8. Error Messages Include File Paths
**File:** `src/storage.rs` (multiple)

Error context strings include database paths and stored values. Minimal risk for a desktop app but worth noting for logging hygiene.

#### 9. `.env` File Permissions Not Set After Write
**File:** `src/app.rs:330-343`

The data directory gets `0o700` on Unix, and the DB file gets `0o600`, but `.env` is written without explicit permission setting. Other users on a shared machine could read it.

**Remediation:** Set `0o600` on `.env` after writing (Unix only).

#### 10. Thread Panic Handling: Good
**Files:** `src/ai.rs:25-44`, `src/app.rs:402-410`

Both AI and refresh threads wrap execution in `std::panic::catch_unwind` and send error messages through the channel. Correct pattern.

#### 11. TLS Configuration: Good
**File:** `src/providers.rs`

Uses `rustls-tls` (not native-tls). `danger_accept_invalid_certs` is not enabled. Certificate validation is active.

---

## Performance Findings

### CRITICAL (Hot Path)

#### 12. Date Formatting in Per-Frame Chart Loop
**File:** `src/app.rs` (paint_chart, x-axis tick loop)

`date.format(date_fmt).to_string()` is called inside a loop that runs 10-60 times per frame, across multiple charts. At 60fps this produces 36,000-216,000 string allocations per second.

**Impact:** Allocator churn. Not visible at current scale but will degrade on high-refresh-rate displays or dense zoom levels.

**Remediation:** Pre-format date strings into a cached Vec, or format only when the visible range changes.

### HIGH

#### 13. Missing Index on `ai_inferences(created_at)`
**File:** `src/storage.rs:66-76`

`load_recent_inferences` uses `ORDER BY created_at DESC` and `load_inferences_in_range` uses `WHERE created_at >= ? AND created_at < ?`. Neither benefits from an index. Currently fast with few rows but degrades at 1000+ inferences.

**Remediation:** Add to `init()`:
```sql
CREATE INDEX IF NOT EXISTS idx_ai_inferences_created_at ON ai_inferences(created_at);
```

#### 14. Vec::collect() for Screen Points Per Frame
**File:** `src/app.rs` (paint_chart)

Each visible series maps all observations to screen-space `Pos2` points via `.collect::<Vec<Pos2>>()` every frame. With ~250 data points per instrument and multiple overlays, this is 600-1200 allocations/sec.

**Impact:** Moderate allocator pressure. Acceptable at current scale.

**Remediation:** Pre-allocate a reusable buffer.

### MEDIUM

#### 15. AI Context Assembly String Growth
**File:** `src/ai.rs:136-152`

`assemble_system_prompt()` builds a 25-100KB string via repeated `push_str()` without pre-allocating capacity. Causes 3-5 reallocation + copy cycles.

**Impact:** Minimal — runs once per user click, not per frame.

**Remediation:** `String::with_capacity(estimated_total_size)` before the loop.

#### 16. Format Calls in Chart Headers
**File:** `src/app.rs` (chart_vix, chart_correlation headers)

`format!()` calls for date range and summary labels run every frame (once per chart, not in loops). ~180 small allocations/sec at 60fps.

**Impact:** Negligible individually but contributes to overall allocator noise.

### LOW

#### 17. AI Request String Clones
**File:** `src/ai.rs:20-23`

`run_analysis()` clones provider, model, system_prompt, and user_message to echo them back in `AiInferenceResult`. Total ~20-40KB per analysis. Acceptable for an infrequent user action.

#### 18. Inference Database Growth
**File:** `src/storage.rs`

No retention policy on `ai_inferences`. At ~25KB per row and daily usage, growth is ~9MB/year. Acceptable for years of use. A cleanup mechanism could be added if needed.

---

## Summary

| # | Finding | Category | Severity | Fix Priority |
|---|---------|----------|----------|-------------|
| 1 | SQL injection in LIKE clause | Security | CRITICAL | High — parameterize the query |
| 2 | API key in error messages | Security | HIGH | High — redact response bodies |
| 3 | Non-atomic .env write | Security | HIGH | Medium — write-then-rename |
| 4 | No LLM response size limit | Security | HIGH | Medium — cap at 100KB |
| 5 | No model name validation | Security | MEDIUM | Low — add character filter |
| 6 | No date range validation | Security | MEDIUM | Low — add bounds check |
| 7 | Unvalidated API response shape | Security | MEDIUM | Low — clearer error chain |
| 8 | Error path leakage | Security | LOW | Low |
| 9 | .env permissions | Security | LOW | Low — one line fix |
| 10 | Panic handling | Security | INFO | None needed (good) |
| 11 | TLS config | Security | INFO | None needed (good) |
| 12 | Date formatting in loop | Performance | CRITICAL | Medium — cache formatted strings |
| 13 | Missing ai_inferences index | Performance | HIGH | High — one SQL line |
| 14 | Per-frame Vec::collect | Performance | HIGH | Low — acceptable at scale |
| 15 | String growth in prompt assembly | Performance | MEDIUM | Low — with_capacity() |
| 16 | Per-frame format!() in headers | Performance | MEDIUM | Low |
| 17 | AI request clones | Performance | LOW | None needed |
| 18 | Inference DB growth | Performance | LOW | None needed |

### Immediate Action Items
1. **Parameterize `load_knowledge_chunks` SQL** — eliminates the only injection vector
2. **Add `CREATE INDEX` on `ai_inferences(created_at)`** — one line, prevents future query degradation
3. **Redact error response bodies in `send_and_parse()`** — prevents potential key leakage

### Overall Assessment
The codebase is well-structured for a PoC. Rust's type system and ownership model eliminate entire classes of memory safety bugs. The critical security finding (#1) is latent rather than exploitable today. Performance is solid at current scale; the hot-path allocation patterns are typical of egui apps and only matter at high frame rates or large datasets.
