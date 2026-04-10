use crate::models::{
    AlertLevel, ChartWindow, Observation, ThresholdConfig, ThresholdMode, ThresholdSnapshot,
    VixStatus,
};
use chrono::{Datelike, Duration, NaiveDate};

#[derive(Debug, Clone)]
pub struct SpikeEpisode {
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub peak: f64,
    pub duration_points: usize,
    pub max_level: AlertLevel,
}

#[allow(dead_code)]
pub fn filter_window(observations: &[Observation], window: ChartWindow) -> Vec<Observation> {
    let Some(last) = observations.last() else {
        return Vec::new();
    };

    match window.approx_days() {
        Some(days) => {
            let cutoff = last.date - Duration::days(days as i64);
            observations
                .iter()
                .filter(|obs| obs.date >= cutoff)
                .cloned()
                .collect()
        }
        None => observations.to_vec(),
    }
}

pub fn normalize_series(observations: &[Observation]) -> Vec<(f64, f64)> {
    let Some(first) = observations.first() else {
        return Vec::new();
    };

    if first.close <= f64::EPSILON {
        return Vec::new();
    }

    observations
        .iter()
        .map(|obs| (date_to_x(obs.date), (obs.close / first.close) * 100.0))
        .collect()
}

pub fn raw_series(observations: &[Observation]) -> Vec<(f64, f64)> {
    observations
        .iter()
        .map(|obs| (date_to_x(obs.date), obs.close))
        .collect()
}

pub fn date_to_x(date: NaiveDate) -> f64 {
    date.num_days_from_ce() as f64
}

pub fn compute_vix_status(
    observations: &[Observation],
    config: &ThresholdConfig,
) -> Option<VixStatus> {
    let latest = observations.last()?.clone();
    let window_size = config.lookback_days.max(5);
    let slice_start = observations.len().saturating_sub(window_size);
    let sample = &observations[slice_start..];
    let closes: Vec<f64> = sample.iter().map(|obs| obs.close).collect();
    let thresholds = match config.mode {
        ThresholdMode::RollingPercentile => ThresholdSnapshot {
            approaching: percentile(&closes, config.percentile_approaching / 100.0),
            extreme: percentile(&closes, config.percentile_extreme / 100.0),
        },
        ThresholdMode::Fixed => ThresholdSnapshot {
            approaching: config.fixed_approaching,
            extreme: config.fixed_extreme,
        },
    };

    let level = if latest.close >= thresholds.extreme {
        AlertLevel::Extreme
    } else if latest.close >= thresholds.approaching {
        AlertLevel::ApproachingExtreme
    } else {
        AlertLevel::Normal
    };

    Some(VixStatus {
        latest,
        level,
        thresholds,
    })
}

pub fn recent_spike_episodes(
    observations: &[Observation],
    config: &ThresholdConfig,
    limit: usize,
) -> Vec<SpikeEpisode> {
    if observations.is_empty() {
        return Vec::new();
    }

    let window_size = config.lookback_days.max(5);
    let mut current: Option<SpikeEpisode> = None;
    let mut episodes = Vec::new();

    // For Fixed mode the thresholds are constant; compute once and reuse.
    let fixed_thresholds: Option<(f64, f64)> = match config.mode {
        ThresholdMode::Fixed => Some((config.fixed_approaching, config.fixed_extreme)),
        ThresholdMode::RollingPercentile => None,
    };

    // Reusable buffer for rolling-percentile windows — avoids one Vec allocation
    // per iteration (was previously two: one per percentile call).
    let mut closes_buf: Vec<f64> = Vec::with_capacity(window_size);

    for idx in 0..observations.len() {
        let obs = &observations[idx];
        let thresholds = if let Some((approaching, extreme)) = fixed_thresholds {
            ThresholdSnapshot { approaching, extreme }
        } else {
            let slice_start = idx.saturating_sub(window_size.saturating_sub(1));
            closes_buf.clear();
            closes_buf.extend(observations[slice_start..=idx].iter().map(|p| p.close));
            // Sort once and compute both percentiles from the same sorted buffer,
            // halving the number of sorts compared to two separate percentile() calls.
            closes_buf.sort_by(|a, b| a.total_cmp(b));
            ThresholdSnapshot {
                approaching: percentile_of_sorted(
                    &closes_buf,
                    config.percentile_approaching / 100.0,
                ),
                extreme: percentile_of_sorted(&closes_buf, config.percentile_extreme / 100.0),
            }
        };

        let level = if obs.close >= thresholds.extreme {
            AlertLevel::Extreme
        } else if obs.close >= thresholds.approaching {
            AlertLevel::ApproachingExtreme
        } else {
            AlertLevel::Normal
        };

        match (&mut current, level) {
            (None, AlertLevel::Normal) => {}
            (None, _) => {
                current = Some(SpikeEpisode {
                    start: obs.date,
                    end: obs.date,
                    peak: obs.close,
                    duration_points: 1,
                    max_level: level,
                });
            }
            (Some(episode), AlertLevel::Normal) => {
                episodes.push(episode.clone());
                current = None;
            }
            (Some(episode), _) => {
                episode.end = obs.date;
                episode.peak = episode.peak.max(obs.close);
                episode.duration_points += 1;
                if level == AlertLevel::Extreme {
                    episode.max_level = AlertLevel::Extreme;
                }
            }
        }
    }

    if let Some(episode) = current {
        episodes.push(episode);
    }

    episodes.reverse();
    episodes.truncate(limit);
    episodes
}

fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    percentile_of_sorted(&sorted, p)
}

/// Compute the `p`-th quantile (0.0–1.0) from a **pre-sorted** slice.
/// Callers that need multiple quantiles from the same data should sort once
/// and call this directly, rather than going through `percentile()`.
fn percentile_of_sorted(sorted: &[f64], p: f64) -> f64 {
    match sorted.len() {
        0 => 0.0,
        1 => sorted[0],
        n => {
            let rank = p.clamp(0.0, 1.0) * (n - 1) as f64;
            let low = rank.floor() as usize;
            let high = rank.ceil() as usize;
            if low == high {
                sorted[low]
            } else {
                let weight = rank - low as f64;
                sorted[low] * (1.0 - weight) + sorted[high] * weight
            }
        }
    }
}
