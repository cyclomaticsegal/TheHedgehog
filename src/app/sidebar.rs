//! Right-sidebar widget renderers — VIX status panel, overlay-instrument
//! checkboxes, recent-spike list, and threshold-config sliders. All are
//! pure UI: take egui state + plain-data references, no `DashboardApp`
//! coupling.

use super::theme::*;
use crate::analysis;
use crate::models::{
    AlertLevel, AppSettings, AssetGroup, Instrument, ThresholdMode, VixStatus,
};
use eframe::egui::{self, Color32, RichText, Sense, Vec2};

pub(super) fn sidebar_vix_summary(ui: &mut egui::Ui, status: &VixStatus) {
    ui.heading("VIX Status");

    let (color, label) = match status.level {
        AlertLevel::Normal => (ALERT_NORMAL_FG, "Normal"),
        AlertLevel::ApproachingExtreme => (ALERT_APPROACHING_FG, "Approaching Extreme"),
        AlertLevel::Extreme => (ALERT_EXTREME_FG, "EXTREME"),
    };

    ui.horizontal(|ui| {
        let (dot_rect, _) = ui.allocate_exact_size(Vec2::new(14.0, 14.0), Sense::hover());
        ui.painter().circle_filled(dot_rect.center(), 7.0, color);
        ui.label(RichText::new(label).color(color).strong().size(14.0));
    });

    ui.label(format!("Latest: {:.2}", status.latest.close));
    ui.label(format!("Date: {}", status.latest.date));
    ui.label(
        RichText::new(format!(
            "Approaching {:.1}  /  Extreme {:.1}",
            status.thresholds.approaching, status.thresholds.extreme
        ))
        .size(11.0)
        .color(TEXT_MUTED),
    );

    let src = if status.latest.source == "Seeded sample" {
        "example"
    } else {
        "live"
    };
    ui.label(
        RichText::new(format!("Source: {src}"))
            .size(11.0)
            .color(TEXT_MUTED),
    );
}

pub(super) fn sidebar_overlay_controls(ui: &mut egui::Ui, settings: &mut AppSettings) {
    ui.heading("Overlay on VIX");

    let n = settings.overlay_instruments.len();
    ui.label(
        RichText::new(format!(
            "{n} asset{} selected",
            if n == 1 { "" } else { "s" }
        ))
        .size(11.0)
        .color(if n > 0 { ALERT_NORMAL_FG } else { TEXT_MUTED }),
    );

    ui.horizontal_wrapped(|ui| {
        if ui.small_button("Core 3").clicked() {
            settings.overlay_instruments =
                vec![Instrument::Gold, Instrument::Silver, Instrument::Bitcoin];
        }
        if ui.small_button("Energy").clicked() {
            settings.overlay_instruments = vec![Instrument::CrudeOil, Instrument::NaturalGas];
        }
        if ui.small_button("Metals").clicked() {
            settings.overlay_instruments = vec![Instrument::Gold, Instrument::Silver];
        }
        if ui.small_button("All").clicked() {
            settings.overlay_instruments = Instrument::ALL
                .iter()
                .copied()
                .filter(|i| *i != Instrument::Vix)
                .collect();
        }
        if ui.small_button("Clear").clicked() {
            settings.overlay_instruments.clear();
        }
    });

    ui.add_space(4.0);

    for group in AssetGroup::ALL {
        if group == AssetGroup::Volatility {
            continue;
        }
        ui.label(RichText::new(group.label()).size(11.0).color(TEXT_MUTED));
        for instrument in Instrument::group_members(group) {
            let mut enabled = settings.overlay_instruments.contains(instrument);
            let color = instrument_color(*instrument);
            ui.horizontal(|ui| {
                let (swatch_rect, _) =
                    ui.allocate_exact_size(Vec2::new(10.0, 10.0), Sense::hover());
                ui.painter().rect_filled(swatch_rect, 2.0, color);
                if ui.checkbox(&mut enabled, instrument.as_str()).changed() {
                    if enabled {
                        settings.overlay_instruments.push(*instrument);
                    } else {
                        settings.overlay_instruments.retain(|i| i != instrument);
                    }
                }
            });
        }
    }
}

