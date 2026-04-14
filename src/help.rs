pub const HELP_TEXT: &str = r##"## Why the hedgehog?

> *"The fox knows many things, but the hedgehog knows one big thing."*
> — Archilochus, via Isaiah Berlin

Isaiah Berlin opens his 1953 essay *The Hedgehog and the Fox* with that line from the Greek poet Archilochus. Foxes see the world in its messy particulars and pursue many ends at once; hedgehogs see it through a single organising idea and subordinate everything else to it.

**This app is a hedgehog.** Its one big thing is **causal, probabilistic modelling of capital-markets regimes** — marrying generative AI, 51Folds Bayesian causal networks, and real-time market data through a single prism. Everything else the app does (the VIX monitor, the commodity overlays, the spike detector, the inference history, the report generator) exists to feed that one prism.

---

## About

The Hedgehog is a desktop tool for monitoring how commodities and risk assets behave during periods of elevated market volatility. Its core purpose is to help you observe, in real time and historically, how assets like gold, silver, crude oil, and Bitcoin respond when the VIX — the market's primary fear gauge — spikes into extreme territory.

The dashboard answers one central question: **when the VIX moves sharply, what happens to everything else?**

Beyond visualization, the dashboard includes an AI analysis engine that can interpret the current market regime using an LLM (Claude or GPT), drawing on a built-in knowledge base of VIX/commodity relationships. Every analysis is saved automatically, and you can generate summary reports across time periods to track how regimes evolve.

---

## Why the VIX Matters for Commodities

The CBOE Volatility Index (VIX) measures the market's expectation of 30-day forward volatility, derived from S&P 500 index option prices. It is widely regarded as the leading barometer of investor fear and market stress.

When the VIX spikes, it signals that institutional investors are rapidly repricing risk. This repricing cascades across asset classes — and commodities respond in characteristic but distinct ways:

- **Gold** tends to rise during VIX spikes, acting as a safe haven. Investors rotate into gold as a store of value when equity markets are under stress. Research by Baur & Lucey (2010) established that gold is both a hedge against stocks on average and a safe haven during extreme market conditions — though the safe-haven property is typically short-lived (approximately 15 trading days).

- **Silver** has a more ambiguous relationship. Its dual nature as both an industrial metal and a monetary metal means it can sell off initially (industrial demand destruction) before recovering as monetary demand takes over. The gold/silver ratio typically widens during crises.

- **Crude Oil** reacts depending on the *type* of shock. Demand-driven crises (recessions, pandemics) push oil down sharply as growth expectations collapse. Supply-driven crises (geopolitical conflict) can push both VIX and oil higher simultaneously.

- **Industrial Metals** (copper, aluminum) are pro-cyclical and tend to fall during VIX spikes, as they are proxies for global manufacturing and construction activity.

- **Agricultural Commodities** (wheat, corn, soybeans) have weaker direct correlation with the VIX. They are driven more by weather, planting cycles, and trade policy. However, dollar strength during VIX spikes (flight-to-safety into USD) can pressure dollar-denominated commodity prices.

- **Bitcoin** was initially positioned as an uncorrelated safe-haven asset, but empirical evidence shows it behaves primarily as a risk asset during VIX spikes — selling off alongside equities in March 2020 and throughout 2022.

### Key Historical Episodes

- **2008 Global Financial Crisis**: VIX reached ~80. Gold initially dipped (margin calls forced liquidation) then rallied strongly. Crude oil collapsed from $147 to $32. Copper fell ~65%.
- **2020 COVID Crash**: VIX hit ~82. Gold dipped briefly then reached all-time highs. WTI crude oil futures briefly went negative. Bitcoin crashed 50%+ in March before recovering.
- **2022 Ukraine / Inflation**: VIX spiked to ~36. Energy, wheat, and nickel surged on supply disruption fears. Crypto sold off substantially.

### Further Reading

- Baur, D. G. & Lucey, B. M. (2010). *Is Gold a Hedge or a Safe Haven? An Analysis of Stocks, Bonds and Gold.* Financial Review, 45, 217-229.
- Baur, D. G. & McDermott, T. K. (2010). *Is gold a safe haven? International evidence.* Journal of Banking & Finance.
- Cheng, I. & Xiong, W. (2014). *Financialization of Commodity Markets.* Annual Review of Financial Economics.
- Silvennoinen, A. & Thorp, S. (2013). *Financialization, crisis and commodity correlation dynamics.* Journal of International Financial Markets.
- CBOE VIX Index methodology and data: cboe.com/tradable_products/vix
- FRED VIX daily close series: fred.stlouisfed.org/series/VIXCLS
- World Gold Council research: gold.org/goldhub/research
- CME Group volatility education: cmegroup.com/education
- BIS Working Papers on volatility risk premia: bis.org/publ/work619.pdf

