//! Commodity-bias validation for the AI analysis pipeline.
//!
//! Two layers, both active at runtime:
//!
//! **Layer 1 — Prompt eval** (`EvalRule` / `run_all_checks`): deterministic
//! string assertions against the generated system + user prompts. Validates
//! source attribution, instrument separation, knowledge relevance. No LLM.
//!
//! **Layer 2 — Response validation** (`ResponseRule` / `validate_response` +
//! `BiasJudge`): checks the LLM's actual analysis output. Deterministic
//! checks run instantly (instrument naming, price anchoring). An optional
//! LLM-as-judge call validates semantic properties (mechanism relevance,
//! tertiary boundary respect) using whatever provider/key the user has.

#![allow(dead_code)]

use crate::ai::InstrumentSnapshot;
use crate::knowledge;
use crate::models::Instrument;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Public types — always compiled, reusable for runtime pre-flight checks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalRule {
    /// FRED attributed only to VIX; Alpha Vantage for all commodities including Soybeans.
    GroundTruthSources,
    /// "Instruments in view" in the user message lists exactly the selected instruments.
    PrimarySubjectMatch,
    /// Every selected instrument appears in the "Latest closes" block.
    SelectedInLatestCloses,
    /// Unselected instruments appear under "Other available instruments" with TERTIARY warning.
    UnselectedTertiaryFraming,
    /// Selected instruments must NOT appear in the tertiary block.
    NoSelectedInTertiary,
    /// Unselected instruments must NOT appear in the "Latest closes" block.
    NoUnselectedInPrimary,
    /// Knowledge chunks include tags for each selected instrument.
    KnowledgeRelevance,
    /// System prompt template does not name a specific commodity as the hypothesis subject.
    NoHardcodedSubject,
    /// Source attribution strings are consistent (no stale FRED references for commodities).
    SourceAttributionConsistent,
}

#[derive(Debug)]
pub struct EvalResult {
    pub rule: EvalRule,
    pub pass: bool,
    pub reason: String,
}

pub struct EvalScenario {
    pub name: &'static str,
    pub selected: Vec<Instrument>,
    pub system_prompt: String,
    pub user_message: String,
    /// Storage keys of selected instruments (for knowledge relevance checks).
    pub knowledge_tags: Vec<String>,
}

impl EvalResult {
    fn pass(rule: EvalRule) -> Self {
        Self { rule, pass: true, reason: String::new() }
    }
    fn fail(rule: EvalRule, reason: String) -> Self {
        Self { rule, pass: false, reason }
    }
}

// ---------------------------------------------------------------------------
// Text extraction helpers
// ---------------------------------------------------------------------------

