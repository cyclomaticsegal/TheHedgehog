use crate::models::{Instrument, Observation, ObservationBatch, RefreshEvent};
use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

// Alpha Vantage premium tier (75 req/min): 60000 / 75 = 800ms.
// 850ms gives a small safety margin against jitter (≈70 req/min effective).
const ALPHA_VANTAGE_REQUEST_INTERVAL: Duration = Duration::from_millis(850);

pub fn refresh_market_data(
    fred_api_key: &str,
    alpha_vantage_api_key: &str,
    tx: Sender<RefreshEvent>,
    cached_dates_fred: HashMap<Instrument, NaiveDate>,
    cached_dates_alpha: HashMap<Instrument, NaiveDate>,
) {
    let client = Client::builder()
        .user_agent(crate::USER_AGENT)
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .danger_accept_invalid_certs(false)
        .build();

    let Ok(client) = client else {
        let _ = tx.send(RefreshEvent::FetchFailed {
            instrument: Instrument::Vix,
            source: "Unknown".to_string(),
            error: "Failed to initialize HTTP client.".to_owned(),
        });
        let _ = tx.send(RefreshEvent::Done);
        return;
    };

    let today = chrono::Local::now().date_naive();

    // Fetch from FRED (VIX)
    for spec in build_specs_fred() {
        if let Some(&cached_date) = cached_dates_fred.get(&spec.instrument) {
            if cached_date == today {
                let _ = tx.send(RefreshEvent::Cached {
                    instrument: spec.instrument,
                    source: spec.source.to_string(),
                    date: today.format("%Y-%m-%d").to_string(),
                });
                continue;
            }
        }

        let _ = tx.send(RefreshEvent::Fetching {
            instrument: spec.instrument,
            source: spec.source.to_string(),
        });
        match fetch_batch(
            &client,
            &spec,
            fred_api_key,
            alpha_vantage_api_key,
        ) {
            Ok(batch) => {
                let _ = tx.send(RefreshEvent::Fetched(batch));
            }
            Err(err) => {
                let _ = tx.send(RefreshEvent::FetchFailed {
                    instrument: spec.instrument,
                    source: spec.source.to_string(),
                    error: redact_url_key(&format!("{err:#}")),
                });
            }
        }
    }

    // Fetch from Alpha Vantage (commodities)
    let mut last_was_alpha = false;
    for spec in build_specs_alpha_vantage() {
        if let Some(&cached_date) = cached_dates_alpha.get(&spec.instrument) {
            if cached_date == today {
                let _ = tx.send(RefreshEvent::Cached {
                    instrument: spec.instrument,
                    source: spec.source.to_string(),
                    date: today.format("%Y-%m-%d").to_string(),
                });
                continue;
            }
        }

        let _ = tx.send(RefreshEvent::Fetching {
            instrument: spec.instrument,
            source: spec.source.to_string(),
        });
        if last_was_alpha {
            thread::sleep(ALPHA_VANTAGE_REQUEST_INTERVAL);
        }
        match fetch_batch(
            &client,
            &spec,
            fred_api_key,
            alpha_vantage_api_key,
        ) {
            Ok(batch) => {
                let _ = tx.send(RefreshEvent::Fetched(batch));
            }
            Err(err) => {
                let _ = tx.send(RefreshEvent::FetchFailed {
                    instrument: spec.instrument,
                    source: spec.source.to_string(),
                    error: redact_url_key(&format!("{err:#}")),
                });
            }
        }
        last_was_alpha = true;
    }

    let _ = tx.send(RefreshEvent::Done);
}

#[derive(Clone, Copy)]
struct ProviderSpec {
    instrument: Instrument,
    source: &'static str,
    kind: ProviderKind,
}

#[derive(Clone, Copy)]
enum ProviderKind {
    Fred {
        series_id: &'static str,
    },
    AlphaCommodity {
        function: &'static str,
        interval: &'static str,
    },
    AlphaMetal {
        symbol: &'static str,
    },
    AlphaDigitalCurrency {
        symbol: &'static str,
        market: &'static str,
    },
}

fn build_specs_fred() -> Vec<ProviderSpec> {
    vec![
        ProviderSpec {
            instrument: Instrument::Vix,
            source: "FRED VIXCLS",
            kind: ProviderKind::Fred {
                series_id: "VIXCLS",
            },
        },
    ]
}

