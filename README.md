<p align="center">
  <img src="artwork/hedgehog-mascot-white-bground.png" alt="The Hedgehog mascot" width="260">
</p>

<h1 align="center">The Hedgehog</h1>

<p align="center">
  A desktop dashboard for monitoring how commodities and risk assets behave during periods of elevated market volatility. Built with Rust and egui.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-000000?style=flat&logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/status-Preview%200.1-60a5fa?style=flat" alt="Preview 0.1">
</p>

> *"The fox knows many things, but the hedgehog knows one big thing."*
> &mdash; Archilochus, via Isaiah Berlin

## Why the hedgehog?

Isaiah Berlin opens his 1953 essay *The Hedgehog and the Fox* with that line from the Greek poet Archilochus. Foxes, Berlin argued, see the world in its messy particulars and pursue many ends at once; hedgehogs see it through a single organising idea and subordinate everything else to it.

**This app is a hedgehog.** Its one big thing is **causal, probabilistic modelling of capital-markets regimes** &mdash; marrying generative AI, [51Folds](https://51folds.ai) Bayesian causal networks, and real-time market data through a single prism. Everything else the app does (the VIX monitor, the commodity overlays, the spike detector, the inference history, the report generator) exists to feed that one prism.

## What It Does

The Hedgehog tracks the CBOE Volatility Index (VIX) — the market's primary fear gauge — alongside commodities like gold, silver, crude oil, and Bitcoin. It answers one question: **when the VIX spikes, what happens to everything else?**

The dashboard provides:

- **Real-time VIX monitoring** with configurable alert thresholds (rolling percentile or fixed levels)
- **Normalized comparison charts** showing how commodities move relative to VIX during stress periods
- **AI-powered regime analysis** — send the current market state to Claude or GPT for interpretation against a built-in knowledge base of VIX/commodity relationships
- **51Folds Bayesian modelling** — turn the AI's regime hypothesis into a probabilistic model with causal drivers, outcome probabilities, and "what if" scenario testing
- **Tabbed central panel** — switch between market charts and the 51Folds model explorer
- **Inference persistence** — every AI analysis is automatically saved with timestamp and VIX context, linked to any 51Folds models created from it
- **Summary reports** — select a date range of saved analyses and generate a retrospective synthesis
- **Interactive crosshairs** with synced hover across charts
- **Drag-to-zoom** on any chart to focus on a specific date range
- **Price panel** — press [P] to open a quick-pick panel for any individual instrument's raw price chart
- **Spike episode detection** with clickable highlights
- **Live data** — FRED for VIX and Soybeans, Alpha Vantage for all other commodities (daily spot closes)
- **Daily cache** — same-day re-launches reuse stored data so the API quota only burns once per trading day

## 51Folds Integration

When the AI analysis produces a regime hypothesis, the app can submit it to [51Folds](https://51folds.ai) to create a Bayesian causal model. The 51Folds Rust SDK is vendored in this repository at `vendor/fiftyone-folds/` — no additional repositories need to be cloned.

After a model completes (~25-30 minutes for Advanced tier), the central panel switches to a model explorer with two tabs:

- **Outcome** — probability bars for each outcome, before/after deltas when re-evaluating, and a prose take-away summary
- **Drivers** — the causal drivers identified by the Bayesian network, each with a state selector, expandable justification, and context sections ("Why does this matter?", "What could shift?", "What should we monitor?")

Users can change driver states and click **Re-evaluate** to see how outcome probabilities shift — enabling "what if" scenario analysis against the regime hypothesis.

A 51Folds API key is required for this feature. The rest of the app works fully without one.

## Why VIX and Commodities?

When the VIX spikes, it signals institutional repricing of risk. This repricing cascades across asset classes — and commodities respond in characteristic ways:

- **Gold** tends to rise (safe haven behavior — Baur & Lucey, 2010)
- **Silver** is ambiguous — industrial demand weakens its safe-haven status vs gold
- **Crude Oil** depends on the shock type — demand crises push it down, supply crises push it up
- **Industrial Metals** (copper, aluminum) are pro-cyclical and tend to fall
- **Bitcoin** empirically behaves as a risk asset during VIX spikes

The app's built-in help system includes a full research summary with academic sources and historical episode analysis (2008 GFC, 2020 COVID, 2022 Ukraine).

## Download The Hedgehog

Pre-built binaries are published on the [GitHub Releases page](https://github.com/cyclomaticsegal/TheHedgehog/releases). You don't need Rust or any build tools — just download, extract, set up your API keys, and run.

| Platform | Download |
|---|---|
| macOS — Apple Silicon (M1 / M2 / M3 / M4) | [`the-hedgehog-aarch64-apple-darwin.tar.xz`](https://github.com/cyclomaticsegal/TheHedgehog/releases/latest/download/the-hedgehog-aarch64-apple-darwin.tar.xz) |
| macOS — Intel | [`the-hedgehog-x86_64-apple-darwin.tar.xz`](https://github.com/cyclomaticsegal/TheHedgehog/releases/latest/download/the-hedgehog-x86_64-apple-darwin.tar.xz) |
| Linux — x86_64 (glibc) | [`the-hedgehog-x86_64-unknown-linux-gnu.tar.xz`](https://github.com/cyclomaticsegal/TheHedgehog/releases/latest/download/the-hedgehog-x86_64-unknown-linux-gnu.tar.xz) |
| Windows — x86_64 | [`the-hedgehog-x86_64-pc-windows-msvc.zip`](https://github.com/cyclomaticsegal/TheHedgehog/releases/latest/download/the-hedgehog-x86_64-pc-windows-msvc.zip) |

Each archive contains the binary, an `.env.example` template for your API keys, an `INSTALL.txt` with first-run instructions, and a `README.md`.

> **Heads up — these are unsigned preview builds.** macOS will block the binary on first launch unless you right-click → Open. Windows SmartScreen will show "unrecognized publisher" — click "More info" then "Run anyway". Both are one-time prompts. Full instructions are in `INSTALL.txt` inside each archive.

**Not currently supported:** Linux on ARM (Raspberry Pi, AWS Graviton), Linux distributions using musl (Alpine, Void), and 32-bit anything. If you need one of those, build from source using the steps below.

## Build from source — new to this? Start here

This is a **desktop app** that runs on your computer — not a website. The steps below will get it running from source even if you've never used a terminal before. The whole process takes about 10-15 minutes.

> If you just want to run the app and don't need to modify it, the **Download The Hedgehog** section above is much faster — grab a pre-built binary instead.

### Step 1 — Open a terminal

- **Mac:** Press `Cmd + Space`, type `Terminal`, hit Enter.
- **Windows:** Press `Win + R`, type `cmd`, hit Enter. (Or use Windows Terminal from the Microsoft Store — it's nicer.)

### Step 2 — Install Git

Git is a tool for downloading code. Check if you already have it:

```bash
git --version
```

If you see something like `git version 2.x.x`, skip to Step 3.

If not:
- **Mac:** Running the command above will prompt you to install it automatically. Click through the dialog.
- **Windows:** Download from [git-scm.com](https://git-scm.com/download/win) and run the installer with default settings.

### Step 3 — Install Rust

Rust is the programming language this app is written in. You need it to build the app from source.

Go to [rustup.rs](https://rustup.rs/) and follow the instructions for your operating system. On Mac/Linux it's one command you paste into your terminal. On Windows it's a small installer download.

Once done, **close your terminal and reopen it**, then verify:

```bash
rustc --version
```

You should see `rustc 1.x.x`.

### Step 4 — Download the code

```bash
git clone https://github.com/cyclomaticsegal/TheHedgehog.git
cd TheHedgehog
```

### Step 5 — Get your API keys

The app fetches financial data from free services. You'll need at least two keys:

**FRED** (required — provides VIX data):
1. Go to [fred.stlouisfed.org](https://fred.stlouisfed.org/)
2. Create a free account
3. Go to **My Account > API Keys > Request API Key**
4. Copy the key — it looks like `abcdef1234567890abcdef1234567890`

**Alpha Vantage** (required for commodity data — 25 free requests/day):
1. Go to [alphavantage.co/support/#api-key](https://www.alphavantage.co/support/#api-key)
2. Enter an email address — your free key is shown immediately
3. Copy the key

**51Folds** (optional — enables Bayesian modelling):
1. Go to [51folds.ai](https://51folds.ai)
2. Create an account and generate a service key (starts with `at_sk_`)
3. The AI analysis and all charts work without this key

### Step 6 — Configure your keys

```bash
cp .env.example .env
```

Open `.env` with any text editor and replace the placeholder values with your actual keys.

On Mac: `open -e .env`
On Windows: `notepad .env`

> **Optional:** To use the AI Analysis feature, add an `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` line to the same `.env` file. You only need one.

### Step 7 — Build and run

The **first time** this can take 2-5 minutes — Rust is downloading and building all its dependencies (including the vendored 51Folds SDK). Subsequent runs are instant.

```bash
cargo run --release
```

A window will appear — that's the app.

> **If something goes wrong:** The error message in the terminal is usually descriptive. Copy it and ask an AI assistant (Claude, ChatGPT, etc.) to explain it — they're good at diagnosing Rust build errors.

### Step 8 — First use

1. In the left sidebar, go to **Data Source** and confirm both keys show "set"
2. Click **Refresh** in the top bar and watch the activity log (auto-refresh runs on startup by default)
3. Subsequent same-day launches reuse the cache — no API calls until tomorrow's close

---

## Getting Started (Quick Reference)

### Prerequisites

- [Rust](https://rustup.rs/) (stable toolchain)
- API keys (both free tiers):
  - **FRED** (required for VIX, Soybeans) — [fred.stlouisfed.org](https://fred.stlouisfed.org/) > Account > API Keys
  - **Alpha Vantage** (required for all other commodities) — [alphavantage.co/support/#api-key](https://www.alphavantage.co/support/#api-key) (25 requests/day)
  - **51Folds** (optional) — [51folds.ai](https://51folds.ai) for Bayesian model creation

### Build and Run

```bash
git clone https://github.com/cyclomaticsegal/TheHedgehog.git
cd TheHedgehog
cp .env.example .env
# Edit .env with your keys
cargo run --release
```

All dependencies — including the 51Folds SDK — are either on crates.io or vendored in-tree. No additional repositories need to be cloned.

## Usage Guide

### Top Bar

| Control | Function |
|---------|----------|
| Charts / 51Folds | Switch the central panel between market charts and the model explorer |
| Outcome / Drivers | Sub-tabs within the 51Folds model explorer |
| 1M / 3M / 6M / 1Y / All | Time window selection (Charts view only) |
| Refresh | Fetch fresh data from all providers |
| Save | Persist settings and API keys |
| Report | Open the Summary Report window |
| Help | Open comprehensive documentation |

### Sidebar

- **VIX Status** — Current reading, alert level, and threshold values
- **Compare Against VIX** — Select which assets appear on the comparison chart
- **Recent Spikes** — Detected VIX spike episodes (click to highlight on chart)
- **Data Source** — API key entry with set/not-set badges and auto-refresh toggle
- **AI Analysis** — LLM provider/model selector, analyze button, and inference history
- **Thresholds** — Configure alert levels (percentile or fixed mode)
- **51Folds** — API key entry for Bayesian model creation

### Chart Interaction

- **Hover** over a chart to see a crosshair with date and values — the other chart syncs automatically
- **Drag horizontally** to zoom into a date range — both charts zoom together
- **Click a spike** in the sidebar to highlight its date range on the VIX chart
- **Press [P]** to open a quick-pick instrument selector for a raw price chart
- **Click chart headers** to collapse/expand individual chart panels

### 51Folds Model Workflow

1. Click **Analyze Current View** in the AI Analysis sidebar section
2. The LLM produces a regime classification and a structured hypothesis with outcomes
3. Review and optionally edit the hypothesis, or click **Different outcomes** for alternatives
4. Click **Create 51Folds Model** — the model builds in ~25-30 minutes
5. When complete, the central panel auto-switches to the **Outcome** tab showing probability bars
6. Switch to the **Drivers** tab to explore causal factors and run scenario analysis
7. Loading a saved inference from the sidebar history will restore its linked model if one exists

## Data Sources

| Provider | Instruments | Free Tier |
|----------|------------|-----------|
| FRED | VIX (VIXCLS), Soybeans (PSOYBUSDM) | Generous, no practical limit |
| Alpha Vantage | Gold, Silver, Bitcoin, Crude Oil (WTI), Natural Gas, Copper, Aluminum, Wheat, Corn | 25 requests/day, 1 req/sec |
| 51Folds | Bayesian causal models | Requires account and API key |

All market data is stored locally in SQLite. The daily cache means each instrument is fetched at most once per trading day. No data is sent to external services beyond API fetch requests (FRED, Alpha Vantage), optional LLM calls (Anthropic, OpenAI) when using AI Analysis, and optional 51Folds API calls when creating Bayesian models.

## Project Structure

```
src/
  main.rs          Entry point, window setup
  app.rs           UI layout, chart rendering, state management, model explorer
  models.rs        Data structures (instruments, settings, events)
  analysis.rs      VIX status, spike detection, percentile computation
  providers.rs     FRED + Alpha Vantage API integration
  ai.rs            LLM provider abstraction (Anthropic + OpenAI), hypothesis generation
  folds.rs         51Folds SDK bridge (async SDK <-> sync thread via tokio runtime)
  knowledge.rs     RAG knowledge base: VIX/commodity regime analysis chunks
  storage.rs       SQLite persistence layer
  help.rs          Built-in documentation (rendered markdown)

vendor/
  fiftyone-folds/  Vendored 51Folds Rust SDK (compiled from source during build)

docs/
  adr/             Architecture Decision Records
  specs/           Product specifications
```

## Architecture Decisions

The project maintains Architecture Decision Records in `docs/adr/`:

| ADR | Summary |
|-----|---------|
| 0001 | Pure Rust + egui with free daily data |
| 0002 | Grouped macro monitor and registry-driven instrument model |
| 0003 | Real-data-first mode and selector-driven overlays |
| 0004 | Performance caching, interactive charts, and multi-provider architecture |
| 0005 | RAG-powered AI analysis panel |
| 0006 | Inference persistence and summary reports |
| 0007 | Roll back tabbed workspace; focus integration on 51Folds |
| 0008 | 51Folds integration — hypothesis generation and model creation |
| 0009 | Strengthen hypothesis quality and fix OpenAI tool compatibility |
| 0010 | Multi-provider commodity caching (superseded by 0011) |
| 0011 | Single-provider daily cache |
| 0012 | Analysis quality hardening, persistent tracking, editor consolidation |
| 0013 | 51Folds Rust SDK integration, rich model explorer, tabbed central panel |
| 0014 | Model explorer navigation stack UI redesign |
| 0015 | Dark theme hardening and 51Folds model explorer UI polish |
| 0016 | Splash screen, revert architecture, and 51Folds PATCH/PUT drift (in limbo) |

## License

All rights reserved. No license is granted for use, modification, or distribution. The vendored 51Folds SDK (`vendor/fiftyone-folds/`) is a client library for the [51Folds](https://51folds.ai) commercial API service.

## References

- Baur, D. G. & Lucey, B. M. (2010). *Is Gold a Hedge or a Safe Haven?* Financial Review, 45, 217-229.
- Baur, D. G. & McDermott, T. K. (2010). *Is gold a safe haven? International evidence.* Journal of Banking & Finance.
- Cheng, I. & Xiong, W. (2014). *Financialization of Commodity Markets.* Annual Review of Financial Economics.
- [CBOE VIX Index](https://www.cboe.com/tradable_products/vix/)
- [FRED VIXCLS Series](https://fred.stlouisfed.org/series/VIXCLS)
- [World Gold Council Research](https://www.gold.org/goldhub/research)
- [CME Group Volatility Education](https://www.cmegroup.com/education/articles-and-reports/introduction-to-gold-volatility-trading.html)
- [BIS Working Papers — Volatility Risk Premia](https://www.bis.org/publ/work619.pdf)