/// Extract text between a markdown bold header and the next bold header or end.
fn extract_block(text: &str, header: &str) -> Option<String> {
    let start = text.find(header)?;
    let after = start + header.len();
    let rest = &text[after..];
    let end = rest.find("\n**").unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// Extract the tertiary block (from "Other available instruments" to "---" or end).
fn extract_tertiary_block(text: &str) -> Option<String> {
    let marker = "**Other available instruments";
    let start = text.find(marker)?;
    let rest = &text[start..];
    let end = rest.find("\n---").unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// Extract the system prompt instruction prefix (before the KNOWLEDGE BASE section).
/// Uses the section divider `"---\nKNOWLEDGE BASE"` to avoid matching the inline
/// mention in the three-tier instructions.
fn system_prompt_prefix(text: &str) -> &str {
    text.find("---\nKNOWLEDGE BASE")
        .map(|i| &text[..i])
        .unwrap_or(text)
}

/// Compute the unselected non-VIX instruments for a given selection.
fn unselected_instruments(selected: &[Instrument]) -> Vec<Instrument> {
    Instrument::ALL
        .iter()
        .copied()
        .filter(|i| *i != Instrument::Vix && !selected.contains(i))
        .collect()
}

// ---------------------------------------------------------------------------
// Check implementations
// ---------------------------------------------------------------------------

fn check_ground_truth_sources(s: &EvalScenario) -> EvalResult {
    let rule = EvalRule::GroundTruthSources;
    let expected = "FRED (VIX) and Alpha Vantage (all commodities including Soybeans)";
    if s.system_prompt.contains(expected) {
        EvalResult::pass(rule)
    } else {
        // Detect the specific regression: Soybeans attributed to FRED
        let prefix = system_prompt_prefix(&s.system_prompt);
        if prefix.contains("FRED (VIX, Soybeans)")
            || prefix.contains("FRED (VIX and Soybeans)")
        {
            EvalResult::fail(
                rule,
                "Soybeans is attributed to FRED instead of Alpha Vantage".into(),
            )
        } else {
            EvalResult::fail(rule, format!("Expected exact phrase: \"{expected}\""))
        }
    }
}

fn check_primary_subject_match(s: &EvalScenario) -> EvalResult {
    let rule = EvalRule::PrimarySubjectMatch;
    let marker = "**Instruments in view**: ";
    let Some(start) = s.user_message.find(marker) else {
        return EvalResult::fail(rule, "Missing 'Instruments in view' section".into());
    };
    let after = start + marker.len();
    let rest = &s.user_message[after..];
    let end = rest.find('\n').unwrap_or(rest.len());
    let instruments_text = rest[..end].trim();

    let found: HashSet<&str> = instruments_text.split(", ").collect();
    let expected: HashSet<&str> = s.selected.iter().map(|i| i.as_str()).collect();

    if found == expected {
        EvalResult::pass(rule)
    } else {
        EvalResult::fail(rule, format!("Expected {expected:?}, found {found:?}"))
    }
}

fn check_selected_in_latest_closes(s: &EvalScenario) -> EvalResult {
    let rule = EvalRule::SelectedInLatestCloses;
    let Some(block) = extract_block(&s.user_message, "**Latest closes") else {
        return EvalResult::fail(rule, "Missing 'Latest closes' block".into());
    };
    let missing: Vec<&str> = s
        .selected
        .iter()
        .filter(|i| !block.contains(i.as_str()))
        .map(|i| i.as_str())
        .collect();
    if missing.is_empty() {
        EvalResult::pass(rule)
    } else {
        EvalResult::fail(rule, format!("Missing from Latest closes: {missing:?}"))
    }
}

fn check_unselected_tertiary_framing(s: &EvalScenario) -> EvalResult {
    let rule = EvalRule::UnselectedTertiaryFraming;
    let unselected = unselected_instruments(&s.selected);

    if unselected.is_empty() {
        return EvalResult::pass(rule);
    }

    let Some(block) = extract_tertiary_block(&s.user_message) else {
        return EvalResult::fail(
            rule,
            "Missing 'Other available instruments' block".into(),
        );
    };

    if !block.contains("TERTIARY") {
        return EvalResult::fail(
            rule,
            "Tertiary block missing 'TERTIARY' warning text".into(),
        );
    }

    let missing: Vec<&str> = unselected
        .iter()
        .filter(|i| !block.contains(i.as_str()))
        .map(|i| i.as_str())
        .collect();
    if missing.is_empty() {
        EvalResult::pass(rule)
    } else {
        EvalResult::fail(rule, format!("Missing from tertiary block: {missing:?}"))
    }
}

fn check_no_selected_in_tertiary(s: &EvalScenario) -> EvalResult {
    let rule = EvalRule::NoSelectedInTertiary;
    let Some(block) = extract_tertiary_block(&s.user_message) else {
        return EvalResult::pass(rule);
    };
    let leaked: Vec<&str> = s
        .selected
        .iter()
        .filter(|i| block.contains(i.as_str()))
        .map(|i| i.as_str())
        .collect();
    if leaked.is_empty() {
        EvalResult::pass(rule)
    } else {
        EvalResult::fail(
            rule,
            format!("Selected instruments leaked into tertiary: {leaked:?}"),
        )
    }
}

fn check_no_unselected_in_primary(s: &EvalScenario) -> EvalResult {
    let rule = EvalRule::NoUnselectedInPrimary;
    let Some(block) = extract_block(&s.user_message, "**Latest closes") else {
        return EvalResult::fail(rule, "Missing 'Latest closes' block".into());
    };
    let unselected = unselected_instruments(&s.selected);
    let leaked: Vec<&str> = unselected
        .iter()
        .filter(|i| block.contains(i.as_str()))
        .map(|i| i.as_str())
        .collect();
    if leaked.is_empty() {
        EvalResult::pass(rule)
    } else {
        EvalResult::fail(
            rule,
            format!("Unselected instruments in Latest closes: {leaked:?}"),
        )
    }
}

fn check_knowledge_relevance(s: &EvalScenario) -> EvalResult {
    let rule = EvalRule::KnowledgeRelevance;

    // Universal ("all"-tagged) chunks must appear in the system prompt.
    let has_universal = knowledge::KNOWLEDGE_BASE
        .iter()
        .any(|c| c.tags.contains("all") && s.system_prompt.contains(c.title));
    if !has_universal {
        return EvalResult::fail(
            rule,
            "No universal (tags: 'all') knowledge chunks in system prompt".into(),
        );
    }

    // For each selected instrument, verify instrument-specific chunks appear
    // (only if such chunks exist in the knowledge base).
    let mut missing_tags = Vec::new();
    for tag in &s.knowledge_tags {
        let specific_exists = knowledge::KNOWLEDGE_BASE
            .iter()
            .any(|c| c.tags.contains(tag.as_str()) && c.tags != "all");
        if !specific_exists {
            continue; // no instrument-specific chunks defined — skip
        }
        let has_specific = knowledge::KNOWLEDGE_BASE.iter().any(|c| {
            c.tags.contains(tag.as_str())
                && c.tags != "all"
                && s.system_prompt.contains(c.title)
        });
        if !has_specific {
            missing_tags.push(tag.as_str());
        }
    }
    if missing_tags.is_empty() {
        EvalResult::pass(rule)
    } else {
        EvalResult::fail(
            rule,
            format!("Missing instrument-specific knowledge for: {missing_tags:?}"),
        )
    }
}

fn check_no_hardcoded_subject(s: &EvalScenario) -> EvalResult {
    let rule = EvalRule::NoHardcodedSubject;
    let prefix = system_prompt_prefix(&s.system_prompt);

    let commodity_names = [
        "Gold",
        "Silver",
        "Bitcoin",
        "Crude Oil",
        "Natural Gas",
        "Copper",
        "Aluminum",
        "Wheat",
        "Corn",
        "Soybeans",
    ];

    // Patterns that indicate a commodity is hardcoded as a hypothesis subject.
    let subject_patterns: &[&str] = &[" will ", " should ", " is expected to "];

    // Known safe occurrences in the ground-truth example.
    let allowlisted = ["gold is around"];

    let prefix_lower = prefix.to_lowercase();
    let mut violations = Vec::new();

    for name in &commodity_names {
        let name_lower = name.to_lowercase();
        for pattern in subject_patterns {
            let needle = format!("{name_lower}{pattern}");
            if let Some(pos) = prefix_lower.find(&needle) {
                let ctx_start = pos.saturating_sub(30);
                let ctx_end = (pos + needle.len() + 30).min(prefix_lower.len());
                let context = &prefix_lower[ctx_start..ctx_end];
                let is_allowed =
                    allowlisted.iter().any(|a| context.contains(a));
                if !is_allowed {
                    violations
                        .push(format!("'{name}' appears as hypothesis subject"));
                }
            }
        }
    }

    if violations.is_empty() {
        EvalResult::pass(rule)
    } else {
        EvalResult::fail(rule, violations.join("; "))
    }
}

fn check_source_attribution_consistent(s: &EvalScenario) -> EvalResult {
    let rule = EvalRule::SourceAttributionConsistent;
    let prefix = system_prompt_prefix(&s.system_prompt);

    // Stale attribution patterns from before the Soybeans migration.
    let stale_patterns = [
        "FRED (VIX, Soybeans)",
        "FRED (VIX and Soybeans)",
        "FRED PSOYBUSDM",
    ];
    for pattern in &stale_patterns {
        if prefix.contains(pattern) {
            return EvalResult::fail(
                rule,
                format!("Found stale FRED attribution: '{pattern}'"),
            );
        }
    }

    // FRED must still be mentioned (for VIX).
    if !prefix.contains("FRED") {
        return EvalResult::fail(rule, "FRED not mentioned in source attribution".into());
    }

    EvalResult::pass(rule)
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

pub fn run_all_checks(scenario: &EvalScenario) -> Vec<EvalResult> {
    vec![
        check_ground_truth_sources(scenario),
        check_primary_subject_match(scenario),
        check_selected_in_latest_closes(scenario),
        check_unselected_tertiary_framing(scenario),
        check_no_selected_in_tertiary(scenario),
        check_no_unselected_in_primary(scenario),
        check_knowledge_relevance(scenario),
        check_no_hardcoded_subject(scenario),
        check_source_attribution_consistent(scenario),
    ]
}

// ===========================================================================
// Layer 2: Response validation (deterministic + LLM judge)
// ===========================================================================

/// Rules checked deterministically against the LLM's analysis response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseRule {
    /// The hypothesis text names at least one selected instrument.
    HypothesisNamesSelected,
    /// No unselected instrument appears as the subject of the hypothesis.
    NoUnselectedAsSubject,
    /// Dollar amounts in the hypothesis are anchored to the latest closes,
    /// not stale training-data prices.
    PriceAnchoring,
}

/// Context for response validation: what the user selected and what the
/// LLM produced.
pub struct ResponseScenario {
    pub selected: Vec<Instrument>,
    /// Authoritative latest close per selected instrument.
    pub latest_closes: Vec<(Instrument, f64)>,
    /// The parsed hypothesis question text.
    pub hypothesis: String,
    /// The parsed outcome strings.
    pub outcomes: Vec<String>,
    /// The parsed hypothesis context.
    pub context: String,
}

impl ResponseScenario {
    /// Build from instrument snapshots and parsed hypothesis fields.
    pub fn from_analysis(
        selected: &[Instrument],
        snapshots: &[InstrumentSnapshot],
        hypothesis: &str,
        outcomes: &[String],
        context: &str,
    ) -> Self {
        let latest_closes: Vec<(Instrument, f64)> = snapshots
            .iter()
            .filter_map(|s| s.latest_close.map(|c| (s.instrument, c)))
            .collect();
        Self {
            selected: selected.to_vec(),
            latest_closes,
            hypothesis: hypothesis.to_owned(),
            outcomes: outcomes.to_vec(),
            context: context.to_owned(),
        }
    }
}

// -- Deterministic response checks ------------------------------------------

fn check_hypothesis_names_selected(s: &ResponseScenario) -> EvalResult {
    let rule_name = "HypothesisNamesSelected";
    let found: Vec<&str> = s
        .selected
        .iter()
        .filter(|i| s.hypothesis.contains(i.as_str()))
        .map(|i| i.as_str())
        .collect();
    if found.is_empty() {
        EvalResult {
            rule: EvalRule::GroundTruthSources, // placeholder — uses ResponseRule below
            pass: false,
            reason: format!(
                "[{rule_name}] Hypothesis does not mention any selected instrument. \
                 Selected: {:?}",
                s.selected.iter().map(|i| i.as_str()).collect::<Vec<_>>()
            ),
        }
    } else {
        EvalResult {
            rule: EvalRule::GroundTruthSources,
            pass: true,
            reason: String::new(),
        }
    }
}

fn check_no_unselected_as_subject(s: &ResponseScenario) -> EvalResult {
    let rule_name = "NoUnselectedAsSubject";
    let unselected = unselected_instruments(&s.selected);
    let hyp_lower = s.hypothesis.to_lowercase();
    let subject_patterns: &[&str] = &[" will ", " should ", " is expected to "];

    let mut violations = Vec::new();
    for inst in &unselected {
        let name_lower = inst.as_str().to_lowercase();
        for pattern in subject_patterns {
            if hyp_lower.contains(&format!("{name_lower}{pattern}")) {
                violations.push(inst.as_str());
                break;
            }
        }
    }
    if violations.is_empty() {
        EvalResult {
            rule: EvalRule::GroundTruthSources,
            pass: true,
            reason: String::new(),
        }
    } else {
        EvalResult {
            rule: EvalRule::GroundTruthSources,
            pass: false,
            reason: format!(
                "[{rule_name}] Unselected instruments appear as hypothesis subject: {violations:?}"
            ),
        }
    }
}

/// Extract dollar amounts from text. Returns (raw_text, value) pairs.
fn extract_dollar_amounts(text: &str) -> Vec<(String, f64)> {
    let mut results = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        if ch == '$' {
            let rest = &text[i + 1..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit() && c != ',' && c != '.')
                .unwrap_or(rest.len());
            let raw = &rest[..end];
            if !raw.is_empty() {
                let cleaned: String = raw.chars().filter(|c| *c != ',').collect();
                if let Ok(val) = cleaned.parse::<f64>() {
                    if val > 0.0 {
                        results.push((format!("${raw}"), val));
                    }
                }
            }
            // Advance past the number.
            for _ in 0..end {
                chars.next();
            }
        }
    }
    results
}

fn check_price_anchoring(s: &ResponseScenario) -> EvalResult {
    let rule_name = "PriceAnchoring";
    let amounts = extract_dollar_amounts(&s.hypothesis);

    if amounts.is_empty() {
        // No dollar amounts in hypothesis — can't validate, pass.
        return EvalResult {
            rule: EvalRule::GroundTruthSources,
            pass: true,
            reason: String::new(),
        };
    }

    // For each dollar amount, check if it's within 25% of any selected
    // instrument's latest close. Amounts that don't match any instrument
    // are flagged — they may be from training-data priors.
    let tolerance = 0.25;
    let mut unanchored = Vec::new();
    for (raw, val) in &amounts {
        let anchored = s.latest_closes.iter().any(|&(_, close)| {
            let ratio = (*val - close).abs() / close;
            ratio <= tolerance
        });
        if !anchored {
            unanchored.push(raw.as_str());
        }
    }

    if unanchored.is_empty() {
        EvalResult {
            rule: EvalRule::GroundTruthSources,
            pass: true,
            reason: String::new(),
        }
    } else {
        EvalResult {
            rule: EvalRule::GroundTruthSources,
            pass: false,
            reason: format!(
                "[{rule_name}] Price(s) not anchored to any latest close (>25% deviation): \
                 {unanchored:?}. Latest closes: {:?}",
                s.latest_closes
                    .iter()
                    .map(|(i, c)| format!("{}: ${c:.2}", i.as_str()))
                    .collect::<Vec<_>>()
            ),
        }
    }
}

/// Unified result type for response validation (both deterministic and judge).
#[derive(Debug, Clone)]
pub struct ResponseValidation {
    pub rule: String,
    pub pass: bool,
    pub reason: String,
}

/// Run all deterministic response checks. Returns results instantly.
pub fn validate_response(scenario: &ResponseScenario) -> Vec<ResponseValidation> {
    let checks = [
        check_hypothesis_names_selected(scenario),
        check_no_unselected_as_subject(scenario),
        check_price_anchoring(scenario),
    ];
    let rule_names = ["HypothesisNamesSelected", "NoUnselectedAsSubject", "PriceAnchoring"];

    checks
        .into_iter()
        .zip(rule_names)
        .map(|(r, name)| ResponseValidation {
            rule: name.to_owned(),
            pass: r.pass,
            reason: r.reason,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// LLM bias judge — prompt template + response parser
// ---------------------------------------------------------------------------

/// Build the (system, user) prompt pair for the LLM bias judge.
///
/// The judge receives the selected instruments, authoritative prices, and the
/// analysis response, then validates against five semantic rules. Output is
/// structured for machine parsing.
pub fn assemble_bias_judge_prompt(
    selected: &[Instrument],
    snapshots: &[InstrumentSnapshot],
    response: &str,
) -> (String, String) {
    let system = "\
You are a quality-control validator for a macro-financial analysis system. You will receive \
the user's selected instruments, authoritative latest prices, and an AI-generated analysis \
to validate.

Check the analysis against these five rules. Be strict.

RULES:

1. SUBJECT_MATCH: The hypothesis MUST be about the selected instruments. If the user \
selected Soybeans, the hypothesis must be about Soybeans — not Gold, not Crude Oil. \
An instrument is 'the subject' if it is the grammatical subject of the hypothesis \
statement and the instrument whose price action is being predicted.

2. PRICE_ANCHORING: Any specific dollar price cited in the hypothesis or outcome bands \
MUST be consistent with the provided latest closes (within ~10%). The model must not \
have substituted stale training-data prices (e.g. citing gold at $2,000 when the latest \
close is $4,624).

3. MECHANISM_RELEVANCE: The causal mechanism in the Hypothesis Context must be specific \
to the selected instruments — not generic macro filler. For example, if the user selected \
Corn, the mechanism should reference ethanol linkage, USDA reports, or crop conditions, \
not just 'monetary policy'.

4. TERTIARY_BOUNDARY: Instruments NOT in the user's selection may appear for supporting \
context but must NOT be the subject of the hypothesis or any outcome band.

5. OUTCOME_ALIGNMENT: Each outcome band must reference the selected instruments' price \
levels and represent a distinct causal path (not just the same outcome at different \
thresholds).

RESPOND IN EXACTLY THIS FORMAT (no other text):
SUBJECT_MATCH: PASS|FAIL — [one sentence reason]
PRICE_ANCHORING: PASS|FAIL — [one sentence reason]
MECHANISM_RELEVANCE: PASS|FAIL — [one sentence reason]
TERTIARY_BOUNDARY: PASS|FAIL — [one sentence reason]
OUTCOME_ALIGNMENT: PASS|FAIL — [one sentence reason]"
        .to_owned();

    let mut user = String::with_capacity(1024);
    user.push_str("## Selected Instruments\n\n");
    let names: Vec<&str> = selected.iter().map(|i| i.as_str()).collect();
    user.push_str(&names.join(", "));
    user.push_str("\n\n## Authoritative Latest Closes\n\n");
    for snap in snapshots {
        if let Some(close) = snap.latest_close {
            user.push_str(&format!("- {}: ${:.2}\n", snap.instrument.as_str(), close));
        }
    }
    user.push_str("\n## Analysis Response to Validate\n\n");
    user.push_str(response);
    user.push_str("\n\n---\nValidate this response against all five rules. Be strict.");

    (system, user)
}

/// Parse the structured output from the LLM bias judge.
pub fn parse_bias_judge(response: &str) -> Vec<ResponseValidation> {
    let rule_names = [
        "SUBJECT_MATCH",
        "PRICE_ANCHORING",
        "MECHANISM_RELEVANCE",
        "TERTIARY_BOUNDARY",
        "OUTCOME_ALIGNMENT",
    ];

    let mut results = Vec::new();
    for rule in rule_names {
        let prefix = format!("{rule}:");
        if let Some(line) = response.lines().find(|l| l.trim_start().starts_with(&prefix)) {
            let after_prefix = line.trim_start().strip_prefix(&prefix).unwrap_or("").trim();
            let pass = after_prefix.starts_with("PASS");
            // Split on em-dash or hyphen. The em-dash is multi-byte
            // UTF-8, so we split on the string rather than slicing by index.
            let reason = if let Some(rest) = after_prefix.split_once('—') {
                rest.1.trim().to_owned()
            } else if let Some(rest) = after_prefix.split_once('-') {
                rest.1.trim().to_owned()
            } else {
                String::new()
            };
            results.push(ResponseValidation {
                rule: rule.to_owned(),
                pass,
                reason,
            });
        } else {
            results.push(ResponseValidation {
                rule: rule.to_owned(),
                pass: false,
                reason: "Judge did not return this rule".to_owned(),
            });
        }
    }
    results
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{self, InstrumentSnapshot};
    use crate::analysis::SpikeEpisode;
    use crate::models::{AlertLevel, Observation, ThresholdSnapshot, VixStatus};
    use crate::storage::Storage;
    use chrono::NaiveDate;

    /// Synthetic observation series: gentle uptrend from base_price over `days`.
    fn synthetic_series(
        instrument: Instrument,
        base_price: f64,
        days: usize,
    ) -> Vec<Observation> {
        let start = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        (0..days)
            .map(|i| Observation {
                instrument,
                date: start + chrono::Duration::days(i as i64),
                close: base_price * (1.0 + 0.001 * (i as f64)),
                source: match instrument {
                    Instrument::Vix => "FRED VIXCLS",
                    Instrument::Gold => "Alpha Vantage GOLD",
                    Instrument::Silver => "Alpha Vantage SILVER",
                    Instrument::Bitcoin => "Alpha Vantage BTC",
                    Instrument::CrudeOil => "Alpha Vantage WTI",
                    Instrument::NaturalGas => "Alpha Vantage NATURAL_GAS",
                    Instrument::Copper => "Alpha Vantage COPPER",
                    Instrument::Aluminum => "Alpha Vantage ALUMINUM",
                    Instrument::Wheat => "Alpha Vantage WHEAT",
                    Instrument::Corn => "Alpha Vantage CORN",
                    Instrument::Soybeans => "Alpha Vantage SOYBEANS",
                },
            })
            .collect()
    }

    fn synthetic_vix(close: f64) -> VixStatus {
        let date = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
        VixStatus {
            latest: Observation {
                instrument: Instrument::Vix,
                date,
                close,
                source: "FRED VIXCLS",
            },
            level: if close > 30.0 {
                AlertLevel::Extreme
            } else if close > 20.0 {
                AlertLevel::ApproachingExtreme
            } else {
                AlertLevel::Normal
            },
            thresholds: ThresholdSnapshot {
                approaching: 20.0,
                extreme: 30.0,
            },
        }
    }

    /// Representative prices for each instrument (structurally valid, not real).
    fn representative_price(instrument: Instrument) -> f64 {
        match instrument {
            Instrument::Vix => 18.0,
            Instrument::Gold => 4624.50,
            Instrument::Silver => 32.10,
            Instrument::Bitcoin => 97500.0,
            Instrument::CrudeOil => 72.50,
            Instrument::NaturalGas => 3.450,
            Instrument::Copper => 4.850,
            Instrument::Aluminum => 2650.0,
            Instrument::Wheat => 5.800,
            Instrument::Corn => 4.500,
            Instrument::Soybeans => 10.250,
        }
    }

    /// Build an `EvalScenario` from a set of selected instruments + VIX level.
    ///
    /// Seeds knowledge into in-memory SQLite, generates synthetic series for
    /// every instrument, and calls the real `assemble_system_prompt` /
    /// `assemble_user_message` functions.
    fn build_scenario(
        name: &'static str,
        selected: &[Instrument],
        vix_close: f64,
    ) -> EvalScenario {
        let storage = Storage::open_memory().expect("in-memory storage");
        storage
            .seed_knowledge_chunks(
                &knowledge::KNOWLEDGE_BASE
                    .iter()
                    .map(|c| (c.title, c.tags, c.body))
                    .collect::<Vec<_>>(),
            )
            .expect("seed knowledge");

        let instrument_tags: Vec<&str> =
            selected.iter().map(|i| i.storage_key()).collect();
        let knowledge_chunks =
            knowledge::retrieve_for_context(&storage, &instrument_tags);

        let instrument_snapshots: Vec<InstrumentSnapshot> = selected
            .iter()
            .map(|&inst| {
                let series =
                    synthetic_series(inst, representative_price(inst), 60);
                InstrumentSnapshot::from_series(inst, &series)
            })
            .collect();

        let unselected: Vec<Instrument> = Instrument::ALL
            .iter()
            .copied()
            .filter(|i| *i != Instrument::Vix && !selected.contains(i))
            .collect();
        let unselected_snapshots: Vec<InstrumentSnapshot> = unselected
            .iter()
            .map(|&inst| {
                let series =
                    synthetic_series(inst, representative_price(inst), 60);
                InstrumentSnapshot::from_series(inst, &series)
            })
            .collect();

        let vix_status = synthetic_vix(vix_close);
        let spike_episodes: Vec<SpikeEpisode> = Vec::new();

        let system_prompt = ai::assemble_system_prompt(&knowledge_chunks);
        let user_message = ai::assemble_user_message(
            Some(&vix_status),
            selected,
            &instrument_snapshots,
            &unselected_snapshots,
            &spike_episodes,
        );

        EvalScenario {
            name,
            selected: selected.to_vec(),
            system_prompt,
            user_message,
            knowledge_tags: instrument_tags
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }

    // -- Full-scenario checks -----------------------------------------------

    #[test]
    fn eval_single_gold() {
        let scenario =
            build_scenario("single_gold", &[Instrument::Gold], 18.2);
        let results = run_all_checks(&scenario);
        for r in &results {
            assert!(
                r.pass,
                "FAIL [{}] {:?} — {}",
                scenario.name, r.rule, r.reason
            );
        }
    }

    #[test]
    fn eval_metals_basket() {
        let scenario = build_scenario(
            "metals_basket",
            &[Instrument::Gold, Instrument::Silver, Instrument::Copper],
            25.0,
        );
        let results = run_all_checks(&scenario);
        for r in &results {
            assert!(
                r.pass,
                "FAIL [{}] {:?} — {}",
                scenario.name, r.rule, r.reason
            );
        }
    }

    #[test]
    fn eval_soybeans_only() {
        let scenario =
            build_scenario("soybeans_only", &[Instrument::Soybeans], 15.0);
        let results = run_all_checks(&scenario);
        for r in &results {
            assert!(
                r.pass,
                "FAIL [{}] {:?} — {}",
                scenario.name, r.rule, r.reason
            );
        }
    }

    #[test]
    fn eval_energy_agriculture_mix() {
        let scenario = build_scenario(
            "energy_agriculture_mix",
            &[
                Instrument::CrudeOil,
                Instrument::NaturalGas,
                Instrument::Wheat,
                Instrument::Corn,
            ],
            30.0,
        );
        let results = run_all_checks(&scenario);
        for r in &results {
            assert!(
                r.pass,
                "FAIL [{}] {:?} — {}",
                scenario.name, r.rule, r.reason
            );
        }
    }

    // -- Targeted regression checks -----------------------------------------

    #[test]
    fn soybeans_not_attributed_to_fred() {
        let scenario = build_scenario(
            "soybeans_regression",
            &[Instrument::Soybeans],
            15.0,
        );
        let result = check_ground_truth_sources(&scenario);
        assert!(
            result.pass,
            "Soybeans attribution regression: {}",
            result.reason
        );
    }

    // -- Response validation tests ------------------------------------------

    fn make_response_scenario(
        selected: &[Instrument],
        hypothesis: &str,
        outcomes: &[&str],
    ) -> ResponseScenario {
        let latest_closes: Vec<(Instrument, f64)> = selected
            .iter()
            .map(|&i| (i, representative_price(i)))
            .collect();
        ResponseScenario {
            selected: selected.to_vec(),
            latest_closes,
            hypothesis: hypothesis.to_owned(),
            outcomes: outcomes.iter().map(|s| s.to_string()).collect(),
            context: String::new(),
        }
    }

    #[test]
    fn response_good_gold_hypothesis() {
        let scenario = make_response_scenario(
            &[Instrument::Gold],
            "Gold will hold above $4,600 through May as real-rate \
             compression holds despite dollar strength.",
            &[
                "Holds above $4,600 — rate compression",
                "Falls below $4,400 — dollar surge",
            ],
        );
        let results = validate_response(&scenario);
        for r in &results {
            assert!(r.pass, "FAIL {} — {}", r.rule, r.reason);
        }
    }

    #[test]
    fn response_wrong_instrument_subject() {
        let scenario = make_response_scenario(
            &[Instrument::Soybeans],
            "Gold will hold above $4,600 through May as safe-haven \
             demand persists.",
            &["Holds above $4,600 — safe haven"],
        );
        let results = validate_response(&scenario);
        let subject = results.iter().find(|r| r.rule == "HypothesisNamesSelected").unwrap();
        assert!(!subject.pass, "Should fail: hypothesis is about Gold, not Soybeans");
    }

    #[test]
    fn response_unselected_as_subject() {
        let scenario = make_response_scenario(
            &[Instrument::Soybeans],
            "Soybeans will hold near $10 but Gold will spike above \
             $5,000 through June.",
            &["Soybeans holds $10", "Gold spikes $5,000"],
        );
        let results = validate_response(&scenario);
        let unsub = results.iter().find(|r| r.rule == "NoUnselectedAsSubject").unwrap();
        assert!(!unsub.pass, "Should fail: Gold (unselected) appears as subject");
    }

    #[test]
    fn response_stale_price() {
        let scenario = make_response_scenario(
            &[Instrument::Gold],
            "Gold will hold above $2,000 through May as monetary \
             easing continues.",
            &["Holds above $2,000 — easing"],
        );
        let results = validate_response(&scenario);
        let price = results.iter().find(|r| r.rule == "PriceAnchoring").unwrap();
        assert!(
            !price.pass,
            "Should fail: $2,000 is far from latest close of $4,624.50"
        );
    }

    #[test]
    fn response_correct_price() {
        let scenario = make_response_scenario(
            &[Instrument::Gold],
            "Gold will hold above $4,500 through May as real-rate \
             compression holds.",
            &["Holds above $4,500"],
        );
        let results = validate_response(&scenario);
        let price = results.iter().find(|r| r.rule == "PriceAnchoring").unwrap();
        assert!(price.pass, "Should pass: $4,500 within 25% of $4,624.50");
    }

    // -- Dollar extraction tests --

    #[test]
    fn extract_dollars_basic() {
        let amounts = extract_dollar_amounts("Gold at $4,624.50 and oil at $72");
        assert_eq!(amounts.len(), 2);
        assert!((amounts[0].1 - 4624.50).abs() < 0.01);
        assert!((amounts[1].1 - 72.0).abs() < 0.01);
    }

    // -- Bias judge parser tests --

    #[test]
    fn parse_judge_all_pass() {
        let response = "\
SUBJECT_MATCH: PASS — Hypothesis correctly targets Gold.
PRICE_ANCHORING: PASS — $4,600 is within range of $4,624.50 close.
MECHANISM_RELEVANCE: PASS — Real-rate compression is specific to gold dynamics.
TERTIARY_BOUNDARY: PASS — No unselected instruments appear as subjects.
OUTCOME_ALIGNMENT: PASS — Three distinct causal paths with appropriate levels.";

        let results = parse_bias_judge(response);
        assert_eq!(results.len(), 5);
        assert!(results.iter().all(|r| r.pass), "All rules should pass");
    }

    #[test]
    fn parse_judge_with_failure() {
        let response = "\
SUBJECT_MATCH: FAIL — Hypothesis is about Crude Oil but user selected Soybeans.
PRICE_ANCHORING: PASS — Prices match latest closes.
MECHANISM_RELEVANCE: FAIL — Generic macro commentary, not soybean-specific.
TERTIARY_BOUNDARY: PASS — Non-selected instruments are background only.
OUTCOME_ALIGNMENT: PASS — Outcomes are distinct.";

        let results = parse_bias_judge(response);
        assert!(!results[0].pass);
        assert!(results[0].reason.contains("Crude Oil"));
        assert!(results[1].pass);
        assert!(!results[2].pass);
    }
}
