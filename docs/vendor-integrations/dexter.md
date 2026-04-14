Q: I want to embed this app into my rust egui app: https://github.com/virattt/dexter

Now I have a clear picture of the internals. Let me give you a straight assessment.

## What Dexter actually consists of

It's built with TypeScript, Ink (React for CLI), and LangChain. The core pieces are:

- **ReAct agent loop** — plan → tool call → scratchpad → validate → repeat, iterating until the LLM determines it has sufficient data to answer
- **LangChain** — for structured tool schemas, LLM abstraction, prompt management
- **Ink** — React-based terminal rendering (the pretty UI)
- **Tools** — financial_metrics, read_filings, web_search, browser (Playwright-based scraping), and the skill invocation system
- **Scratchpad** — working memory with context compaction to handle large data volumes
- **Event system** — typed events (tool_start, tool_end, thinking, answer_start, done) yielded for real-time UI updates

---

## The honest port assessment

### Easy (you'd barely notice the translation)
- The agent loop itself — pure logic, Rust eats this for breakfast
- All the API-based tools: financial datasets, SEC filings, Exa/Tavily web search — all just `reqwest` + `serde_json`
- The scratchpad/memory — plain structs
- The event system — `tokio` channels are actually *better* for this than JS async generators
- Multi-provider LLM calls — use [`async-openai`](https://github.com/64bit/async-openai) for OpenAI; raw `reqwest` for the rest

### Moderate (real work but solved problems)
- **LangChain's tool schema/executor pattern** — use [`rig`](https://github.com/0xPlaygrounds/rig), which is the closest Rust equivalent for agentic workflows. Not a 1:1 match but covers 80% of what Dexter uses LangChain for
- **Structured tool call parsing** — JSON schema serialization with `schemars` + `serde` is verbose but reliable
- **The Skills system** (SKILL.md frontmatter parsing) — a few hours with `gray_matter` or a simple parser

### Hard (the real blocker)
- **The Playwright browser tool** — Playwright-based web scraping for reading pages the agent discovers. Rust's `chromiumoxide` crate exists but is significantly less mature. If you can live without browser scraping (the financial data APIs cover most of what you'd need), this drops off the list entirely

### The terminal widget for egui
Not a blocker at all. You'd build a `DexterPanel` struct that holds a `Vec<AgentEvent>` and renders each event as a colored line based on type. Roughly 200–300 lines of egui code. `egui` has `RichText` with color support and a `ScrollArea` — that's all you need to replicate the Ink look.

---

Q: Can we assess exactly how much web scraping is done and how important that scraping feature is for arbitrary pages? Do we know? Can we tell?

## The bottom line

If you're comfortable in Rust and can drop the browser tool (or stub it), this is a **2–3 week solo port** to functional parity. The agent logic and all financial API tools are genuinely straightforward. The LangChain abstraction layer is the most labor-intensive piece to replace, but `rig` removes most of that pain.

The browser tool is the honest wildcard. If Dexter's value for you is the financial data path (income statements, balance sheets, SEC filings, web search), that's all clean HTTP calls and you're fine. If you need full web scraping of arbitrary pages, budget extra time or keep a small Bun subprocess just for that one tool.

The egui terminal widget is the *easiest* part of the whole exercise.

## What we know for certain

**It's a recent addition.** The "initial browser implementation with Playwright for web scraping and page interaction" landed in v2026.2.5 — that's February 2026. It's not a foundational feature; it was bolted on two months ago.

**It's conditionally compiled into the tool registry.** Tools are conditionally included based on env vars. Dexter will run fine without it — the tool simply won't be offered to the LLM if the condition isn't met.

**It's a second-step tool, not a primary one.** The tool's described role is Playwright-based web scraping for reading pages the agent discovers — meaning the agent uses `web_search` (Exa/Tavily) to find URLs first, *then* optionally follows them with the browser tool. It's not the entry point to anything.

**The system prompt actively suppresses the web search path.** The LLM is instructed: ALWAYS prefer `financial_search` over `web_search` for financial data — call `financial_search` ONCE with a full natural language query, do NOT break queries into multiple tool calls when one can handle it. So for the core financial research use case, the primary path is `financial_search → financial_metrics → SEC filings`, all of which are clean HTTP API calls.

## What we can infer

The browser tool exists for one specific scenario: the agent has done a web search, found an article or filing on a non-API site, and wants to read the full page. Think: an analyst report on a broker site, a press release, a news article that Exa surfaced.

For that use case, the Playwright overhead buys you JS rendering — SPAs, dynamically loaded content. But for most financial pages (investor relations sites, news sites, SEC EDGAR), a plain HTTP GET and HTML parsing is sufficient. In Rust that's `reqwest` + the `scraper` crate. Two dependencies, zero headless browser.

## The honest bottom line

The browser tool is **optional**, **new**, **narrow in scope**, and **partially replaceable** with plain HTTP in the vast majority of cases. The only scenario where you'd genuinely feel its absence is if the agent discovers a modern SPA with client-side-only rendering — which is unusual for financial source material.

For your Rust port: stub it as plain HTTP fetch + HTML text extraction on day one. Ship. If you hit a real case where a JS-rendered page is blocking a useful query, that's the moment to decide whether it's worth adding `chromiumoxide`. My bet is that moment doesn't arrive often enough to block anything.

The browser tool is not a blocker. It's a nice-to-have that arrived two months ago.


