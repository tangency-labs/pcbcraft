//! Shared Home surfaces with PCBcraft's compact layout and vector recent-project previews.

use egui::{Align, FontId, Layout, Rect, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use solidcraft_ui_kit::{FileView, Icon};

use crate::{Tokens, fonts, menu::MenuIcon};

/// Whether a page of this width stacks the navigation above the content instead of beside it.
pub fn is_narrow(width: f32) -> bool {
    let t = Tokens::get();
    width < t.file_nav_width + t.file_tile_min_width + 2.0 * t.file_content_inset_x
}

/// Fill the space left by the bars with the Home page and lay out `navigation` and `content`.
/// Wide pages pin the navigation in a rail at the left. Narrow pages scroll both in one column,
/// navigation first, so every row stays reachable at phone widths.
pub fn page(ui: &mut Ui, navigation: impl FnOnce(&mut Ui), content: impl FnOnce(&mut Ui)) {
    let t = Tokens::get();
    let mut shared = solidcraft_theme::Tokens::get();
    shared.file_tile_height = t.file_tile_height;
    shared.file_tile_icon = t.file_tile_icon;
    let _home_tokens = solidcraft_theme::scoped_panel_tokens(shared);
    FileView::page(ui, |ui| {
        let full = ui.available_rect_before_wrap();
        if is_narrow(full.width()) {
            egui::ScrollArea::vertical().id_salt("home_page").auto_shrink([false, false]).show(ui, |ui| {
                crate::workspace::describe_scroll_viewport(ui);
                ui.spacing_mut().item_spacing.y = 0.0;
                FileView::navigation(ui, |ui| {
                    ui.set_width(ui.available_width());
                    navigation(ui);
                });
                FileView::content(ui, true, content);
            });
            return;
        }
        let rail = Rect::from_min_size(full.min, vec2(t.file_nav_width, full.height()));
        FileView::rail(ui, rail, |ui| {
            egui::ScrollArea::vertical().id_salt("home_navigation").auto_shrink([false, false]).show(ui, |ui| {
                crate::workspace::describe_scroll_viewport(ui);
                navigation(ui);
            });
        });
        let body = Rect::from_min_max(pos2(rail.right(), full.top()), full.max);
        ui.scope_builder(UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min)), |ui| {
            egui::ScrollArea::vertical().id_salt("home_content").auto_shrink([false, false]).show(ui, |ui| {
                crate::workspace::describe_scroll_viewport(ui);
                FileView::content(ui, false, content);
            });
        });
    });
}

/// Allocate and name the recent card's open target. Disabled cards keep their hover text.
fn recent_target(ui: &mut Ui, id: &str, width: f32, height: f32, label: &str, enabled: bool) -> (Rect, Response) {
    let (rect, _) = ui.allocate_exact_size(vec2(width.min(ui.available_width()).max(0.0), height), Sense::hover());
    let enabled = enabled && ui.is_enabled();
    let response = ui.interact(rect, ui.make_persistent_id(id), if enabled { Sense::click() } else { Sense::hover() });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    if enabled && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    (rect, response)
}

/// Paint one line of text inside `rect`, vertically centered and elided when it does not fit.
fn text(ui: &Ui, rect: Rect, text: &str, font: FontId, color: egui::Color32) {
    if text.is_empty() || !rect.is_positive() {
        return;
    }
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap.max_width = rect.width();
    job.wrap.max_rows = 1;
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
    let galley = painter.layout_job(job);
    painter.galley(pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, color);
}

/// Return to the open project or the empty workspace through the shared back row.
pub fn back_to(ui: &mut Ui, document: Option<&str>) -> Response {
    let (template, name) = document.map_or(("Back to the workspace", ""), |name| ("Back to {name}", name));
    FileView::back_to(ui, "home:back", template, name, "Esc")
}

/// A navigation row with its shortcut at the right. A selected row keeps the hover fill and a
/// heavier label; a disabled row is dimmed and never reports a click.
pub fn nav_row(ui: &mut Ui, id: &str, label: &str, shortcut: &str, selected: bool, enabled: bool) -> Response {
    let response = FileView::nav_row(ui, id, label, shortcut, selected, enabled);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, response.enabled(), selected, label));
    response
}

/// A faint rule between groups of navigation rows.
pub fn separator(ui: &mut Ui) {
    FileView::separator(ui);
}

/// The page heading and the line that explains it, followed by the gap before the first section.
pub fn heading(ui: &mut Ui, title: &str, subtitle: &str) {
    let t = Tokens::get();
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.scope(|ui| FileView::title(ui, title)).response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Heading, true, title));
    ui.add_space(t.space_6);
    ui.scope(|ui| FileView::subtitle(ui, subtitle)).response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, subtitle));
    ui.add_space(t.file_section_gap);
}

/// A titled group of tiles or rows.
pub fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get();
    ui.scope(|ui| FileView::section_title(ui, title)).response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Heading, true, title));
    ui.add_space(t.space_12);
    add(ui);
    ui.add_space(t.file_section_gap);
}

