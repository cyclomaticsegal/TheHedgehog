# ADR-0005: RAG-Powered AI Analysis Panel

**Status:** Accepted  
**Date:** 2026-04-03  
**Branch:** `perf-imp`

---

## Context

The Regime Shift Dashboard displays VIX levels alongside commodity price movements, with static educational content in `src/help.rs` explaining known relationships (e.g. gold as safe haven during VIX spikes, oil falling in demand shocks). However, interpreting what the user is currently seeing on screen — "is this a demand shock or a supply shock, and how does it compare to 2008 or 2020?" — requires domain expertise that the static help system cannot provide dynamically.

The goal is to add an LLM-backed "Analyze" button that assembles the user's live app state (VIX level, alert status, selected instruments, percentage changes, detected spike episodes) and interprets it against a knowledge base of VIX/commodity regime behaviour.

---

## Decision

### LLM Provider Abstraction

Support both Anthropic (Claude) and OpenAI (GPT) via a `LlmProvider` enum with provider-specific API call implementations. Both use `reqwest::blocking::Client` in a spawned thread with `mpsc::channel` — the same pattern used for market data refresh. API keys are stored in `.env` (never serialized to SQLite), consistent with the existing `ApiKeys` security model.

**Why not a trait-based abstraction?** Two providers with known-at-compile-time endpoints. A `Box<dyn Provider>` would add heap allocation and vtable indirection for zero benefit. Adding a third provider means one new enum variant and one match arm.

### RAG Without Vector Embeddings

Knowledge is stored in a `knowledge_chunks` SQLite table (33 chunks, ~15K tokens total) and retrieved via simple `LIKE '%tag%'` SQL matching against instrument `storage_key()` values. The selected instruments on screen determine which chunks are retrieved — deterministic, not semantic.

**Why not embeddings?** The knowledge domain is bounded (11 instruments, 4 regime types, 3 historical episodes). The full knowledge base fits comfortably in any modern LLM context window. Embeddings would add: an embeddings API call, vector storage infrastructure, non-deterministic chunk selection, and a new dependency — all with zero practical benefit at this scale. The threshold for needing embeddings is ~100K+ tokens; this knowledge base will not approach that.

### Context Packet Assembly

Two functions in `src/ai.rs` construct the LLM input:
- `assemble_system_prompt()` — analyst persona + all retrieved knowledge chunks
- `assemble_user_message()` — structured market snapshot (VIX status, thresholds, 30-day % changes per instrument, recent spike episodes)

The system prompt carries the knowledge base; the user message carries the live state. This separation means the knowledge base is cached by providers that support prompt caching.

### UI Integration

- **Top bar**: "Analyze" button with spinner during in-flight calls
- **Sidebar**: "AI Analysis" collapsing section with provider selector, API key field, model name (editable, with "Default" reset), and "Analyze Current View" button
- **Bottom panel**: Resizable `TopBottomPanel` rendering the LLM response as markdown via `egui_commonmark`
- Switching provider auto-resets the model name to the new provider's default

---

## Files

| File | Change |
|------|--------|
| `src/ai.rs` | New: LLM abstraction, API calls, context assembly |
| `src/knowledge.rs` | New: 33 knowledge chunks, retrieval helper |
| `src/models.rs` | `LlmProvider`, `AiEvent`, AI fields on `ApiKeys` and `AppSettings` |
| `src/storage.rs` | `knowledge_chunks` table, seed/load methods |
| `src/app.rs` | AI fields on `DashboardApp`, `start_ai_analysis()`, `poll_ai()`, 3 UI touchpoints |
| `src/main.rs` | `mod ai; mod knowledge;` |

No new crate dependencies.

---

## Consequences

- The app can now provide contextual LLM commentary on live market data
- Knowledge base is seeded on first launch (idempotent — checks row count)
- Old `AppSettings` JSON blobs deserialize cleanly via `#[serde(default)]`
- LLM calls are non-blocking (spawned thread) and handle panics gracefully
- The knowledge base can be expanded by adding entries to `KNOWLEDGE_BASE` in `knowledge.rs`
