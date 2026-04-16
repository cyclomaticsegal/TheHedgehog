use crate::analysis::SpikeEpisode;
use crate::models::{
    AiEvent, AiInferenceResult, AlertLevel, Instrument, LlmProvider, Observation, ParsedHypothesis,
    SavedInference, VixStatus,
};
use anyhow::{Context, Result};
use chrono::NaiveDate;
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::sync::mpsc::Sender;

/// Per-instrument snapshot sent to the LLM. Carries the absolute close price
/// (authoritative ground truth for the model) alongside the 30-day percent
/// change. Without the absolute close, the model has no anchor and falls back
/// on training-data priors (e.g. quoting gold at $2,000 when spot is $4,600).
pub struct InstrumentSnapshot {
    pub instrument: Instrument,
    pub latest_close: Option<f64>,
    pub latest_date: Option<NaiveDate>,
    pub pct_change_30d: Option<f64>,
}

impl InstrumentSnapshot {
    pub fn from_series(instrument: Instrument, series: &[Observation]) -> Self {
        let latest = series.last();
        Self {
            instrument,
            latest_close: latest.map(|o| o.close),
            latest_date: latest.map(|o| o.date),
            pct_change_30d: pct_change_over_window(series, 30),
        }
    }
}

pub struct AiRequest {
    pub provider: LlmProvider,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub system_prompt: String,
    pub user_message: String,
}

pub fn run_analysis(request: AiRequest, tx: Sender<AiEvent>) {
    let provider_str = request.provider.storage_key().to_owned();
    let model_str = request.model.clone();
    let sys_prompt = request.system_prompt.clone();
    let usr_msg = request.user_message.clone();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call_llm(&request)));
    match result {
        Ok(Ok(text)) => {
            let _ = tx.send(AiEvent::Response(AiInferenceResult {
                provider: provider_str,
                model: model_str,
                system_prompt: sys_prompt,
                user_message: usr_msg,
                response: text,
            }));
        }
        Ok(Err(err)) => {
            let _ = tx.send(AiEvent::Failed(format!("{err:#}")));
        }
        Err(_) => {
            let _ = tx.send(AiEvent::Failed(
                "Analysis thread panicked unexpectedly.".to_owned(),
            ));
        }
    }
}

fn call_llm(request: &AiRequest) -> Result<String> {
    let client = Client::builder()
        .user_agent(crate::USER_AGENT)
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .context("failed to build HTTP client")?;

    match request.provider {
        LlmProvider::Anthropic => call_anthropic(&client, request),
        LlmProvider::OpenAI => call_openai(&client, request),
    }
}

// Web-search-augmented responses (Anthropic's web_search_20250305 and
// OpenAI's web_search_preview) include search snippets and tool-call
// scaffolding alongside the final text, so the JSON envelope is larger
// than a plain chat completion. 500KB leaves room without inviting abuse.
const MAX_RESPONSE_BYTES: usize = 500_000;

/// Send an HTTP request and return the JSON body. On non-2xx status codes,
/// the error message includes the response body with sensitive data redacted.
fn send_and_parse(response: reqwest::blocking::Response) -> Result<Value> {
    let status = response.status();
    let bytes = response
        .bytes()
        .context("failed to read response body")?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(anyhow::anyhow!(
            "response too large ({} bytes, max {})",
            bytes.len(),
            MAX_RESPONSE_BYTES
        ));
    }
    if !status.is_success() {
        let body = String::from_utf8_lossy(&bytes);
        let redacted = redact_keys(&body);
        return Err(anyhow::anyhow!("HTTP {status}: {redacted}"));
    }
    serde_json::from_slice(&bytes).context("failed to parse JSON response")
}

/// Remove anything that looks like an API key from error output.
fn redact_keys(text: &str) -> String {
    let mut result = text.to_owned();
    for prefix in ["sk-", "key-"] {
        while let Some(start) = result.find(prefix) {
            let end = result[start..]
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .map(|i| start + i)
                .unwrap_or(result.len());
            result.replace_range(start..end, "[REDACTED]");
        }
    }
    result
}

fn call_anthropic(client: &Client, request: &AiRequest) -> Result<String> {
    let body = json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "temperature": 0.3,
        "system": request.system_prompt,
        "messages": [{"role": "user", "content": request.user_message}],
        "tools": [{"type": "web_search_20250305"}]
    });

    let raw = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &request.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .context("Anthropic request failed")?;

    let response = send_and_parse(raw)?;

    // When web_search is used the content array may contain tool_use blocks
    // before the final text block — find the text block explicitly.
    response["content"]
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .find(|block| block["type"].as_str() == Some("text"))
                .and_then(|block| block["text"].as_str())
                .map(|s| s.to_owned())
        })
        .context("unexpected Anthropic response shape")
}