/// Columns of equal width, as many as fit at `file_tile_min_width`, and the width of each.
fn grid_geometry(available: f32, count: usize) -> (usize, f32) {
    let t = Tokens::get();
    let gap = t.file_grid_gap;
    let available = available.max(0.0);
    let fit = ((available + gap) / (t.file_tile_min_width + gap)).floor().max(1.0) as usize;
    let columns = fit.min(count.max(1));
    let width = ((available - gap * columns.saturating_sub(1) as f32) / columns as f32).max(0.0);
    (columns, width)
}

/// Lay out `count` cells in rows. `cell` gets each index and the cell width.
pub fn grid(ui: &mut Ui, id: &str, count: usize, cell: impl FnMut(&mut Ui, usize, f32)) {
    bounded_grid(ui, id, count, f32::INFINITY, cell);
}

/// Allocate rows using the available column count, capping each cell independently so a
/// partially filled row and a single-column page both retain the requested width bound.
fn bounded_grid(ui: &mut Ui, id: &str, count: usize, max_width: f32, mut cell: impl FnMut(&mut Ui, usize, f32)) {
    let t = Tokens::get();
    let (columns, width) = grid_geometry(ui.available_width(), count);
    let width = width.min(max_width);
    ui.push_id(id, |ui| {
        ui.spacing_mut().item_spacing = vec2(t.file_grid_gap, t.file_grid_gap);
        for row in (0..count).step_by(columns) {
            ui.horizontal_top(|ui| {
                for index in row..(row + columns).min(count) {
                    cell(ui, index, width);
                }
            });
        }
    });
}

/// Keep action cards compact on wide pages, wrapping into rows when space is limited.
/// Recent-project grids retain their independent preview sizing.
pub fn action_grid(ui: &mut Ui, id: &str, count: usize, cell: impl FnMut(&mut Ui, usize, f32)) {
    bounded_grid(ui, id, count, Tokens::get().file_tile_max_width, cell);
}

/// What an action tile shows and whether it can be clicked.
#[derive(Clone, Copy, Debug)]
pub struct Tile<'a> {
    pub icon: MenuIcon,
    pub label: &'a str,
    pub caption: &'a str,
    /// Tint the tile as the page's main action.
    pub primary: bool,
    pub enabled: bool,
}

/// A card with an icon, a title and a caption of up to two lines. The primary tile is tinted,
/// and a disabled tile is dimmed and never reports a click.
pub fn action_tile(ui: &mut Ui, id: &str, width: f32, tile: Tile<'_>) -> Response {
    let Tile { icon, label, caption, primary, enabled } = tile;
    let t = Tokens::get();
    let paint = |painter: &egui::Painter, rect: Rect, opacity| {
        let mut painter = painter.clone();
        painter.multiply_opacity(opacity);
        icon.paint_sized(&painter, rect.center(), rect.width().min(rect.height()) * 0.75, t, true);
    };
    ui.add_enabled_ui(enabled, |ui| FileView::action_tile(ui, id, width, Icon::Paint(&paint), label, caption, primary)).inner
}

/// Preserve both ends of a path inside a card, including Unicode filenames. Adapted from
/// SolidCraft's `ui_kit/text.rs::ellipsize_middle`; invalid widths yield an empty label.
fn elide_path(ui: &Ui, path: &str, font: FontId, width: f32) -> String {
    if !width.is_finite() || width <= 0.0 {
        return String::new();
    }
    let measure = |text: &str| ui.painter().layout_no_wrap(text.to_owned(), font.clone(), Tokens::get().file_dim).size().x;
    if measure(path) <= width {
        return path.to_owned();
    }
    let chars: Vec<_> = path.chars().collect();
    let mut best = "…".to_owned();
    let (mut low, mut high) = (1, chars.len() / 2);
    while low <= high {
        let keep = (low + high) / 2;
        let candidate = format!("{}…{}", chars.iter().take(keep).collect::<String>(), chars.iter().skip(chars.len() - keep).collect::<String>());
        if measure(&candidate) <= width {
            best = candidate;
            low = keep + 1;
        } else {
            high = keep - 1;
        }
    }
    best
}

/// Preview availability is presentation state, independent of any application's document model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewState {
    Ready,
    Loading,
    Empty,
    Unavailable,
}

/// The separate actions in a recent project card; removing never activates opening.
#[derive(Clone, Copy, Debug, Default)]
pub struct RecentAction {
    pub open: bool,
    pub remove: bool,
}

/// Application-owned labels and availability for a family-style rendered project card.
pub struct RecentCard<'a> {
    pub title: &'a str,
    pub path: &'a str,
    pub enabled: bool,
    pub preview: PreviewState,
}

