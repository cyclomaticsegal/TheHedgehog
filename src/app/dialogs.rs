//! Reusable modal dialogs, banners, and small layout primitives used
//! across the dashboard. Each function is pure UI — egui state in,
//! click/confirm signal out.

use super::theme::*;
use crate::models::{AlertLevel, Instrument, VixStatus};
use eframe::egui::{
    self, Align2, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};

// ---------------------------------------------------------------------------
// Small leaf widgets
// ---------------------------------------------------------------------------

pub(super) fn api_key_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    api_key_field_with_hint(ui, label, value, "")
}

pub(super) fn api_key_field_with_hint(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    hint: &str,
) {
    let is_set = !value.trim().is_empty();
    ui.horizontal(|ui| {
        ui.label(label);
        let (marker, color) = if is_set {
            ("● set", ALERT_NORMAL_FG)
        } else {
            ("○ not set", ALERT_EXTREME_FG)
        };
        ui.label(RichText::new(marker).size(10.0).color(color));
    });
    let mut edit = egui::TextEdit::singleline(value).password(true);
    if !hint.is_empty() {
        edit = edit.hint_text(hint);
    }
    ui.add(edit);
}

/// Back-navigation button used in the 51Folds model explorer detail pages.
pub(super) fn back_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(format!("\u{276E}  {label}"))
                .size(14.0)
                .color(ACCENT_BLUE),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(egui::Stroke::NONE),
    )
}

/// Dark surface card used throughout the 51Folds model explorer.
pub(super) fn section_card<R>(
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::default()
        .fill(SURFACE)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(18, 16))
        .show(ui, add_contents)
        .inner
}

pub(super) fn empty_state_panel(ui: &mut egui::Ui, refreshing: bool) {
    ui.add_space(80.0);
    ui.vertical_centered(|ui| {
        ui.heading(RichText::new("No Market Data Loaded").size(20.0));
        ui.add_space(16.0);

        if refreshing {
            ui.spinner();
            ui.label("Fetching data...");
        } else {
            egui::Frame::default()
                .fill(SURFACE)
                .corner_radius(8.0)
                .inner_margin(egui::Margin::same(20))
                .show(ui, |ui| {
                    ui.label(RichText::new("To get started:").strong().size(14.0));
                    ui.add_space(8.0);
                    ui.label("1. Open \"API Keys\" in the sidebar");
                    ui.label("2. Enter your FRED and/or Alpha Vantage API keys");
                    ui.label("3. Click \"Refresh\" for live market data");
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Free FRED key: fred.stlouisfed.org (Account > API Keys)  |  Free Alpha Vantage key: alphavantage.co/support/#api-key")
                            .size(11.0)
                            .color(TEXT_MUTED),
                    );
                });
        }
    });
}

// ---------------------------------------------------------------------------
// Banners and toasts
// ---------------------------------------------------------------------------

/// Full-width banner shown at the top of the 51Folds model explorer
/// while a driver re-evaluate is in flight.
pub(super) fn render_reeval_in_flight_banner(ui: &mut egui::Ui) {
    egui::Frame::default()
        .fill(SURFACE_HOVER)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Re-evaluating with your driver changes\u{2026}")
                            .size(14.0)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(
                            "Driver edits are locked while 51Folds recomputes outcome probabilities. This usually takes a few seconds.",
                        )
                        .size(12.0)
                        .color(TEXT_SECONDARY),
                    );
                });
            });
        });
}

/// Red error banner shown at the top of the 51Folds model explorer when
/// a re-evaluate failed.
pub(super) fn render_reeval_error_banner(ui: &mut egui::Ui, err: &str) {
    egui::Frame::default()
        .fill(Color32::from_rgb(60, 20, 25))
        .stroke(egui::Stroke::new(1.0, ALERT_EXTREME_FG))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Re-evaluation failed")
                        .size(14.0)
                        .strong()
                        .color(ALERT_EXTREME_FG),
                );
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(err).size(12.0).color(TEXT_PRIMARY),
                    )
                    .wrap(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Your driver edits are preserved. Click Re-evaluate on the Drivers tab to retry.",
                    )
                    .size(11.0)
                    .color(TEXT_SECONDARY),
                );
            });
        });
}