/// OpenAI integration uses the Responses API (`/v1/responses`) — not Chat
/// Completions — because the `web_search_preview` tool is only exposed
/// through that endpoint. The shape is: `instructions` for the system
/// prompt, `input` for the user message, `tools` for the web search tool,
/// and the response carries either an `output_text` convenience field or
/// a structured `output` array with `output_text` content blocks.
fn call_openai(client: &Client, request: &AiRequest) -> Result<String> {
    let body = json!({
        "model": request.model,
        "max_output_tokens": request.max_tokens,
        "temperature": 0.3,
        "instructions": request.system_prompt,
        "input": request.user_message,
        "tools": [{"type": "web_search_preview"}]
    });

    let raw = client
        .post("https://api.openai.com/v1/responses")
        .header("Authorization", format!("Bearer {}", request.api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .context("OpenAI request failed")?;

    let response = send_and_parse(raw)?;

    // Prefer the convenience field if the SDK populated it.
    if let Some(text) = response["output_text"].as_str() {
        if !text.is_empty() {
            return Ok(text.to_owned());
        }
    }

    // Fall back to walking the output array. With web_search_preview the
    // array contains tool-call entries before the final assistant message,
    // so we explicitly look for `type == "message"` and inside it the
    // `output_text` content block.
    response["output"]
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .filter(|item| item["type"].as_str() == Some("message"))
                .find_map(|item| {
                    item["content"].as_array().and_then(|content| {
                        content
                            .iter()
                            .find(|c| c["type"].as_str() == Some("output_text"))
                            .and_then(|c| c["text"].as_str())
                            .map(str::to_owned)
                    })
                })
        })
        .context("unexpected OpenAI response shape (no output_text found)")
}

// ---------------------------------------------------------------------------
// Context assembly
// ---------------------------------------------------------------------------

/// Render one instrument snapshot line for the user message. The chosen
/// precision tracks the magnitude (e.g. BTC needs no decimals, gas needs more)
/// so the LLM doesn't anchor on rounding artefacts.
fn format_snapshot_line(snap: &InstrumentSnapshot) -> String {
    let name = snap.instrument.as_str();
    match (snap.latest_close, snap.latest_date, snap.pct_change_30d) {
        (Some(close), Some(date), Some(pct)) => {
            format!(
                "- {}: ${} as of {} ({:+.1}% over 30d)\n",
                name,
                format_price(close),
                date.format("%Y-%m-%d"),
                pct,
            )
        }
        (Some(close), Some(date), None) => {
            format!(
                "- {}: ${} as of {} (insufficient history for 30d change)\n",
                name,
                format_price(close),
                date.format("%Y-%m-%d"),
            )
        }
        _ => format!("- {name}: (no data)\n"),
    }
}

fn format_price(value: f64) -> String {
    if value >= 1000.0 {
        format!("{value:.0}")
    } else if value >= 10.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.3}")
    }
}

pub fn pct_change_over_window(observations: &[Observation], window_days: i64) -> Option<f64> {
    let latest = observations.last()?;
    let cutoff = latest.date - chrono::Duration::days(window_days);
    let baseline = observations.iter().find(|obs| obs.date >= cutoff)?;
    if baseline.close.abs() < f64::EPSILON {
        return None;
    }
    Some((latest.close - baseline.close) / baseline.close * 100.0)
}