fn build_specs_alpha_vantage() -> Vec<ProviderSpec> {
    vec![
        ProviderSpec {
            instrument: Instrument::Soybeans,
            source: "Alpha Vantage SOYBEANS",
            kind: ProviderKind::AlphaCommodity {
                function: "SOYBEANS",
                interval: "daily",
            },
        },
        ProviderSpec {
            instrument: Instrument::Gold,
            source: "Alpha Vantage GOLD",
            kind: ProviderKind::AlphaMetal { symbol: "GOLD" },
        },
        ProviderSpec {
            instrument: Instrument::Silver,
            source: "Alpha Vantage SILVER",
            kind: ProviderKind::AlphaMetal { symbol: "SILVER" },
        },
        ProviderSpec {
            instrument: Instrument::Bitcoin,
            source: "Alpha Vantage BTC",
            kind: ProviderKind::AlphaDigitalCurrency {
                symbol: "BTC",
                market: "USD",
            },
        },
        ProviderSpec {
            instrument: Instrument::CrudeOil,
            source: "Alpha Vantage WTI",
            kind: ProviderKind::AlphaCommodity {
                function: "WTI",
                interval: "daily",
            },
        },
        ProviderSpec {
            instrument: Instrument::NaturalGas,
            source: "Alpha Vantage NATURAL_GAS",
            kind: ProviderKind::AlphaCommodity {
                function: "NATURAL_GAS",
                interval: "daily",
            },
        },
        ProviderSpec {
            instrument: Instrument::Copper,
            source: "Alpha Vantage COPPER",
            kind: ProviderKind::AlphaCommodity {
                function: "COPPER",
                interval: "daily",
            },
        },
        ProviderSpec {
            instrument: Instrument::Aluminum,
            source: "Alpha Vantage ALUMINUM",
            kind: ProviderKind::AlphaCommodity {
                function: "ALUMINUM",
                interval: "daily",
            },
        },
        ProviderSpec {
            instrument: Instrument::Wheat,
            source: "Alpha Vantage WHEAT",
            kind: ProviderKind::AlphaCommodity {
                function: "WHEAT",
                interval: "daily",
            },
        },
        ProviderSpec {
            instrument: Instrument::Corn,
            source: "Alpha Vantage CORN",
            kind: ProviderKind::AlphaCommodity {
                function: "CORN",
                interval: "daily",
            },
        },
    ]
}

fn fetch_batch(
    client: &Client,
    spec: &ProviderSpec,
    fred_api_key: &str,
    alpha_vantage_api_key: &str,
) -> Result<ObservationBatch> {
    let observations = match spec.kind {
        ProviderKind::Fred { series_id } => fetch_fred_series(
            client,
            spec.instrument,
            spec.source,
            series_id,
            fred_api_key,
        )?,
        ProviderKind::AlphaCommodity { function, interval } => fetch_alpha_commodity(
            client,
            spec.instrument,
            spec.source,
            function,
            interval,
            alpha_vantage_api_key,
        )?,
        ProviderKind::AlphaMetal { symbol } => fetch_alpha_metal(
            client,
            spec.instrument,
            spec.source,
            symbol,
            alpha_vantage_api_key,
        )?,
        ProviderKind::AlphaDigitalCurrency { symbol, market } => fetch_alpha_digital_currency(
            client,
            spec.instrument,
            spec.source,
            symbol,
            market,
            alpha_vantage_api_key,
        )?,
    };

    Ok(ObservationBatch {
        instrument: spec.instrument,
        source: spec.source,
        observations,
    })
}