/// Fading success toast shown at the top of the Outcome tab for a few
/// seconds after a re-evaluation completes.
pub(super) fn render_reeval_success_toast(ui: &mut egui::Ui, fade_out: f32) {
    let alpha_f = (1.0 - fade_out).clamp(0.0, 1.0);
    let fade = |c: Color32| -> Color32 {
        let a = (c.a() as f32 * alpha_f) as u8;
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
    };
    egui::Frame::default()
        .fill(fade(Color32::from_rgb(18, 52, 36)))
        .stroke(egui::Stroke::new(1.0, fade(ALERT_NORMAL_FG)))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(16, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("\u{2713}")
                        .size(16.0)
                        .strong()
                        .color(fade(ALERT_NORMAL_FG)),
                );
                ui.add_space(8.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "Outcome probabilities updated from your driver edits. Rows below show before / after deltas.",
                        )
                        .size(13.0)
                        .color(fade(Color32::WHITE)),
                    )
                    .wrap(),
                );
            });
        });
}

/// Returns `true` when the user clicks the Analyze button in the banner.
pub(super) fn status_banner(ui: &mut egui::Ui, status: &VixStatus, ai_in_flight: bool) -> bool {
    let accent = match status.level {
        AlertLevel::Normal => ALERT_NORMAL_FG,
        AlertLevel::ApproachingExtreme => ALERT_APPROACHING_FG,
        AlertLevel::Extreme => ALERT_EXTREME_FG,
    };

    let mut analyze_clicked = false;

    let outer = egui::Frame::default()
        .fill(SURFACE)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "VIX {:.2} - {}",
                        status.latest.close,
                        status.level.label()
                    ))
                    .color(accent)
                    .strong()
                    .size(16.0),
                );
                ui.label(
                    RichText::new(format!(
                        "Thresholds: {:.1} / {:.1}",
                        status.thresholds.approaching, status.thresholds.extreme
                    ))
                    .color(TEXT_SECONDARY)
                    .size(12.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ai_in_flight {
                        ui.spinner();
                        ui.label(
                            RichText::new("Analyzing...")
                                .size(11.0)
                                .color(TEXT_MUTED),
                        );
                    } else if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Analyze")
                                    .size(11.0)
                                    .color(ACCENT_BLUE),
                            )
                            .fill(PANEL_BG)
                            .stroke(egui::Stroke::new(1.0, ACCENT_BLUE_DIM))
                            .corner_radius(4.0),
                        )
                        .on_hover_text("Run AI analysis on the current view")
                        .clicked()
                    {
                        analyze_clicked = true;
                    }
                });
            });
        });

    let r = outer.response.rect;
    ui.painter().rect_filled(
        Rect::from_min_max(r.min, Pos2::new(r.min.x + 3.0, r.max.y)),
        egui::CornerRadius::same(6),
        accent,
    );

    analyze_clicked
}

// ---------------------------------------------------------------------------
// Confirmation modals
// ---------------------------------------------------------------------------