/// A bounded card with an app-rendered picture above its title/path and a separate removal
/// button. The full path stays in accessible labels and hover text, including at compact widths.
/// `paint` runs only for visible images, with a clipped painter and the computed image rectangle.
/// The requested width is capped by `file_recent_card_max_width` and the available space.
/// Preview height follows the inset width's aspect ratio within the theme's height bounds.
pub fn recent_card(ui: &mut Ui, id: &str, width: f32, card: RecentCard<'_>, paint: impl FnOnce(&egui::Painter, Rect)) -> RecentAction {
    let t = Tokens::get();
    let width = width.min(t.file_recent_card_max_width).min(ui.available_width()).max(0.0);
    let image_width = (width - t.file_card_inset * 2.0).max(0.0);
    let image_height = (image_width / t.file_preview_aspect_ratio).clamp(t.file_preview_min_height, t.file_preview_max_height);
    let height = image_height + t.file_card_inset * 2.0 + t.space_8 * 2.0 + t.font_button + t.font_small;
    let label = format!("Open {} ({})", card.title, card.path);
    let (rect, response) = recent_target(ui, id, width, height, &label, card.enabled);
    let hot = card.enabled && (response.hovered() || response.has_focus());
    ui.painter().rect(
        rect,
        t.radius_lg,
        if hot { t.hover } else { t.file_surface },
        Stroke::new(1.0, if response.has_focus() { t.accent } else { t.file_border }),
        StrokeKind::Inside,
    );
    let inner = rect.shrink(t.file_card_inset);
    let image = Rect::from_min_size(inner.min, vec2(inner.width(), image_height));
    ui.painter().rect(image, t.radius_md, t.canvas, Stroke::new(1.0, t.file_border), StrokeKind::Inside);
    let image_label = format!("Schematic preview: {} ({})", card.title, card.path);
    let image_response = ui.interact(image, ui.make_persistent_id(("recent:preview", id)), Sense::hover());
    image_response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &image_label));
    if card.preview == PreviewState::Ready && ui.clip_rect().intersects(image) {
        paint(&ui.painter().with_clip_rect(ui.clip_rect().intersect(image.shrink(t.space_2))), image);
    } else if card.preview != PreviewState::Ready {
        let label = match card.preview {
            PreviewState::Loading => "Loading preview…",
            PreviewState::Empty => "Empty schematic",
            PreviewState::Unavailable => "Preview unavailable",
            PreviewState::Ready => "",
        };
        MenuIcon::Open.paint_sized(ui.painter(), image.center() - vec2(0.0, t.space_8), t.space_24, t, false);
        ui.painter().text(image.center() + vec2(0.0, t.space_16), egui::Align2::CENTER_CENTER, label, FontId::proportional(t.font_small), t.file_dim);
    }
    let top = image.bottom() + t.space_8;
    let remove_size = t.space_24 + t.space_4;
    let remove_rect = Rect::from_center_size(pos2(inner.right() - remove_size / 2.0, (top + inner.bottom()) / 2.0), vec2(remove_size, remove_size));
    let right = (remove_rect.left() - t.space_4).max(inner.left());
    text(
        ui,
        Rect::from_min_max(pos2(inner.left(), top), pos2(right, top + t.font_button + t.space_2)),
        card.title,
        fonts::semibold(t.font_button),
        t.text,
    );
    let path_rect = Rect::from_min_max(pos2(inner.left(), top + t.font_button + t.space_4), pos2(right, inner.bottom()));
    let path_font = FontId::monospace(t.font_small);
    let path = elide_path(ui, card.path, path_font.clone(), path_rect.width());
    text(ui, path_rect, &path, path_font, t.file_dim);
    let remove = ui.interact(remove_rect, ui.make_persistent_id(("recent:remove", id)), Sense::click());
    remove.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), format!("Remove {} ({}) from recent projects", card.title, card.path))
    });
    if remove.hovered() || remove.has_focus() {
        ui.painter().rect_filled(remove_rect, t.radius_md, t.hover);
    }
    MenuIcon::Close.paint_sized(ui.painter(), remove_rect.center(), t.space_12, t, true);
    if remove.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let picked = RecentAction { open: response.clicked() && !remove.clicked(), remove: remove.clicked() };
    response.on_hover_text(card.path);
    remove.on_hover_text("Remove from recent projects");
    picked
}

/// Show the empty history without adding an inactive project row.
pub fn empty_recents(ui: &mut Ui) {
    let t = Tokens::get();
    ui.label(egui::RichText::new("No recent projects yet").size(t.font_heading).color(t.file_dim));
}

/// Forget all history, without deleting project files.
pub fn clear_recents(ui: &mut Ui) -> Response {
    ui.add_space(Tokens::get().space_8);
    crate::controls::button(ui, "Clear recent projects")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_fits_columns_to_the_width_and_never_more_than_the_cells() {
        let t = Tokens::get();
        assert_eq!(grid_geometry(100.0, 2), (1, 100.0));
        let (columns, width) = grid_geometry(2.0 * t.file_tile_min_width + t.file_grid_gap, 2);
        assert_eq!(columns, 2);
        assert_eq!(width, t.file_tile_min_width);
        assert_eq!(grid_geometry(10_000.0, 2).0, 2);
        assert_eq!(grid_geometry(-5.0, 0), (1, 0.0));
    }

    #[test]
    fn the_rail_moves_above_the_content_when_both_do_not_fit() {
        let t = Tokens::get();
        let threshold = t.file_nav_width + t.file_tile_min_width + 2.0 * t.file_content_inset_x;
        assert!(is_narrow(threshold - 1.0));
        assert!(!is_narrow(threshold));
    }
}