fn fetch_fred_series(
    client: &Client,
    instrument: Instrument,
    source: &'static str,
    series_id: &str,
    fred_api_key: &str,
) -> Result<Vec<Observation>> {
    let mut request = client
        .get("https://api.stlouisfed.org/fred/series/observations")
        .query(&[("series_id", series_id), ("file_type", "json")]);

    if !fred_api_key.trim().is_empty() {
        request = request.query(&[("api_key", fred_api_key.trim())]);
    }

    let payload: Value = request
        .send()
        .context("request to FRED failed")?
        .error_for_status()
        .context("FRED returned an error status")?
        .json()
        .context("failed to parse FRED JSON response")?;

    if let Some(message) = payload.get("error_message").and_then(Value::as_str) {
        return Err(anyhow!(message.to_owned()));
    }

    let rows = payload
        .get("observations")
        .and_then(Value::as_array)
        .context("FRED observations array missing")?;

    let mut observations = Vec::new();
    for row in rows {
        let Some(date_str) = row.get("date").and_then(Value::as_str) else {
            continue;
        };
        let Some(value_str) = row.get("value").and_then(Value::as_str) else {
            continue;
        };
        if value_str == "." {
            continue;
        }

        let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
            .with_context(|| format!("invalid FRED date: {date_str}"))?;
        let close = value_str
            .parse::<f64>()
            .with_context(|| format!("invalid FRED value: {value_str}"))?;

        if !close.is_finite() {
            continue;
        }

        observations.push(Observation {
            instrument,
            date,
            close,
            source,
        });
    }

    ensure_non_empty(instrument, observations)
}

fn fetch_alpha_commodity(
    client: &Client,
    instrument: Instrument,
    source: &'static str,
    function: &str,
    interval: &str,
    api_key: &str,
) -> Result<Vec<Observation>> {
    let key = alpha_vantage_key(api_key)?;
    let payload: Value = client
        .get("https://www.alphavantage.co/query")
        .query(&[
            ("function", function),
            ("interval", interval),
            ("apikey", key),
        ])
        .send()
        .with_context(|| format!("request to Alpha Vantage {function} failed"))?
        .error_for_status()
        .context("Alpha Vantage returned an error status")?
        .json()
        .context("failed to parse Alpha Vantage commodity response")?;

    check_alpha_vantage_error(&payload)?;
    let observations = parse_data_array_series(&payload, instrument, source)?;
    ensure_non_empty(instrument, observations)
}

fn fetch_alpha_metal(
    client: &Client,
    instrument: Instrument,
    source: &'static str,
    symbol: &str,
    api_key: &str,
) -> Result<Vec<Observation>> {
    let key = alpha_vantage_key(api_key)?;
    let payload: Value = client
        .get("https://www.alphavantage.co/query")
        .query(&[
            ("function", "GOLD_SILVER_HISTORY"),
            ("symbol", symbol),
            ("interval", "daily"),
            ("apikey", key),
        ])
        .send()
        .context("request to Alpha Vantage metal endpoint failed")?
        .error_for_status()
        .context("Alpha Vantage returned an error status")?
        .json()
        .context("failed to parse Alpha Vantage metal response")?;

    check_alpha_vantage_error(&payload)?;
    let observations = parse_data_array_series(&payload, instrument, source)?;
    ensure_non_empty(instrument, observations)
}

fn fetch_alpha_digital_currency(
    client: &Client,
    instrument: Instrument,
    source: &'static str,
    symbol: &str,
    market: &str,
    api_key: &str,
) -> Result<Vec<Observation>> {
    let key = alpha_vantage_key(api_key)?;
    let payload: Value = client
        .get("https://www.alphavantage.co/query")
        .query(&[
            ("function", "DIGITAL_CURRENCY_DAILY"),
            ("symbol", symbol),
            ("market", market),
            ("apikey", key),
        ])
        .send()
        .context("request to Alpha Vantage bitcoin endpoint failed")?
        .error_for_status()
        .context("Alpha Vantage returned an error status")?
        .json()
        .context("failed to parse Alpha Vantage bitcoin response")?;

    check_alpha_vantage_error(&payload)?;

    let rows = payload
        .get("Time Series (Digital Currency Daily)")
        .and_then(Value::as_object)
        .context("bitcoin time-series payload missing")?;

    let mut observations = Vec::new();
    for (date_str, value) in rows {
        let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
            .with_context(|| format!("invalid bitcoin date: {date_str}"))?;
        let object = value
            .as_object()
            .context("bitcoin payload row was not an object")?;
        let close = extract_close_value(object)
            .with_context(|| format!("missing bitcoin close value for {date_str}"))?;
        if !close.is_finite() || close <= 0.0 {
            continue;
        }
        observations.push(Observation {
            instrument,
            date,
            close,
            source,
        });
    }

    ensure_non_empty(instrument, observations)
}