/// Render a reusable "apply this change?" confirmation window with a
/// bulleted plain-English diff and Cancel/Apply buttons.
/// Returns `(cancelled, confirmed)`.
pub(super) fn render_apply_confirm_dialog(
    ctx: &egui::Context,
    title: &str,
    lines: &[String],
    disabled: bool,
) -> (bool, bool) {
    let screen = ctx.screen_rect();
    let win_w = 520.0_f32.min(screen.width() * 0.85);
    let win_h = 420.0_f32.min(screen.height() * 0.85);
    let mut cancel = false;
    let mut confirm = false;
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .fixed_size([win_w, win_h])
        .default_pos([
            (screen.width() - win_w) / 2.0,
            (screen.height() - win_h) / 2.0,
        ])
        .show(ctx, |ui| {
            ui.label(
                RichText::new(title)
                    .size(16.0)
                    .strong()
                    .color(Color32::WHITE),
            );
            ui.add_space(10.0);
            ui.label(
                RichText::new(
                    "Here's what will change. 51Folds will re-infer and return updated probabilities.",
                )
                .size(12.0)
                .color(TEXT_SECONDARY),
            );
            ui.add_space(14.0);
            egui::Frame::default()
                .fill(SURFACE)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            if lines.is_empty() {
                                ui.label(
                                    RichText::new(
                                        "(This snapshot is identical to the current state — applying it will have no effect.)",
                                    )
                                    .size(12.0)
                                    .italics()
                                    .color(TEXT_MUTED),
                                );
                            } else {
                                for line in lines {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!("• {line}"))
                                                .size(12.0)
                                                .color(TEXT_PRIMARY),
                                        )
                                        .wrap(),
                                    );
                                    ui.add_space(3.0);
                                }
                            }
                        });
                });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().button_padding = Vec2::new(14.0, 8.0);
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("Cancel")
                                .size(13.0)
                                .color(TEXT_PRIMARY),
                        )
                        .fill(SURFACE)
                        .stroke(egui::Stroke::new(1.0, BORDER))
                        .corner_radius(6.0),
                    )
                    .clicked()
                {
                    cancel = true;
                }
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        let btn = ui.add_enabled(
                            !disabled && !lines.is_empty(),
                            egui::Button::new(
                                RichText::new("Apply")
                                    .size(13.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(ACCENT_BLUE_DIM)
                            .corner_radius(6.0),
                        );
                        if btn.clicked() {
                            confirm = true;
                        }
                    },
                );
            });
        });
    (cancel, confirm)
}

/// Render the "pick the primary instrument" modal shown when the user
/// clicks Analyze with more than one overlay instrument selected.
pub(super) fn render_analyze_primary_dialog(
    ctx: &egui::Context,
    selected: &[Instrument],
    choice: &mut Option<Instrument>,
) -> (bool, bool) {
    let screen = ctx.screen_rect();
    let win_w = 420.0_f32.min(screen.width() * 0.85);
    let win_h = (160.0 + 30.0 * selected.len() as f32).min(screen.height() * 0.85);
    let mut cancel = false;
    let mut confirm = false;
    egui::Window::new("pick_primary_instrument")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .fixed_size([win_w, win_h])
        .default_pos([
            (screen.width() - win_w) / 2.0,
            (screen.height() - win_h) / 2.0,
        ])
        .show(ctx, |ui| {
            ui.label(
                RichText::new("Pick the primary instrument for this analysis")
                    .size(15.0)
                    .strong()
                    .color(Color32::WHITE),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "The primary is the subject of the hypothesis. The other \
                     selected instruments will be mentioned as corroborative \
                     signal, not as subject.",
                )
                .size(11.0)
                .color(TEXT_SECONDARY),
            );
            ui.add_space(12.0);
            for inst in selected {
                let selected_now = *choice == Some(*inst);
                if ui.radio(selected_now, inst.as_str()).clicked() {
                    *choice = Some(*inst);
                }
            }
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().button_padding = Vec2::new(14.0, 6.0);
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("Cancel")
                                .size(12.0)
                                .color(TEXT_PRIMARY),
                        )
                        .fill(SURFACE)
                        .stroke(egui::Stroke::new(1.0, BORDER))
                        .corner_radius(6.0),
                    )
                    .clicked()
                {
                    cancel = true;
                }
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        let btn = ui.add_enabled(
                            choice.is_some(),
                            egui::Button::new(
                                RichText::new("Analyze")
                                    .size(12.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(ACCENT_BLUE_DIM)
                            .corner_radius(6.0),
                        );
                        if btn.clicked() {
                            confirm = true;
                        }
                    },
                );
            });
        });
    (cancel, confirm)
}

// ---------------------------------------------------------------------------
// Misc
// ---------------------------------------------------------------------------