/// Build the system prompt. `prior_failures_block`, when non-empty, is
/// inserted after the three-tier framing and before the response
/// template as a "prior attempt — address these" correction block. Used
/// by Re-analyze to feed bias-judge failures back into the model.
pub fn assemble_system_prompt(
    knowledge_chunks: &[String],
    prior_failures_block: Option<&str>,
) -> String {
    let prefix = "\
You are an expert macro-financial analyst specializing in volatility regimes and market microstructure.

Your task: (1) Classify the current volatility regime from VIX and commodity data, (2) Generate a testable, \
time-bounded hypothesis suitable for Bayesian updating in a forecasting model.

PRIMARY DIRECTIVE — the subject of the hypothesis:
The hypothesis MUST be about the single instrument listed under 'Primary instrument' in the user message. \
That name is the subject. It is not a SECONDARY and it is not a TERTIARY. Do NOT substitute a different \
commodity regardless of which one a template example happens to mention — the example text is a SHAPE, not \
a SUBJECT. The subject is set by the user, not by this prompt.

HOW TO USE YOUR INPUTS — three tiers:
1. PRIMARY (the subject): The one instrument named under 'Primary instrument' in the user message. Every \
price reference, strike level, and outcome band you write must come from the 'Latest close' block for that \
instrument. The hypothesis lives or dies on this instrument.
2. SECONDARY (corroborative signal only): The instruments listed under 'Secondary instruments' in the user \
message — the ones the user selected alongside the primary. Name their current behaviour in the Hypothesis \
Context when it materially corroborates or challenges the primary thesis. They must NEVER appear as the \
grammatical subject of the hypothesis or any outcome band. If a secondary is silent on the mechanism, omit it.
3. TERTIARY (background only): The 'Other available instruments' block — commodities the user has NOT \
selected. Mention them only if their current behaviour materially corroborates or contradicts the primary \
thesis. Never make them the subject. If they don't pull their weight, omit them.

KNOWLEDGE LIBRARY (the mechanism source, not a tier): The chunks at the bottom of this system prompt. They \
describe how the primary has behaved during prior volatility regimes (2008 GFC, 2020 COVID, 2022 Ukraine, \
etc.). Use them to name the specific causal transmission channels you cite in the Hypothesis Context section, \
so the mechanism is grounded in observed history rather than generic macro talk.

GROUND TRUTH RULE: The 'Latest close' values in the user message are authoritative. They come directly from \
FRED (VIX) and Alpha Vantage (gold, silver, bitcoin, crude oil, natural gas) and are dated. You MUST use them as the current \
price level for every numeric claim — strike prices in your hypothesis, level references in your context, the \
'$X' figures in your outcome bands. Do NOT substitute prices from your training data. Do NOT round them to \
historically familiar figures. If your training prior says gold is around $2,000 but the user message says \
$4,624, the user message wins. The data may be more recent than your training cutoff.

CRITICAL: For the Hypothesis Context section, use web search to enrich the narrative with current \
events — Fed/ECB/BoE signals, supply disruptions, geopolitical incidents, sentiment shifts. But for any \
specific price level (gold, crude oil, bitcoin, equity indices), the user message values override anything \
web search returns. Web search dates and headline context: yes. Web search prices that contradict the user \
message: no.

RESPOND USING EXACTLY THIS TEMPLATE (no other sections, no preamble):

**Regime**: [one of: Demand Shock | Supply Shock | Financial Contagion | Geopolitical Spike | Normal | Mixed]
**Confidence**: [Low | Medium | High]
**Closest Historical Analogue**: [e.g. \"2020 COVID — early demand shock phase\"]

**Signal Reading** (2-3 sentences): What the VIX level and commodity movements tell you.

**Key Confirmation**: Which asset behavior most strongly supports this regime classification.

**Key Divergence**: Which asset behavior contradicts or complicates the picture (or \"None\" if all signals align).

**Watch For**: One specific signal that would change this assessment.

**Hypothesis**: [A substantive, time-bounded claim (7-90 days) explaining a specific price/behaviour change in the PRIMARY instrument (named under 'Primary instrument') and the mechanism driving it. Not a question. Use this shape, substituting the actual primary instrument and figures drawn from 'Latest close': \"[Primary instrument] will [hold above / break above / fall below / spike to] $[level from Latest close] through [horizon] as [named mechanism] [holds / fails] despite [counter-pressure].\"]
**Hypothesis Outcomes**: [2-4 mutually exclusive outcomes, each ≤ 60 chars, representing distinct causal paths for the PRIMARY instrument. Use this shape: \"Holds above $[level] — [mechanism A] | Falls below $[level] — [mechanism B] | Spikes to $[level]+ — [mechanism C]\"]
**Hypothesis Context**: [HARD MAX 300 words. Do not exceed 300 words under any circumstances; if you run long, cut detail rather than truncate mid-sentence. End on a complete sentence.

PURPOSE OF THIS FIELD: This text is sent verbatim to the 51Folds Bayesian forecasting API as the `additionalContext` field of the model-creation request. 51Folds parses it to derive the causal drivers (the 15 factor nodes for an Insights-tier model) that will be assigned states and probabilities. Treat it as briefing material for a Bayesian model builder, not prose for a human reader. The drivers 51Folds extracts are only as good as the signals you name explicitly here.

REQUIRED CONTENT (in this order, not as labelled subsections):
(1) The current macro setup that makes the hypothesis live RIGHT NOW — concrete numbers, dated events, named actors. Use the Latest closes from the user message for any price reference and use web search for dated headlines / central-bank signals / supply disruptions.
(2) The causal mechanism you expect — what propagates the move, in which order, through which channels (rates, liquidity, positioning, supply, sentiment). Name the specific transmission factors so 51Folds can lift them as drivers.
(3) The signals that would confirm vs contradict the hypothesis — at least three observable, measurable indicators per direction.
(4) Why the chosen 7–90 day horizon is correct — what calendar events, data releases, or structural deadlines anchor it.

Be dense. Avoid filler, hedging, and generic macro commentary. End on a complete sentence inside the 300-word limit.]

---
KNOWLEDGE LIBRARY — historical regime behaviour for the primary instrument. These chunks describe how the \
instrument your user is focused on has responded to prior volatility regimes. Use them to name the specific \
causal transmission channels you cite in Hypothesis Context — they are how you ground the mechanism in \
observed history rather than generic macro commentary. They are NOT the subject of the hypothesis; they are \
the source of the mechanism.\n\n";
    let extra_len = prior_failures_block.map(|s| s.len() + 2).unwrap_or(0);
    let total: usize =
        knowledge_chunks.iter().map(|c| c.len() + 2).sum::<usize>() + extra_len;
    let mut prompt = String::with_capacity(prefix.len() + total);
    prompt.push_str(prefix);
    if let Some(block) = prior_failures_block {
        if !block.trim().is_empty() {
            prompt.push_str(block);
            prompt.push_str("\n\n");
        }
    }
    for chunk in knowledge_chunks {
        prompt.push_str(chunk);
        prompt.push_str("\n\n");
    }
    prompt
}

pub fn assemble_user_message(
    vix_status: Option<&VixStatus>,
    primary: Instrument,
    secondary: &[Instrument],
    primary_snapshot: &InstrumentSnapshot,
    secondary_snapshots: &[InstrumentSnapshot],
    tertiary_snapshots: &[InstrumentSnapshot],
    spike_episodes: &[SpikeEpisode],
) -> String {
    let mut msg = String::from("## Current Market Snapshot\n\n");

    // VIX status
    match vix_status {
        Some(status) => {
            msg.push_str(&format!(
                "**VIX Status**: {:.2} — {} (thresholds: approaching {:.1} / extreme {:.1})\n",
                status.latest.close,
                status.level.label(),
                status.thresholds.approaching,
                status.thresholds.extreme,
            ));
            msg.push_str(&format!(
                "**Data as of**: {}\n\n",
                status.latest.date.format("%Y-%m-%d")
            ));
        }
        None => {
            msg.push_str("**VIX Status**: No data loaded\n\n");
        }
    }

    // Primary instrument — the sole subject of the hypothesis.
    msg.push_str(&format!("**Primary instrument**: {}\n\n", primary.as_str()));

    // Primary's latest close — the single anchor for strike levels and
    // outcome bands. Kept as its own block (not merged with secondaries)
    // so the LLM never uses a secondary's price as the subject price.
    msg.push_str("**Latest close (authoritative — use this in your hypothesis)**:\n");
    msg.push_str(&format_snapshot_line(primary_snapshot));
    msg.push('\n');

    // Secondary instruments — corroborative only. Block is emitted only
    // when the user selected more than one instrument.
    if !secondary.is_empty() {
        msg.push_str(
            "**Secondary instruments**: Selected alongside the primary — mention as \
corroborative or challenging signal in the Hypothesis Context, but NOT as the subject of the \
hypothesis or any outcome band. The same ground-truth rule applies: these prices override your \
training data.\n",
        );
        let names: Vec<&str> = secondary.iter().map(|i| i.as_str()).collect();
        msg.push_str(&format!("(Selected: {})\n", names.join(", ")));
        for snap in secondary_snapshots {
            msg.push_str(&format_snapshot_line(snap));
        }
        msg.push('\n');
    }

    // Spike episodes
    if !spike_episodes.is_empty() {
        msg.push_str("**Recent VIX Spike Episodes**:\n");
        for (i, spike) in spike_episodes.iter().enumerate() {
            let level_str = match spike.max_level {
                AlertLevel::Extreme => "EXTREME",
                AlertLevel::ApproachingExtreme => "Approaching",
                AlertLevel::Normal => "Normal",
            };
            msg.push_str(&format!(
                "{}. {} to {} | Peak VIX {:.1} | Duration {} days | {}\n",
                i + 1,
                spike.start.format("%b %d"),
                spike.end.format("%b %d"),
                spike.peak,
                spike.duration_points,
                level_str,
            ));
        }
        msg.push('\n');
    }

    // Tertiary instruments — not selected, background only.
    if !tertiary_snapshots.is_empty() {
        msg.push_str(
            "**Other available instruments (not in the user's selection)**: \
TERTIARY background only — see the three-tier framing in the system prompt. The user has NOT \
selected these for analysis, and they are NOT the subject of your hypothesis. Mention them only \
if their current behaviour materially corroborates or contradicts the primary thesis. If they \
don't pull their weight, omit them. Do not analyse them for their own sake. The same ground-truth \
rule applies: these prices override your training data.\n",
        );
        for snap in tertiary_snapshots {
            msg.push_str(&format_snapshot_line(snap));
        }
        msg.push('\n');
    }

    msg.push_str(
        "---\nClassify the regime and fill in the template. Be specific and concise. No filler.",
    );
    msg
}

// ---------------------------------------------------------------------------
// Outcomes reroll — ask the LLM for a fresh set of outcomes for an existing
// hypothesis without regenerating the whole regime analysis.
// ---------------------------------------------------------------------------

/// Build a focused (system, user) prompt pair that asks the LLM to produce a
/// new set of outcomes for an existing hypothesis. The previous outcomes are
/// included so the model can deliberately diverge from them.
pub fn assemble_outcomes_reroll_prompt(
    question: &str,
    context: &str,
    previous_outcomes: &[String],
) -> (String, String) {
    let system = "\
You are a Bayesian forecasting assistant. The user has an existing hypothesis and a fresh set of \
mutually exclusive outcomes is needed for it. You will return ONLY the outcomes — no preamble, no \
explanation, no regime classification.

RULES:
- 2 to 4 outcomes
- Each outcome ≤ 60 characters
- Each outcome must represent a DISTINCT causal path or mechanism
- Outcomes must be mutually exclusive and collectively cover the plausible space
- Avoid restating the previous outcomes verbatim — diverge in framing, thresholds, or mechanism
- Each outcome should pair a price/state band with the driving mechanism in shorthand

OUTPUT FORMAT (exactly this, nothing else):
**Outcomes**: outcome A — mechanism A | outcome B — mechanism B | outcome C — mechanism C
"
    .to_owned();

    let mut user = String::with_capacity(question.len() + context.len() + 256);
    user.push_str("## Hypothesis\n\n");
    user.push_str(question.trim());
    user.push_str("\n\n## Context\n\n");
    user.push_str(context.trim());
    user.push_str("\n\n## Previous outcomes (do not repeat verbatim)\n\n");
    for o in previous_outcomes {
        user.push_str("- ");
        user.push_str(o.trim());
        user.push('\n');
    }
    user.push_str("\n---\nReturn a fresh set of 2–4 outcomes using the exact output format above.");
    (system, user)
}

/// Split a raw outcomes block into individual outcome strings. The LLM is
/// asked to use `a | b | c` format, but in practice it often emits a
/// newline-bulleted list. This handles both: pipe-separated when present,
/// newline-bulleted when not. Bullet markers (`-`, `*`, `•`, `1.`, `1)`)
/// are stripped from each line. Used by both `parse_hypothesis` (initial
/// analysis) and `parse_outcomes_reroll` so the two paths stay consistent.
pub fn split_outcomes_block(body: &str) -> Vec<String> {
    let candidates: Vec<String> = if body.contains('|') {
        body.split('|')
            .map(strip_bullet)
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        body.lines()
            .map(strip_bullet)
            .filter(|s| !s.is_empty())
            .collect()
    };
    // Defensive cap — never accept more than 8 outcomes from a malformed
    // response, otherwise a stray bullet list could swamp the editor.
    candidates.into_iter().take(8).collect()
}

/// Parse the response of an outcomes-reroll request.
pub fn parse_outcomes_reroll(response: &str) -> Option<Vec<String>> {
    // Prefer the structured field if the LLM honoured the instructions.
    let raw = extract_field(response, "**Outcomes**:")
        .or_else(|| extract_field(response, "**Hypothesis Outcomes**:"));

    let candidates = match raw {
        Some(body) => split_outcomes_block(&body),
        None => response
            .lines()
            .map(strip_bullet)
            .filter(|s| !s.is_empty() && looks_like_outcome(s))
            .take(8)
            .collect(),
    };

    if candidates.len() < 2 {
        return None;
    }
    Some(candidates)
}

fn strip_bullet(line: &str) -> String {
    let mut s = line.trim();
    // Strip a leading list marker: -, *, •, or 1./1)
    loop {
        let trimmed = s
            .trim_start_matches(['-', '*', '•', '·', '−'])
            .trim_start();
        if trimmed == s {
            break;
        }
        s = trimmed;
    }
    // Strip a leading "1." / "1)" enumerator.
    let bytes = s.as_bytes();
    let digit_end = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    if digit_end > 0
        && digit_end < bytes.len()
        && (bytes[digit_end] == b'.' || bytes[digit_end] == b')')
    {
        s = s[digit_end + 1..].trim_start();
    }
    s.trim().to_owned()
}

/// Heuristic for the freeform fallback path: a line is "outcome-shaped" if
/// it has at least one alphabetic character and isn't a section header
/// (which always starts with `**`).
fn looks_like_outcome(line: &str) -> bool {
    !line.starts_with("**")
        && line.chars().any(|c| c.is_alphabetic())
        && line.len() <= 120
}

// ---------------------------------------------------------------------------
// Hypothesis parsing
// ---------------------------------------------------------------------------

pub fn parse_hypothesis(response: &str) -> Option<ParsedHypothesis> {
    let question = extract_field(response, "**Hypothesis**:")?;
    let outcomes_raw = extract_field(response, "**Hypothesis Outcomes**:")?;
    let context = extract_field(response, "**Hypothesis Context**:")?;

    // Use the shared splitter so the initial parser handles bullet-list
    // outcomes (`- a\n- b\n- c`) the same way the reroll parser does. The
    // old code only split on `|`, which collapsed bullet lists into a
    // single outcome and produced the doubled `•  -` rendering.
    let outcomes = split_outcomes_block(&outcomes_raw);

    if outcomes.is_empty() {
        return None;
    }

    Some(ParsedHypothesis { question, outcomes, context })
}

fn extract_field(text: &str, marker: &str) -> Option<String> {
    let start = text.find(marker)? + marker.len();
    let rest = text[start..].trim_start();
    // Value ends at the next bold marker or end of string
    let end = rest.find("\n**").unwrap_or(rest.len());
    let value = rest[..end].trim().to_owned();
    if value.is_empty() { None } else { Some(value) }
}

// ---------------------------------------------------------------------------
// Report generation (Phase 2)
// ---------------------------------------------------------------------------

pub fn assemble_report_prompt(
    inferences: &[SavedInference],
    from: &str,
    to: &str,
) -> (String, String) {
    let system_prompt = "\
You are a senior macro-financial analyst producing a retrospective summary report.

RESPOND USING EXACTLY THIS TEMPLATE:

## Executive Summary
2-3 sentences: dominant regime, direction of travel, headline conclusion.

## Period Overview
Chronological summary of regime states observed. One bullet per distinct phase. Include dates and VIX levels.

## Key Themes
3-5 bullet points identifying the strongest recurring signals across analyses.

## Historical Context
Which historical episode(s) most closely match this period? Be specific about similarities and differences.

## Outlook
1-2 sentences on what the trajectory suggests going forward. Name the one signal to watch.

Keep total response under 600 words. No filler. Every sentence must add information."
        .to_owned();

    let mut user_msg = format!(
        "## Report Period: {} to {}\n\nNumber of analyses: {}\n\n",
        from,
        to,
        inferences.len()
    );

    for (i, inf) in inferences.iter().enumerate() {
        let vix_str = inf
            .vix_close
            .map(|v| format!("{v:.1}"))
            .unwrap_or_else(|| "N/A".to_owned());
        let response_text = if inferences.len() > 20 {
            // Truncate to prevent context window overflow
            let truncated: String = inf.response.chars().take(500).collect();
            if inf.response.len() > 500 {
                format!("{truncated}... [truncated]")
            } else {
                truncated
            }
        } else {
            inf.response.clone()
        };
        user_msg.push_str(&format!(
            "---\n### Analysis {} ({}, VIX: {})\n\n{}\n\n",
            i + 1,
            &inf.created_at[..19.min(inf.created_at.len())],
            vix_str,
            response_text,
        ));
    }

    user_msg.push_str(
        "---\nSynthesize these analyses into a comprehensive retrospective report.",
    );

    (system_prompt, user_msg)
}