pub(super) fn sidebar_spike_episodes(
    ui: &mut egui::Ui,
    episodes: &[analysis::SpikeEpisode],
    highlighted: &mut Option<(chrono::NaiveDate, chrono::NaiveDate)>,
) {
    ui.heading("Recent Spikes");

    if episodes.is_empty() {
        ui.label(
            RichText::new("No spike episodes detected.")
                .size(11.0)
                .color(TEXT_MUTED),
        );
        return;
    }

    for ep in episodes {
        let level_color = match ep.max_level {
            AlertLevel::Normal => ALERT_NORMAL_FG,
            AlertLevel::ApproachingExtreme => ALERT_APPROACHING_FG,
            AlertLevel::Extreme => ALERT_EXTREME_FG,
        };
        let is_selected = *highlighted == Some((ep.start, ep.end));

        let bg_idx = ui.painter().add(egui::Shape::Noop);
        let stripe_idx = ui.painter().add(egui::Shape::Noop);

        let row_resp = ui.horizontal_wrapped(|ui| {
            let (circle_rect, _) =
                ui.allocate_exact_size(Vec2::new(12.0, 12.0), Sense::hover());
            ui.painter()
                .circle_filled(circle_rect.center(), 5.0, level_color);
            ui.label(
                RichText::new(format!(
                    "{} to {} | peak {:.1} | {}d",
                    ep.start.format("%b %d"),
                    ep.end.format("%b %d"),
                    ep.peak,
                    ep.duration_points,
                ))
                .size(11.0),
            );
        });

        let row_rect = row_resp.response.rect;
        let row_sense = ui.allocate_rect(row_rect, Sense::click());

        if is_selected {
            ui.painter().set(
                bg_idx,
                egui::Shape::rect_filled(
                    row_rect,
                    4.0,
                    Color32::from_rgba_unmultiplied(59, 130, 246, 28),
                ),
            );
            let stripe = egui::Rect::from_min_size(
                row_rect.left_top(),
                Vec2::new(3.0, row_rect.height()),
            );
            ui.painter()
                .set(stripe_idx, egui::Shape::rect_filled(stripe, 1.5, ACCENT_BLUE));
        }

        if row_sense.hovered() {
            if !is_selected {
                ui.painter().rect_filled(
                    row_rect,
                    4.0,
                    Color32::from_rgba_unmultiplied(255, 255, 255, 8),
                );
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        if row_sense.double_clicked() {
            *highlighted = None;
        } else if row_sense.clicked() {
            *highlighted = Some((ep.start, ep.end));
        }
    }
}

pub(super) fn sidebar_threshold_controls(ui: &mut egui::Ui, settings: &mut AppSettings) {
    ui.horizontal(|ui| {
        ui.selectable_value(
            &mut settings.threshold_config.mode,
            ThresholdMode::RollingPercentile,
            "Percentile",
        );
        ui.selectable_value(
            &mut settings.threshold_config.mode,
            ThresholdMode::Fixed,
            "Fixed",
        );
    });

    ui.add(
        egui::Slider::new(&mut settings.threshold_config.lookback_days, 60..=504).text("Lookback"),
    );

    match settings.threshold_config.mode {
        ThresholdMode::RollingPercentile => {
            ui.add(
                egui::Slider::new(
                    &mut settings.threshold_config.percentile_approaching,
                    50.0..=99.0,
                )
                .text("Approaching %"),
            );
            ui.add(
                egui::Slider::new(
                    &mut settings.threshold_config.percentile_extreme,
                    70.0..=99.9,
                )
                .text("Extreme %"),
            );
        }
        ThresholdMode::Fixed => {
            ui.add(
                egui::Slider::new(
                    &mut settings.threshold_config.fixed_approaching,
                    10.0..=60.0,
                )
                .text("Approaching"),
            );
            ui.add(
                egui::Slider::new(&mut settings.threshold_config.fixed_extreme, 12.0..=80.0)
                    .text("Extreme"),
            );
        }
    }
}