/// Compute a plain-English diff summary between two model states.
pub(super) fn diff_model_states(
    from: &fiftyone_folds::ModelResponse,
    to: &fiftyone_folds::ModelResponse,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();

    for d in &to.current.drivers {
        let before = from
            .current
            .drivers
            .iter()
            .find(|f| f.code == d.code)
            .map(|f| f.state.as_str());
        match before {
            Some(b) if b != d.state => {
                lines.push(format!("{}: {b} → {}", d.code, d.state));
            }
            None => {
                lines.push(format!("{}: (new) → {}", d.code, d.state));
            }
            _ => {}
        }
    }

    for o in &to.current.outcomes {
        if let Some(from_o) = from
            .current
            .outcomes
            .iter()
            .find(|f| f.label == o.label)
        {
            let from_prob = from_o.probability.unwrap_or(0.0);
            let to_prob = o.probability.unwrap_or(0.0);
            let delta = to_prob - from_prob;
            if delta.abs() > 0.001 {
                let arrow = if delta > 0.0 { "↑" } else { "↓" };
                lines.push(format!(
                    "{}: {:.1}% → {:.1}% {arrow}",
                    o.label,
                    from_prob * 100.0,
                    to_prob * 100.0,
                ));
            }
        }
    }

    lines
}

/// Full-width clickable header row with a collapse chevron, title, and
/// right-aligned summary text. Returns `true` when clicked.
pub(super) fn collapsible_chart_header(
    ui: &mut egui::Ui,
    id_salt: &str,
    collapsed: bool,
    title: &str,
    right_text: &str,
) -> bool {
    use eframe::egui::FontId;
    let mut clicked = false;
    ui.push_id(id_salt, |ui| {
        let height = 30.0;
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());

        ui.painter().rect_filled(rect, 0.0, SURFACE);

        let cx = rect.min.x + 12.0;
        let cy = rect.center().y;
        let chevron_stroke = Stroke::new(1.5, TEXT_PRIMARY);
        if collapsed {
            let tip = Pos2::new(cx + 4.0, cy);
            let top = Pos2::new(cx - 2.0, cy - 4.0);
            let bot = Pos2::new(cx - 2.0, cy + 4.0);
            ui.painter().line_segment([top, tip], chevron_stroke);
            ui.painter().line_segment([bot, tip], chevron_stroke);
        } else {
            let tip = Pos2::new(cx, cy + 3.0);
            let left = Pos2::new(cx - 4.0, cy - 3.0);
            let right = Pos2::new(cx + 4.0, cy - 3.0);
            ui.painter().line_segment([left, tip], chevron_stroke);
            ui.painter().line_segment([right, tip], chevron_stroke);
        }

        ui.painter().text(
            Pos2::new(rect.min.x + 24.0, rect.center().y),
            Align2::LEFT_CENTER,
            title,
            FontId::proportional(14.0),
            TEXT_PRIMARY,
        );

        if !right_text.is_empty() {
            ui.painter().text(
                Pos2::new(rect.max.x - 6.0, rect.center().y),
                Align2::RIGHT_CENTER,
                right_text,
                FontId::proportional(11.0),
                TEXT_PRIMARY,
            );
        }

        clicked = resp.clicked();
    });
    clicked
}

/// Terminal color palette matching The Hedgehog's dark UI.
pub(super) fn hedgehog_terminal_theme() -> egui_term::TerminalTheme {
    let palette = egui_term::ColorPalette {
        background: String::from("#111827"),
        foreground: String::from("#e2e8f0"),
        black: String::from("#0a0e1a"),
        red: String::from("#e53e3e"),
        green: String::from("#38a169"),
        yellow: String::from("#d69e2e"),
        blue: String::from("#60a5fa"),
        magenta: String::from("#aa759f"),
        cyan: String::from("#75b5aa"),
        white: String::from("#e2e8f0"),
        bright_black: String::from("#4a5568"),
        bright_red: String::from("#fc8181"),
        bright_green: String::from("#68d391"),
        bright_yellow: String::from("#feca88"),
        bright_blue: String::from("#82b8c8"),
        bright_magenta: String::from("#c28cb8"),
        bright_cyan: String::from("#93d3c3"),
        bright_white: String::from("#f8f8f8"),
        bright_foreground: None,
        dim_foreground: String::from("#94a3b8"),
        dim_black: String::from("#0a0e1a"),
        dim_red: String::from("#712b2b"),
        dim_green: String::from("#5f6f3a"),
        dim_yellow: String::from("#a17e4d"),
        dim_blue: String::from("#456877"),
        dim_magenta: String::from("#704d68"),
        dim_cyan: String::from("#4d7770"),
        dim_white: String::from("#8e8e8e"),
    };
    egui_term::TerminalTheme::new(Box::new(palette))
}
