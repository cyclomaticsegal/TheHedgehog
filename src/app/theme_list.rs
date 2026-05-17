//! Per-theme model list — pure UI. Renders one page of the rows that
//! the caller has already filtered + sorted, plus a sort dropdown and
//! prev/next page controls. Row painting is delegated to the shared
//! helper in `model_browser.rs` so styling stays consistent.

use super::model_browser::render_row;
use super::theme::*;
use super::views::ThemeListSort;
use crate::models::{FoldsModelRecord, FoldsTheme};
use eframe::egui::{self, Color32, RichText};
use std::collections::HashSet;

const PAGE_SIZE: usize = 10;

#[must_use]
pub(super) enum ThemeListAction {
    None,
    Back,
    OpenModel(String),
    ChangeSort(ThemeListSort),
    ChangePage(usize),
}

pub(super) fn render_theme_list(
    ui: &mut egui::Ui,
    theme: &FoldsTheme,
    rows: &[FoldsModelRecord],
    foreground_model_id: Option<&str>,
    models_with_vault: &HashSet<String>,
    sort: ThemeListSort,
    page: usize,
) -> ThemeListAction {
    let mut action = ThemeListAction::None;

    // Header row: back pill + theme name + count.
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if ui
            .selectable_label(false, "\u{2190} Themes")
            .on_hover_text("Back to the cards landing.")
            .clicked()
        {
            action = ThemeListAction::Back;
        }
        ui.add_space(10.0);
        ui.heading(
            RichText::new(&theme.name)
                .size(20.0)
                .strong()
                .color(Color32::WHITE),
        );
        ui.label(
            RichText::new(format!(
                " \u{00B7} {} model{}",
                rows.len(),
                if rows.len() == 1 { "" } else { "s" }
            ))
            .size(13.0)
            .color(TEXT_SECONDARY),
        );
    });
    ui.add_space(8.0);

    // Sort + paging row.
    let total_pages = if rows.is_empty() {
        1
    } else {
        rows.len().div_ceil(PAGE_SIZE)
    };
    let current_page = page.min(total_pages.saturating_sub(1));
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Sort:")
                .size(12.0)
                .color(TEXT_SECONDARY),
        );
        let mut new_sort = sort;
        egui::ComboBox::from_id_salt("theme_list_sort")
            .selected_text(sort.label())
            .show_ui(ui, |ui| {
                for option in ThemeListSort::ALL {
                    ui.selectable_value(&mut new_sort, option, option.label());
                }
            });
        if new_sort != sort {
            action = ThemeListAction::ChangeSort(new_sort);
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let next_enabled = current_page + 1 < total_pages;
            let prev_enabled = current_page > 0;
            if ui.add_enabled(next_enabled, egui::Button::new("Next \u{203A}")).clicked() {
                action = ThemeListAction::ChangePage(current_page + 1);
            }
            ui.label(
                RichText::new(format!("Page {} of {}", current_page + 1, total_pages))
                    .size(12.0)
                    .color(TEXT_SECONDARY),
            );
            if ui.add_enabled(prev_enabled, egui::Button::new("\u{2039} Prev")).clicked()
                && current_page > 0
            {
                action = ThemeListAction::ChangePage(current_page - 1);
            }
        });
    });
    ui.add_space(6.0);

    if rows.is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("No models in this theme yet.")
                    .size(14.0)
                    .color(TEXT_MUTED),
            );
        });
        return action;
    }

    let start = current_page * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(rows.len());
    let page_slice = &rows[start..end];

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for record in page_slice {
                let is_foreground = foreground_model_id == Some(record.model_id.as_str());
                let has_vault = models_with_vault.contains(&record.model_id);
                if render_row(ui, record, is_foreground, has_vault, 56.0) {
                    action = ThemeListAction::OpenModel(record.model_id.clone());
                }
            }
        });

    action
}
