//! Shared model-row painter. Originally the renderer for a flat-list
//! 51Folds browser (ADR 0024). The themed cards landing replaced the
//! flat list, but the row painter is reused by `theme_list.rs` so the
//! styling stays consistent across surfaces.

use super::theme::*;
use crate::models::{
    FoldsModelRecord, FOLDS_STATUS_FAIL, FOLDS_STATUS_PENDING, FOLDS_STATUS_SUCCESS,
    FOLDS_STATUS_UNDISCLOSED_FAILURE,
};
use eframe::egui::{self, Color32, Sense, Stroke, StrokeKind, Vec2};

pub(super) fn render_row(
    ui: &mut egui::Ui,
    record: &FoldsModelRecord,
    is_foreground: bool,
    has_vault: bool,
    row_height: f32,
) -> bool {
    let (status_label, status_color) = status_chip(record.status.as_str());
    let question_text = if record.question.trim().is_empty() {
        "(untitled hypothesis)".to_owned()
    } else {
        record.question.clone()
    };
    let model_id_short = truncate_model_id(&record.model_id);
    let when = record.created_at.format("%Y-%m-%d %H:%M").to_string();

    let avail = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(avail, row_height - 6.0),
        Sense::click(),
    );
    let hovered = response.hovered();
    let bg = if hovered { SURFACE_HOVER } else { SURFACE };
    let stroke = if is_foreground {
        Stroke::new(1.5, ACCENT_BLUE)
    } else {
        Stroke::new(1.0, BORDER)
    };
    ui.painter().rect(rect, 6.0, bg, stroke, StrokeKind::Inside);

    let padding = 12.0;
    let inner = rect.shrink2(Vec2::new(padding, 8.0));

    // Status chip + foreground pin row.
    let chip_w = 84.0;
    let chip_rect = egui::Rect::from_min_size(inner.min, Vec2::new(chip_w, 18.0));
    ui.painter().rect_filled(chip_rect, 4.0, status_color);
    ui.painter().text(
        chip_rect.center(),
        egui::Align2::CENTER_CENTER,
        status_label,
        egui::FontId::new(11.0, egui::FontFamily::Proportional),
        Color32::WHITE,
    );

    let mut next_pin_x = chip_rect.right() + 8.0;
    if is_foreground {
        let pin_pos = egui::Pos2::new(next_pin_x, chip_rect.center().y);
        let galley = ui.painter().layout_no_wrap(
            "\u{1F4CC} Foreground".to_owned(),
            egui::FontId::new(11.0, egui::FontFamily::Proportional),
            ACCENT_BLUE,
        );
        let advance = galley.size().x;
        ui.painter()
            .galley(pin_pos - Vec2::new(0.0, galley.size().y / 2.0), galley, ACCENT_BLUE);
        next_pin_x += advance + 10.0;
    }
    if has_vault {
        let pin_pos = egui::Pos2::new(next_pin_x, chip_rect.center().y);
        ui.painter().text(
            pin_pos,
            egui::Align2::LEFT_CENTER,
            "\u{1F4D2} Vault",
            egui::FontId::new(11.0, egui::FontFamily::Proportional),
            ALERT_NORMAL_FG,
        );
    }

    // Question on the row below.
    let question_pos = egui::Pos2::new(inner.left(), inner.top() + 24.0);
    let question_font = egui::FontId::new(14.0, egui::FontFamily::Proportional);
    let question_galley = ui.fonts(|f| {
        f.layout(
            truncate_for_row(&question_text, 110),
            question_font,
            Color32::WHITE,
            inner.width(),
        )
    });
    ui.painter()
        .galley(question_pos, question_galley, Color32::WHITE);

    // Metadata row at the bottom right.
    let meta = format!("{} \u{00B7} {}", model_id_short, when);
    ui.painter().text(
        egui::Pos2::new(inner.right(), inner.bottom() - 4.0),
        egui::Align2::RIGHT_BOTTOM,
        meta,
        egui::FontId::new(11.0, egui::FontFamily::Proportional),
        TEXT_MUTED,
    );

    ui.add_space(6.0);
    response.clicked()
}

fn status_chip(status: &str) -> (&'static str, Color32) {
    match status {
        FOLDS_STATUS_SUCCESS => ("BUILT", ALERT_NORMAL_FG),
        FOLDS_STATUS_PENDING => ("BUILDING", ACCENT_BLUE),
        FOLDS_STATUS_FAIL => ("FAILED", ALERT_EXTREME_FG),
        FOLDS_STATUS_UNDISCLOSED_FAILURE => ("LOST", ALERT_EXTREME_FG),
        _ => ("UNKNOWN", TEXT_MUTED),
    }
}

fn truncate_model_id(model_id: &str) -> String {
    if model_id.len() > 10 {
        format!("{}\u{2026}", &model_id[..10])
    } else {
        model_id.to_owned()
    }
}

fn truncate_for_row(s: &str, max_chars: usize) -> String {
    let mut out: String = s.chars().take(max_chars).collect();
    if s.chars().count() > max_chars {
        out.push('\u{2026}');
    }
    out
}