---

## Getting Started

### API Keys

The dashboard fetches live market data from two free API services:

1. **FRED** (required for VIX) — Register at fred.stlouisfed.org, then find your API key under Account > API Keys. Free, generous rate limits.
2. **Alpha Vantage** (required for all commodities) — Get a free key at alphavantage.co/support/#api-key. Free tier: 25 requests/day, 1 request/second.

Keys can be entered in the sidebar under **Data Source**, or stored in a `.env` file in the app directory:

```
FRED_API_KEY=your_key_here
ALPHA_VANTAGE_API_KEY=your_key_here
ANTHROPIC_API_KEY=your_key_here
OPENAI_API_KEY=your_key_here
```

The last two keys are optional — they enable the AI Analysis feature (see below). You only need one of them, depending on which LLM provider you prefer.

Keys are loaded from `.env` on startup. You can also enter or update them in the sidebar and click **Save** — this writes them back to your `.env` file. Keys are never stored in the database.

### First Refresh

1. Enter your API keys in the sidebar under **Data Source** (or add them to `.env` before launching)
2. Click **Refresh** in the top bar (or rely on auto-refresh on startup — the cache will skip API calls for instruments whose latest stored close is already today's)
3. Watch the Activity Log at the bottom for per-instrument progress

---

## Dashboard Layout

### Top Bar

- **Time Window Buttons** (1M, 3M, 6M, 1Y, All) — Select how much history to display. Clicking any button clears a custom zoom if one is active.
- **Zoom Indicator** — When you have dragged to zoom on a chart, shows the zoom date range with a Reset button.
- **Refresh** — Fetches fresh data from FRED (VIX) and Alpha Vantage (all commodities). The daily-cache check skips API calls for instruments whose latest stored close is already today's, so back-to-back refreshes within a trading day are free.
- **Save** — Writes API keys back to your `.env` file and persists all other settings (thresholds, overlay choices, AI model preferences) to the local database. Settings survive app restarts.
- **Report** — Opens the Summary Report window for generating retrospective analyses across time periods. See the **Summary Reports** section below.
- **Help** — Opens this help window.
- **Status Line** — Shows the result of the last action (refresh status, save confirmation, errors). When the AI Analysis or Activity panels are hidden, small reopen buttons appear at the right edge of the status line.

### Left Sidebar

#### VIX Status
Shows the current VIX reading, alert level, date, and threshold values. The colored indicator shows:
- **Green (Normal)** — VIX is below the approaching threshold
- **Amber (Approaching Extreme)** — VIX is between the approaching and extreme thresholds
- **Red (EXTREME)** — VIX is above the extreme threshold

#### Overlay on VIX
Select which assets appear on the comparison chart. Each instrument has a colored swatch showing its chart color (dimmed when deselected, bright when selected). Quick-select buttons: Core 3, Energy, Metals, All, Clear.

#### Recent Spikes
Lists detected VIX spike episodes with severity indicators (amber or red circles). **Click a spike** to highlight its date range on the VIX chart. Click again to deselect.

#### Data Source
- **API Keys** — Enter keys for FRED and Alpha Vantage. Each field shows a "● set" / "○ not set" badge so you can see at a glance which providers are configured. Keys are masked as password fields.
- **Auto-refresh on startup** — When checked (default), the app fetches fresh data immediately on launch. The daily cache makes subsequent same-day launches free.

#### AI Analysis
- **Provider** — Switch between Claude (Anthropic) and GPT (OpenAI). Each provider remembers its own model name independently, so switching back and forth preserves your model choice for each.
- **API Key** — Shows the key field for whichever provider is currently selected. Keys are stored in `.env`, not the database.
- **Model** — The model name sent to the API (e.g. `claude-sonnet-4-6`, `gpt-4.1`). Editable for power users who want to pin a specific version. Click **Default** to reset to the provider's recommended model.
- **Analyze Current View** — Sends the current VIX status, selected instruments, 30-day price changes, and detected spike episodes to the LLM along with the built-in knowledge base. The response appears in the AI Analysis panel at the bottom.
- **History** — Below the button, a list of recent analyses shows timestamp, VIX alert level (colored dot), and a preview of the response. Click any entry to reload its full response into the panel.

#### Thresholds
Configure how VIX alert levels are determined. There are two modes:

**Fixed mode** — You set two hard numbers that never change.
- *Approaching* (e.g. 25): "start paying attention if VIX goes above this"
- *Extreme* (e.g. 35): "this is serious"

Simple and predictable, but the same numbers apply forever regardless of what the market has been doing lately.

**Rolling Percentile mode (default)** — Instead of fixed numbers, the app looks at the last *N* trading days of VIX history and asks: where does today's reading rank? A reading at the 95th percentile means today's VIX is higher than 95% of days in the lookback window.

This adapts automatically to the current market regime. A VIX of 20 might be alarming during a calm bull market but completely ordinary during a volatile period. Percentile mode moves with the market's recent behaviour rather than staying anchored to a number set years ago.

Think of it this way: Fixed mode is like saying "call me if it's above 35°C." Percentile mode is like saying "call me if today is hotter than 95% of days this past year."

- **Lookback** — Number of trading days used for the rolling percentile window (default 252, roughly one year).

---

## Charts

### VIX Index Chart (Top)
Displays the VIX time series with colored threshold bands:
- **Green zone** — Normal (below approaching threshold)
- **Amber zone** — Between approaching and extreme thresholds
- **Red zone** — Above extreme threshold

Threshold lines are labeled with their current values. A date range subtitle shows the period covered.

### Asset Performance vs VIX Chart
Displays selected commodities as percentage change from the start of the visible window. The y-axis shows relative change: +20% means the asset is up 20% from the window start, -10% means down 10%.

VIX is not shown on this chart (it is fully visible above). Instead, hover values show each asset's performance **relative to VIX** — telling you whether an asset is outperforming or underperforming VIX at each point.

### Price Panel
Press **[P]** to open a quick-pick instrument selector. Type to filter, use arrow keys to navigate, and press Enter to select. The selected instrument gets its own raw price chart below the comparison chart. Press **[P]** again to close it.

### Collapsible Panels
Each chart section has a collapse/expand toggle in its header. Click the header to collapse a chart you don't need, freeing vertical space for the others. Collapse state is session-only and resets on restart.

---

## Chart Interaction

### Hover Crosshair
Move your mouse over either chart to see:
- A vertical crosshair line at the cursor position
- The exact date at the crosshair
- Interpolated values for each visible series at that date

On the VIX chart, values show absolute VIX levels. On the comparison chart, values show percentage change and the spread vs VIX (e.g., "Gold: +5.2% (+3.1 vs VIX)").

### Synced Crosshairs
Hovering over one chart automatically shows a matching crosshair on the other chart at the same date. This lets you see the VIX level and commodity responses simultaneously without switching focus.

### Tooltip Positioning
When the crosshair approaches the right edge of a chart, tooltip text automatically flips to the left side of the crosshair to remain fully visible.

### Drag to Zoom
Click and drag horizontally on either chart to select a time range:
1. A blue selection overlay appears while dragging
2. On release, both charts zoom to the selected range
3. The top bar shows a zoom indicator with the selected dates
4. Click **Reset** or any time window button (1M, 3M, etc.) to return to normal view

If you zoom into a VIX period that has no commodity data, the comparison chart displays "No commodity data available for this period."

A minimum drag of 3 days is required to avoid accidental zoom from clicks.

---

## Activity Log

The bottom panel shows real-time progress during data refresh:
- Each instrument gets its own row with a colored swatch matching its chart color
- Point counts are shown for successfully fetched instruments
- Error messages are shown in red for failed instruments

The log appears automatically when a refresh starts. Dismiss it with the × button — this hides the panel but preserves the log content. To reopen it, click the **Activity** button that appears in the status bar. A new refresh also reopens the panel automatically.

Alpha Vantage requests include a 1.5-second delay between calls to respect the free-tier rate limit (1 request/second).

---

## AI Analysis

The dashboard can send the current market state to an LLM (Claude or GPT) for regime identification, interpretation, and **hypothesis generation**. When you click **Analyze Current View** in the sidebar, the app assembles:

- The current VIX level, alert status, and thresholds
- Which instruments you have selected for comparison
- Each instrument's 30-day price change
- Any recent VIX spike episodes detected

This snapshot is sent alongside a built-in knowledge base covering VIX mechanics, regime taxonomy (demand shock, supply shock, financial contagion, geopolitical spike), historical episodes, and per-instrument behaviour patterns. The LLM uses this context to:

1. Identify the likely current market regime and highlight notable signals
2. Propose a **hypothesis** — a specific, time-bounded, mechanism-named claim about where a chosen asset is going over the next 7–90 days
3. Suggest 2–4 mutually exclusive outcomes for that hypothesis
4. List driver context the downstream 51Folds model will use

Everything after step 1 feeds directly into the 51Folds Model Explorer (see the next section).

### How it works
The analysis runs in a background thread — the dashboard remains responsive while waiting for the response. Results are rendered as formatted markdown in the AI Analysis panel on the right.

### Automatic persistence
Every analysis response is automatically saved to the local database with a timestamp, the VIX reading at the time, and the full prompt context. You never need to manually save an analysis. Past analyses appear in the **History** list in the sidebar and can be reloaded by clicking on them.

### Panel visibility
Close the AI panel with the × button. To reopen it, click the **AI** button in the status bar, click any history entry in the sidebar, or run a new analysis.

---

## Commodity-Bias Validation

Every AI analysis is automatically validated for commodity bias — both before the prompt is sent and after the response is received. The results appear in a **Bias Check** bar between the analysis response and the 51Folds section.

### Two layers

**Structural checks (instant, deterministic)** — run the moment the response arrives. No LLM call needed. These verify:
- The hypothesis names at least one of the instruments the user selected (not a different commodity)
- No unselected instrument appears as the grammatical subject of the hypothesis
- Dollar amounts in the hypothesis are anchored to the latest closing prices, not stale training-data figures (e.g. citing gold at $2,000 when the latest close is $4,624)

**Semantic checks (async, LLM judge)** — a second, focused LLM call that validates the analysis against five rules:
- **SUBJECT_MATCH** — is the hypothesis actually about the selected instruments?
- **PRICE_ANCHORING** — are cited prices consistent with the latest closes?
- **MECHANISM_RELEVANCE** — is the causal mechanism specific to the selected instruments, or generic filler?
- **TERTIARY_BOUNDARY** — are non-selected instruments kept as background context, not subjects?
- **OUTCOME_ALIGNMENT** — do outcome bands reference the right instruments with distinct causal paths?

### Cross-model validation

When you have API keys for both Claude and GPT, the bias judge automatically uses the **other** provider — the one that did NOT produce the analysis. If Claude wrote the analysis, GPT judges it (and vice versa). This gives you independent cross-model validation for free, with no extra configuration.

If only one key is present, the judge uses the same provider. This is still useful because the judge prompt is a validation task (rule-based checklist), which is structurally different from the creative generation task.

### Reading the Bias Check bar

The bar shows two counts:
- **Structural** — e.g. "3/3 structural" (green if all pass, red if any fail)
- **Semantic** — e.g. "5/5 semantic (GPT)" (shows which provider judged; spinner while in flight)

If any rule fails, the failing rule name and reason are listed below the bar.

---

## Research Agent

The **Research** tab in the central panel provides an embedded terminal running the Dexter financial research agent. Dexter is a conversational AI agent with access to financial data tools (market data, filings, earnings, stock screening), web search, and a persistent memory system.

### Getting started

1. Install [Bun](https://bun.sh) (the JavaScript runtime Dexter runs on) — the app checks for it automatically
2. Make sure you have at least one LLM API key configured (Anthropic or OpenAI)
3. Click the **Research** tab in the central panel

The agent launches in an embedded terminal. You can interact with it directly — ask questions, request research, and it will use its tools to find answers.

### Provider selection

The Research Agent uses the provider configured in the sidebar under **Settings > Research Agent**. This is independent of the AI Analysis provider, so you can run analysis on Claude while the research agent uses GPT (or vice versa).

### /51folds command

After doing research in the terminal, type `/51folds` to synthesize the conversation into a structured 51Folds hypothesis. The agent takes the full conversation context, generates a hypothesis with outcomes and driver context, and writes it directly to the Hedgehog's database. The sidebar populates automatically — no manual copy-paste needed.

You can add a directional prompt to steer the hypothesis focus:

- `/51folds` — synthesize from the full conversation, agent decides the angle
- `/51folds focus on the bear case around margin compression` — conversation context plus your specific direction shapes the hypothesis

The hypothesis appears in the sidebar under **Research Analysis** and is ready for 51Folds model creation, the same as an AI Analysis hypothesis. The sidebar header changes between "AI Analysis" and "Research Analysis" depending on the source.

---

## 51Folds Model Explorer

After an AI Analysis or Research Agent session produces a hypothesis, the app can submit it to **51Folds** — a Bayesian modelling service that builds a causal-driver model and returns probability-weighted outcomes plus a full driver graph. The Hedgehog ships with the 51Folds Rust SDK integrated directly: enter your `FOLDS_API_KEY` in `.env`, run an analysis (or use `/51folds` in the Research Agent), and the hypothesis editor appears in the sidebar.

### Creating a model

**From AI Analysis:**
1. Click **Analyze** on the VIX status banner or in the sidebar
2. The right-hand panel shows the regime assessment and a collapsible hypothesis editor
3. Review and edit the hypothesis, outcomes, and driver context fields
4. Click **Create 51Folds Model**

**From Research Agent:**
1. Do your research in the Dexter terminal
2. Type `/51folds` (optionally with a directional prompt)
3. The hypothesis populates the sidebar automatically
4. Click **Create 51Folds Model**

In both cases, the panel shows "Model ID: X — building…" while the model is under construction (typically 25–30 minutes for the Advanced tier). When the model completes, the sidebar summary switches to green **Model ID: X — complete** with the outcome probabilities listed below, and the central panel auto-switches from Charts to **51Folds**.

Models persist across restarts. If you close the app while a model is building, the resume sweep will re-attach the polling thread on next launch and update the sidebar once the SDK reports completion (or gives up after 35 minutes).

### Central panel views

The central panel has several top-level views selectable from the toolbar:

- **Charts** — VIX time series, asset-performance-vs-VIX comparison chart, optional price panel (press `[P]`). This is the default view.
- **51Folds** — the Model Explorer. Disabled (greyed) until a model has been completed for the current inference. The label turns blue once a model is available.
- **Research Agent** — embedded Dexter terminal for conversational financial research.
- **Report** — Summary Report view for loading inference histories and generating retrospective reports.

Inside the 51Folds tab there are three sub-views selectable from the sub-toolbar: **Outcome**, **Drivers**, and **Visual Map**.

### Outcome view

Shows the model's answer in card form:

- The **question / hypothesis** as the page heading
- An **Outcome probabilities** card with one row per outcome: label on the left, proportional blue bar, percentage on the right
- A **Take away** card — a plain-English summary from the model
- A **Show me the drivers ›** call-to-action that jumps to the Drivers view

After you re-evaluate the model (see below), each outcome also shows a delta annotation (`↑ up from X%` / `↓ down from X%`) in green / red so you can see how your driver edits shifted the probabilities.

### Drivers view

Shows every causal driver the model is tracking as a dark card with:

- The driver's name and short code, e.g. `Real Interest Rates (RIR)`
- A row of **state pills** — Negligent / Low / Medium / High / Extreme (or whatever custom states the model defines). The currently-selected state is highlighted in blue.
- A **Details ›** button on the right that navigates to the full driver detail page

Below the driver list:
- **Reset** — reverts every driver back to the model's original selected states
- **Re-evaluate** — sends your current driver edits to 51Folds and returns updated outcome probabilities. The Outcome view will show deltas after this completes.

Click a pill to change a driver's state. The driver name turns amber when you've diverged from the original. Re-evaluate applies all edits in a single patch call.

### Driver detail pages

Clicking **Details ›** on a driver row opens a dedicated page:

- Back button in the top-left (`‹ Drivers`)
- Driver name as the page heading
- **Current state** card showing the selected state name and its description
- **All states** card listing every possible state with its description, current state highlighted in blue
- A **Related** list with four navigable rows:
  - **Why was [original state] selected?** — the model's justification for the initial assignment, with numbered citations to source URLs
  - **Why does this matter?** — the driver's importance to the hypothesis
  - **What could shift?** — what market conditions would push the driver into a different state
  - **What should we monitor?** — observable indicators that track this driver

Each Related row opens a full-page content screen with its own back button. Citations appear in a **Sources** card with clickable URLs.

### Sidebar summary

While a model is building or complete, the right-hand AI panel shows a compact 51Folds summary underneath the regime text:

- **Model ID: X — complete** (green, when finished)
- Each outcome on its own line: percentage followed by the outcome label
- A **View in 51Folds tab** button that jumps to the central-panel Outcome view

If the model is still building, the spinner + "Model ID: X — building…" message is shown instead. If an error occurs, the red error text replaces both.

### Navigating back

Every detail / section page has a back button in the top-left. The navigation stack is:

```
Outcome ────────────────┐
                        │
Drivers ─→ DriverDetail ─→ DriverSection
  ↑            ↑              │
  └────────────┴──────────────┘
         Back buttons
```

### Visual Map

The Visual Map tab shows an interactive directed acyclic graph (DAG) of the model's causal network, rendered via D3.js and dagre in an embedded WebView.

- **Nodes** — each driver appears as a gold circle with its abbreviated code. The Outcome node is red and sits at the bottom of the graph.
- **Edges** — straight lines showing causal relationships between drivers, converging toward the Outcome node.
- **Hover** — hovering over a node highlights its direct connections in blue. The path from that node to the Outcome highlights in red. All unconnected nodes and edges dim.
- **Click** — clicking a node navigates to its driver detail page. A back button labelled "Visual Map" returns you to the graph.
- **Pan and zoom** — drag to pan, scroll to zoom.

The Visual Map updates automatically when driver states change (pill edits on the Drivers tab) or after a re-evaluation completes.

The Outcome, Drivers, and Visual Map views are reachable from the sub-toolbar at any time.

---

## Summary Reports

The **Report** button in the toolbar switches the central panel to the Summary Report view for generating retrospective summaries across multiple saved analyses. This includes both AI Analysis and Research Agent inferences.

### Workflow
1. Set a date range using the **From** / **To** fields, or click a preset button (Last 7 days, Last 30 days, Last 90 days, All)
2. Click **Load Inferences** to fetch all saved analyses in that range
3. Browse the loaded list — each entry shows its timestamp, source (Analysis or Report), VIX reading, 51Folds model badge, and hypothesis preview. Hover for full text; click to load the inference into the sidebar.
4. Click **Generate Report** to send all loaded analyses to the LLM for synthesis
5. Click **Load Inferences** again to clear a generated report and return to the inference list

### What the report covers
The LLM receives every analysis from the selected period and produces a structured report with:
- **Executive Summary** — the headline regime assessment
- **Period Overview** — what happened across the timeframe
- **Key Themes** — dominant patterns and transitions
- **Historical Context** — comparison to known historical precedents
- **Outlook** — forward-looking assessment based on the trajectory

### Report persistence
Generated reports are themselves saved to the database, so they appear in future inference lists and can be included in subsequent reports. Reports are tagged distinctly from individual analyses.

---

## Data Sources

### FRED (Federal Reserve Economic Data)
Provides the VIX daily close series (VIXCLS). Free API with generous rate limits.

### Alpha Vantage
Provides daily spot prices for all commodities — Gold, Silver, Crude Oil (WTI), Natural Gas, Copper, Aluminum, Wheat, Corn, Soybeans — and Bitcoin via the digital-currency endpoint. Free tier: 25 requests/day, 1 request/second.

The Hedgehog uses spot prices rather than ETF proxies because regime-shift analysis is sensitive to absolute price levels and percentile thresholds, and ETF tracking error / contango drift would muddy the signal.

---

## Data Storage

All data is stored locally in a SQLite database at `data/regime_shift_dashboard.sqlite3` in the app directory. This includes:
- All fetched observation data (prices by instrument and date)
- App settings (thresholds, provider selection, overlay choices, AI model preferences)
- Alert events (VIX level transitions)
- AI analysis knowledge base (seeded on first launch)
- Saved AI inferences and reports (every analysis and generated report, with timestamps and VIX context)

API keys are **not** stored in the database. They live only in your `.env` file.

No data is sent to external services other than:
- API requests to FRED and Alpha Vantage for fetching market data
- API requests to Anthropic or OpenAI when you use the AI Analysis or Report features

---

## Version

The Hedgehog — Preview 0.1
Built with Rust, egui, and eframe.
"##;
