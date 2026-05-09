use crate::storage::Storage;

pub struct KnowledgeChunk {
    pub title: &'static str,
    pub tags: &'static str,
    pub body: &'static str,
}

pub fn retrieve_for_context(storage: &Storage, instrument_tags: &[&str]) -> Vec<String> {
    storage.load_knowledge_chunks(instrument_tags).unwrap_or_default()
}

pub const KNOWLEDGE_BASE: &[KnowledgeChunk] = &[
    // -----------------------------------------------------------------------
    // Universal chunks (tags: "all")
    // -----------------------------------------------------------------------
    KnowledgeChunk {
        title: "VIX Mechanics and Interpretation",
        tags: "all",
        body: "The CBOE Volatility Index (VIX) measures the market's expectation of 30-day \
forward volatility derived from S&P 500 index option prices. Key levels: below 15 is \
historically calm, 15-20 is average, 20-30 signals elevated stress, and above 30 indicates \
crisis-level fear. The VIX is mean-reverting — spikes are typically short-lived (days to weeks) \
but the magnitude and duration carry information about the nature of the shock. VIX futures are \
almost always in contango (upward sloping), which steepens during calm and inverts during \
panics. A persistently elevated VIX (weeks above 25) suggests systemic repricing, not a \
one-off event.",
    },
    KnowledgeChunk {
        title: "VIX Alert Levels in This Dashboard",
        tags: "all",
        body: "The dashboard classifies VIX into three alert levels. Normal (green) means VIX \
is below the approaching threshold. Approaching Extreme (amber) means VIX is between the \
approaching and extreme thresholds. Extreme (red) means VIX is above the extreme threshold. \
In rolling percentile mode (default), the thresholds are computed from the VIX's own \
distribution over a lookback window — typically the 85th and 95th percentiles of 252 trading \
days. This means thresholds adapt to the prevailing volatility regime. In fixed mode, absolute \
VIX levels are used (e.g. 25/35).",
    },
    KnowledgeChunk {
        title: "Regime Taxonomy: Demand Shock",
        tags: "all",
        body: "A demand shock regime occurs when economic activity contracts sharply, pulling \
down both equities and commodities tied to growth. Signature: VIX spikes, crude oil and \
natural gas fall (demand destruction), Bitcoin sells off sharply as a risk asset. Gold may \
initially sell off due to margin calls and liquidity crunch before recovering as a safe haven. \
The dollar typically strengthens as capital flows to safety. Historical examples: 2008 GFC \
(VIX ~89, oil from $147 to $35), 2020 COVID crash (VIX ~85, WTI briefly negative). Duration: \
weeks to months.",
    },
    KnowledgeChunk {
        title: "Regime Taxonomy: Supply Shock",
        tags: "all",
        body: "A supply shock regime occurs when production or distribution of key commodities \
is disrupted, typically by geopolitical conflict or policy. Signature: VIX rises moderately \
(25-40 range, not extreme), energy prices spike (supply constrained). Gold rises on \
uncertainty premium. The key differentiator from demand shocks: commodities and VIX move UP \
together rather than commodities falling. Historical examples: 2022 Ukraine invasion (energy \
+80%, VIX ~36), 1973 oil embargo.",
    },
    KnowledgeChunk {
        title: "Regime Taxonomy: Financial Contagion",
        tags: "all",
        body: "Financial contagion regimes are triggered by cascading failures in the financial \
system — bank runs, credit freezes, counterparty collapses. Signature: VIX extreme (often \
60+), correlations spike to 1 across all risk assets (everything sells off together), gold \
initially sells off (margin calls force liquidation of ALL assets) then rallies as the safe \
haven bid emerges, credit spreads blow out. Bitcoin, if included, trades as a pure risk asset \
and crashes with equities. The defining feature is that diversification fails temporarily — \
correlations approach 1. Historical examples: 2008 Lehman collapse, March 2020 liquidity \
crisis.",
    },
    KnowledgeChunk {
        title: "Regime Taxonomy: Geopolitical Spike",
        tags: "all",
        body: "Geopolitical spikes are short-lived VIX events driven by specific political or \
military developments. Signature: VIX jumps 5-15 points in a day or two, energy prices react \
based on proximity to supply routes, gold gets a brief safe-haven bid, but markets often \
recover within days if the event doesn't escalate. The key differentiator: speed of onset and \
resolution. If the geopolitical event leads to actual supply disruption, it transitions into a \
supply shock regime. If it remains rhetorical, VIX mean-reverts quickly. Watch for: oil price \
persistence (sustained = real supply impact) and credit spread movement (widening = contagion \
risk).",
    },
    KnowledgeChunk {
        title: "Reading the Normalized Comparison Chart",
        tags: "all",
        body: "The comparison chart normalizes all instruments to percentage change from the \
start of the visible window (base = 100). This removes absolute price differences and makes \
relative performance directly comparable. A line at +10% means that asset gained 10% from the \
window start; -10% means it lost 10%. When multiple assets diverge sharply (one at +20%, \
another at -15%), it signals differentiated regime behavior — some assets are acting as hedges \
while others are selling off. Convergence (all lines moving together) suggests correlation \
increase, which is typical in acute crisis phases.",
    },
    KnowledgeChunk {
        title: "Dollar Dynamics During VIX Spikes",
        tags: "all",
        body: "The US dollar typically strengthens during VIX spikes as global capital seeks \
safety in US Treasuries and dollar-denominated assets. This dollar strength creates a headwind \
for all dollar-denominated commodities — even those not directly affected by the underlying \
shock. Gold is partially insulated because its safe-haven demand can offset dollar headwinds, \
but silver feels the full effect. Crude oil and natural gas trade in dollars too, so a strong \
dollar compounds any demand-driven weakness in energy.",
    },
    KnowledgeChunk {
        title: "2008 Global Financial Crisis",
        tags: "all",
        body: "The 2008 GFC was a financial contagion regime triggered by the subprime mortgage \
collapse. VIX peaked at 89.5 in October 2008. Key commodity responses: Gold initially dropped \
~25% from March-October 2008 as margin calls forced liquidation, then rallied to all-time \
highs by 2011. Crude oil collapsed from $147 (July 2008) to $32 (December 2008) — a 78% \
decline driven by demand destruction. Silver fell ~50% (industrial demand collapse). The key \
lesson: in the acute phase of financial contagion, ALL assets sell off as correlations spike \
to 1. Safe-haven behavior only emerges after the liquidity crunch passes (typically 2-4 weeks).",
    },
    KnowledgeChunk {
        title: "2020 COVID Crash",
        tags: "all",
        body: "The COVID crash was a demand shock with financial contagion characteristics. VIX \
peaked at 82.7 in March 2020. Key commodity responses: Gold dipped briefly (-12% from peak) \
during the March liquidity crunch then rallied to all-time highs above $2,000 by August 2020. \
WTI crude oil futures went negative on April 20, 2020 (storage crisis). Bitcoin crashed ~50% \
in March 2020, disproving the digital gold narrative, but recovered to new highs by December. \
The key lesson: the recovery was unusually fast due to unprecedented monetary and fiscal \
stimulus — the VIX normalized within 3 months, much faster than 2008.",
    },
    KnowledgeChunk {
        title: "2022 Ukraine / Inflation Shock",
        tags: "all",
        body: "The 2022 episode was a supply shock layered on existing inflation. VIX was \
sustained in the 25-35 range (not extreme by crisis standards). Key commodity responses: \
Natural gas spiked ~300% (European dependency on Russian gas). Crude oil rose ~65%. Gold was \
range-bound despite high VIX because rising real interest rates offset safe haven demand. \
Bitcoin fell ~65% (crypto winter, compounded by LUNA/FTX collapses). The key lesson: supply \
shocks produce very different commodity winners vs demand shocks — energy UP rather than down.",
    },

    // -----------------------------------------------------------------------
    // Gold
    // -----------------------------------------------------------------------
    KnowledgeChunk {
        title: "Gold: Flight-to-Safety Mechanics",
        tags: "gold,all",
        body: "Gold is the canonical safe-haven asset during VIX spikes. Research by Baur & \
Lucey (2010) established that gold acts as both a hedge against stocks on average and a safe \
haven during extreme market conditions. However, the safe-haven property is typically \
short-lived — approximately 15 trading days. After the initial crisis passes, gold's behavior \
depends on the monetary policy response: rate cuts and QE are strongly gold-positive (lower \
real rates increase gold's relative attractiveness). Central bank gold purchases have become a \
structural demand driver since 2022, with China, Poland, and other central banks accumulating \
at record pace.",
    },
    KnowledgeChunk {
        title: "Gold: When Gold Sells Off Despite High VIX",
        tags: "gold",
        body: "Gold can sell off during VIX spikes in two scenarios. First, in the acute phase \
of a liquidity crisis (margin-call selling), investors are forced to sell their most liquid \
assets — including gold — to meet margin requirements. This happened in October 2008 (-25%) \
and March 2020 (-12%). The sell-off is typically brief (1-3 weeks) and is followed by strong \
recovery. Second, when real interest rates rise sharply (nominal rates rising faster than \
inflation expectations), gold becomes less attractive relative to interest-bearing assets. This \
was the dominant dynamic in 2022 when aggressive Fed rate hikes capped gold despite the \
Ukraine conflict.",
    },
    KnowledgeChunk {
        title: "Gold: Real Rates Relationship",
        tags: "gold",
        body: "Gold's price has a strong negative correlation with US real interest rates \
(nominal rates minus inflation expectations). When real rates fall (especially into negative \
territory), gold becomes more attractive because the opportunity cost of holding a \
zero-yielding asset decreases. During VIX spikes, the key question for gold is: will the \
central bank response push real rates lower (cutting rates, QE) or higher (tightening to fight \
inflation)? Rate-cutting responses are strongly gold-positive; tightening responses cap gold \
even if VIX remains elevated.",
    },

    // -----------------------------------------------------------------------
    // Silver
    // -----------------------------------------------------------------------
    KnowledgeChunk {
        title: "Silver: Hybrid Metal Dynamics",
        tags: "silver",
        body: "Silver has a dual nature: approximately 50% of demand is industrial (solar panels, \
electronics, medical devices) and 50% is investment/monetary. This means silver tends to \
underperform gold during VIX spikes because its industrial demand component is pro-cyclical. \
In demand shocks, silver can fall sharply as manufacturing contracts, even as gold rises. \
Silver is also more volatile than gold — it tends to overshoot in both directions, falling \
harder in crises and rallying more aggressively in recoveries. The gold/silver ratio is a \
useful regime indicator.",
    },
    KnowledgeChunk {
        title: "Silver: Gold/Silver Ratio as Regime Signal",
        tags: "gold,silver",
        body: "The gold/silver ratio (gold price divided by silver price) is a reliable regime \
indicator. A ratio above 80 signals acute risk-off conditions — investors are preferring gold \
over silver's industrial exposure. The ratio spiked to 125 during the March 2020 COVID crash \
(the highest in recorded history). A ratio below 50 signals risk-on recovery and industrial \
demand strength. During VIX spikes, watch whether the ratio is expanding (crisis deepening, \
investors fleeing to pure safe haven) or contracting (crisis stabilizing, industrial recovery \
beginning). A falling ratio while VIX remains elevated suggests the market is looking through \
the crisis toward recovery.",
    },

    // -----------------------------------------------------------------------
    // Crude Oil
    // -----------------------------------------------------------------------
    KnowledgeChunk {
        title: "Crude Oil: Supply vs Demand Shock Identification",
        tags: "crude_oil",
        body: "Crude oil's response to VIX spikes depends entirely on the type of shock. In \
demand shocks (recessions, pandemics), oil falls sharply because global consumption contracts — \
2008: -78%, 2020: briefly negative. In supply shocks (geopolitical conflict, sanctions), oil \
rises because production is constrained while demand persists — 2022: +65%. This makes oil the \
single most useful instrument for regime identification: if VIX is elevated and oil is falling, \
it's a demand shock; if VIX is elevated and oil is rising, it's a supply shock. The direction \
of oil during a VIX spike tells you more about the nature of the crisis than any other \
single indicator.",
    },
    KnowledgeChunk {
        title: "Crude Oil: Contango and Demand Destruction",
        tags: "crude_oil",
        body: "When oil enters steep contango (front-month futures much cheaper than later \
months), it signals demand destruction and storage oversupply. The extreme case was April 2020 \
when WTI went negative because physical storage was full and holders of expiring contracts \
paid others to take delivery. ETF proxies like USO roll futures monthly, so steep contango \
creates a persistent negative roll yield — the ETF loses value even if spot oil is stable. \
During VIX spikes, check whether oil contango is steepening (demand shock worsening) or \
flattening (supply tightening or demand recovery).",
    },

    // -----------------------------------------------------------------------
    // Natural Gas
    // -----------------------------------------------------------------------
    KnowledgeChunk {
        title: "Natural Gas: Seasonality vs Geopolitical",
        tags: "natural_gas",
        body: "Natural gas has strong seasonal patterns (winter heating demand peaks, summer \
cooling peaks) that can dominate VIX-driven moves. Unlike crude oil, natural gas markets are \
regional — US Henry Hub prices can diverge sharply from European TTF prices. The 2022 spike \
was primarily a European phenomenon driven by Russian supply disruption; US natural gas rose \
much less. Natural gas has lower direct correlation to VIX than crude oil because its supply \
and demand are more weather-dependent than growth-dependent. During VIX spikes, check whether \
natural gas is moving with oil (broad energy crisis) or independently (weather/regional \
supply issue).",
    },

    // -----------------------------------------------------------------------
    // Bitcoin
    // -----------------------------------------------------------------------
    KnowledgeChunk {
        title: "Bitcoin: Risk Asset vs Inflation Hedge Narrative",
        tags: "bitcoin,all",
        body: "Despite narratives of 'digital gold', empirical evidence shows Bitcoin behaves \
as a high-beta risk asset during VIX spikes. It sold off ~50% in March 2020 alongside equities, \
and fell ~65% in 2022 during the crypto winter. Bitcoin's correlation to the Nasdaq has \
increased since 2020 as institutional adoption brought it into traditional risk frameworks. \
During VIX spikes, expect Bitcoin to sell off with or more sharply than equities. The \
inflation-hedge narrative may hold over multi-year periods but fails consistently during acute \
market stress. When analyzing Bitcoin alongside VIX, treat it as a risk-on indicator: if \
Bitcoin is holding up during a VIX spike, the market is not truly fearful.",
    },
    KnowledgeChunk {
        title: "Bitcoin: Post-Crisis Recovery Speed",
        tags: "bitcoin",
        body: "While Bitcoin crashes harder than most assets during VIX spikes (high beta on \
the downside), it also tends to recover faster and overshoot on the upside. After the March \
2020 crash (-50%), Bitcoin recovered to pre-crash levels within 2 months and reached new \
all-time highs within 9 months. This high-beta recovery pattern means Bitcoin's drawdown \
during a VIX spike may overstate the lasting damage. For regime analysis, the key question \
is whether Bitcoin is leading or lagging the recovery: if Bitcoin recovers before equities \
normalize, it suggests risk appetite is returning.",
    },
    KnowledgeChunk {
        title: "Bitcoin: Correlation Regime Shift Post-2020",
        tags: "bitcoin",
        body: "Before 2020, Bitcoin had genuinely low correlation to traditional markets, \
supporting the diversification narrative. Since 2020, institutional adoption (ETFs, corporate \
treasury holdings, futures markets) has pulled Bitcoin into traditional risk frameworks. The \
30-day rolling correlation between Bitcoin and the S&P 500 has averaged 0.3-0.5 since 2021, \
compared to near-zero before 2020. During VIX spikes, this correlation spikes further toward \
0.7-0.8. Additionally, crypto-specific risks (exchange failures, stablecoin collapses, \
regulatory actions) can cause Bitcoin-specific VIX-independent drawdowns, as seen with LUNA \
(May 2022) and FTX (November 2022).",
    },

];