fn parse_data_array_series(
    payload: &Value,
    instrument: Instrument,
    source: &'static str,
) -> Result<Vec<Observation>> {
    let rows = payload
        .get("data")
        .and_then(Value::as_array)
        .context("expected a data array in Alpha Vantage response")?;

    let mut observations = Vec::new();
    for item in rows {
        let Some(date_str) = item
            .get("date")
            .and_then(Value::as_str)
            .or_else(|| item.get("timestamp").and_then(Value::as_str))
        else {
            continue;
        };

        // Skip placeholder "." values (missing data points)
        if item.get("value").and_then(Value::as_str) == Some(".") {
            continue;
        }

        let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
            .with_context(|| format!("invalid commodity date: {date_str}"))?;
        let close = extract_close_from_value(item)
            .with_context(|| format!("invalid commodity value for {date_str}"))?;

        if !close.is_finite() || close <= 0.0 {
            continue;
        }

        observations.push(Observation {
            instrument,
            date,
            close,
            source,
        });
    }

    Ok(observations)
}

fn alpha_vantage_key(value: &str) -> Result<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(anyhow!(
            "Alpha Vantage API key is required for non-FRED commodity and crypto series"
        ))
    } else {
        Ok(trimmed)
    }
}

fn check_alpha_vantage_error(payload: &Value) -> Result<()> {
    for key in ["Information", "Note", "Error Message"] {
        if let Some(message) = payload.get(key).and_then(Value::as_str) {
            return Err(anyhow!(message.to_owned()));
        }
    }
    Ok(())
}

/// Returns true when `haystack` contains `needle` (already lowercase ASCII) as a
/// case-insensitive substring, without allocating.
fn contains_ci(haystack: &str, needle: &str) -> bool {
    let nb = needle.as_bytes();
    if nb.is_empty() {
        return true;
    }
    haystack
        .as_bytes()
        .windows(nb.len())
        .any(|w| w.iter().zip(nb).all(|(h, n)| h.to_ascii_lowercase() == *n))
}

fn extract_close_value(object: &Map<String, Value>) -> Result<f64> {
    // Single pass: prefer a key that matches both "close" and "usd"; remember
    // the first "close"-only match as a fallback.  Avoids two separate loops
    // and eliminates the per-key String allocation from to_ascii_lowercase().
    let mut close_only: Option<&Value> = None;
    for (key, value) in object {
        if contains_ci(key, "close") {
            if contains_ci(key, "usd") {
                return parse_numeric_value(value);
            }
            if close_only.is_none() {
                close_only = Some(value);
            }
        }
    }
    if let Some(value) = close_only {
        return parse_numeric_value(value);
    }
    Err(anyhow!("close value not found"))
}

fn extract_close_from_value(value: &Value) -> Result<f64> {
    if let Some(number) = value.as_f64() {
        return Ok(number);
    }
    if let Some(text) = value.as_str() {
        return text
            .parse::<f64>()
            .with_context(|| format!("invalid numeric value: {text}"));
    }
    if let Some(object) = value.as_object() {
        for key in ["value", "price", "close"] {
            if let Some(candidate) = object.get(key) {
                return parse_numeric_value(candidate);
            }
        }
        return extract_close_value(object);
    }

    Err(anyhow!("unsupported numeric value shape"))
}

fn parse_numeric_value(value: &Value) -> Result<f64> {
    if let Some(number) = value.as_f64() {
        Ok(number)
    } else if let Some(text) = value.as_str() {
        text.parse::<f64>()
            .with_context(|| format!("invalid numeric value: {text}"))
    } else {
        Err(anyhow!("numeric field was neither string nor number"))
    }
}

/// Redact API key values from URL query strings before surfacing errors to the
/// user. Alpha Vantage passes the key as `apikey=VALUE` in the request URL,
/// which can appear verbatim in reqwest error messages.
fn redact_url_key(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let needle = "apikey=";
    let Some(pos) = lower.find(needle) else {
        return s.to_owned();
    };
    let value_start = pos + needle.len();
    let value_end = s[value_start..]
        .find(|c: char| c == '&' || c == ' ' || c == '"' || c == '\n')
        .map(|i| value_start + i)
        .unwrap_or(s.len());
    format!("{}***{}", &s[..value_start], &s[value_end..])
}

fn ensure_non_empty(
    instrument: Instrument,
    mut observations: Vec<Observation>,
) -> Result<Vec<Observation>> {
    if observations.is_empty() {
        return Err(anyhow!(
            "provider returned zero usable {instrument} observations"
        ));
    }
    observations.sort_by_key(|obs| obs.date);
    Ok(observations)
}
