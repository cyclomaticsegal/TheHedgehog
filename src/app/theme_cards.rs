//! Cards-landing surface for the 51Folds tab — one card per theme,
//! showing the theme's name, model count, description, and up to two
//! example questions. Pure UI: takes a slice of `ThemeCardData`,
//! returns a `ThemeCardsAction` that the caller translates into a
//! navigation change.

use super::theme::*;
use super::views::ThemeCardData;
use eframe::egui::{self, Color32, RichText, Sense, Stroke, StrokeKind, Vec2};

/// What the user did on the cards landing this frame.
#[must_use]
pub(super) enum ThemeCardsAction {
    None,
    OpenTheme(i64),
    OpenManage,
}

pub(super) fn render_theme_cards(
    ui: &mut egui::Ui,
    cards: &[ThemeCardData],
) -> ThemeCardsAction {
    ui.add_space(10.0);
    ui.heading(
        RichText::new("51Folds models")
            .size(22.0)
            .strong()
            .color(Color32::WHITE),
    );
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(
            RichText::new(
                "Models grouped by theme. Click a card to see the models in that theme; \
                 in-flight builds are unaffected by browsing.",
            )
            .size(13.0)
            .color(TEXT_SECONDARY),
        )
        .wrap(),
    );

    // Manage themes button on its own row (right-aligned) so it never
    // collides with the heading on narrow windows.
    let mut action = ThemeCardsAction::None;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let manage = ui.add(
                egui::Button::new(
                    RichText::new("Manage themes")
                        .size(12.0)
                        .color(ACCENT_BLUE),
                )
                .fill(Color32::TRANSPARENT)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(4.0),
            );
            if manage.clicked() {
                action = ThemeCardsAction::OpenManage;
            }
        });
    });
    ui.add_space(10.0);

    if cards.is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("No themes yet")
                    .size(18.0)
                    .color(TEXT_MUTED),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "The first time the app launches with a populated database, \
                     a five-theme taxonomy is seeded automatically.",
                )
                .size(13.0)
                .color(TEXT_SECONDARY),
            );
        });
        return action;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // 2-column flow, rendered manually so we control gaps and
            // wrap behavior. Card width is computed from available
            // width minus a fixed gutter so two cards fit comfortably.
            let avail = ui.available_width();
            let gutter = 14.0_f32;
            let card_width = ((avail - gutter) / 2.0).max(280.0);
            let card_height = 156.0_f32;

            let mut i = 0;
            while i < cards.len() {
                ui.horizontal(|ui| {
                    // Left card
                    if render_card(ui, &cards[i], card_width, card_height) {
                        action = ThemeCardsAction::OpenTheme(cards[i].theme.id);
                    }
                    ui.add_space(gutter);
                    // Right card (if any)
                    if i + 1 < cards.len()
                        && render_card(ui, &cards[i + 1], card_width, card_height)
                    {
                        action = ThemeCardsAction::OpenTheme(cards[i + 1].theme.id);
                    }
                });
                ui.add_space(gutter);
                i += 2;
            }
        });

    action
}

/// Render one card and return true if clicked.
fn render_card(
    ui: &mut egui::Ui,
    card: &ThemeCardData,
    width: f32,
    height: f32,
) -> bool {
    let dimmed = card.count == 0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let hovered = response.hovered();
    let bg = if hovered { SURFACE_HOVER } else { SURFACE };
    ui.painter().rect(
        rect,
        8.0,
        bg,
        Stroke::new(1.0, BORDER),
        StrokeKind::Inside,
    );

    let padding = 16.0;
    let inner = rect.shrink2(Vec2::new(padding, 14.0));

    let title_color = if dimmed { TEXT_MUTED } else { Color32::WHITE };
    let count_text = if dimmed {
        "no models".to_owned()
    } else {
        format!("{} model{}", card.count, if card.count == 1 { "" } else { "s" })
    };

    let title_galley = ui.fonts(|f| {
        f.layout(
            card.theme.name.clone(),
            egui::FontId::new(15.0, egui::FontFamily::Proportional),
            title_color,
            inner.width() - 90.0,
        )
    });
    ui.painter().galley(inner.min, title_galley.clone(), title_color);

    let count_pos = egui::Pos2::new(inner.right(), inner.top());
    ui.painter().text(
        count_pos,
        egui::Align2::RIGHT_TOP,
        count_text,
        egui::FontId::new(12.0, egui::FontFamily::Proportional),
        TEXT_SECONDARY,
    );

    // Description
    let desc_y = inner.top() + title_galley.size().y + 8.0;
    let desc_galley = ui.fonts(|f| {
        f.layout(
            truncate(&card.theme.description, 200),
            egui::FontId::new(12.0, egui::FontFamily::Proportional),
            TEXT_SECONDARY,
            inner.width(),
        )
    });
    ui.painter().galley(
        egui::Pos2::new(inner.left(), desc_y),
        desc_galley.clone(),
        TEXT_SECONDARY,
    );

    // Sample questions
    let mut y = desc_y + desc_galley.size().y + 10.0;
    for q in card.sample_questions.iter().take(2) {
        let q_galley = ui.fonts(|f| {
            f.layout(
                format!("\u{201C}{}\u{2026}\u{201D}", truncate(q, 80)),
                egui::FontId::new(12.0, egui::FontFamily::Proportional),
                TEXT_MUTED,
                inner.width(),
            )
        });
        ui.painter().galley(
            egui::Pos2::new(inner.left(), y),
            q_galley.clone(),
            TEXT_MUTED,
        );
        y += q_galley.size().y + 2.0;
        if y > inner.bottom() {
            break;
        }
    }

    response.clicked()
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out: String = s.chars().take(max_chars).collect();
    if s.chars().count() > max_chars {
        out.push('\u{2026}');
    }
    out
}
