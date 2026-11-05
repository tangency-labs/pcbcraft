//! The Home page surfaces: a navigation rail beside a scrolling start page of action tiles.
//! Adapted from SolidCraft's `ui_kit/file_view.rs` (see `ATTRIBUTION.md`). The caller owns
//! what each row and tile does; these functions only draw and report clicks.

use egui::{Align, FontId, Frame, Layout, Margin, Rect, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

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
    egui::CentralPanel::default().frame(Frame::new().fill(t.file_background)).show(ui, |ui| {
        let full = ui.available_rect_before_wrap();
        if is_narrow(full.width()) {
            egui::ScrollArea::vertical().id_salt("home_page").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                rail_frame(t).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = t.space_2;
                    navigation(ui);
                });
                content_frame(t, t.space_16).show(ui, content);
            });
            return;
        }
        let rail = Rect::from_min_size(full.min, vec2(t.file_nav_width, full.height()));
        ui.painter().rect_filled(rail, 0, t.file_rail);
        ui.scope_builder(UiBuilder::new().max_rect(rail).layout(Layout::top_down(Align::Min)), |ui| {
            egui::ScrollArea::vertical().id_salt("home_navigation").auto_shrink([false, false]).show(ui, |ui| {
                rail_frame(t).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = t.space_2;
                    navigation(ui);
                });
            });
        });
        let body = Rect::from_min_max(pos2(rail.right(), full.top()), full.max);
        ui.scope_builder(UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min)), |ui| {
            egui::ScrollArea::vertical().id_salt("home_content").auto_shrink([false, false]).show(ui, |ui| {
                content_frame(t, t.file_content_inset_x).show(ui, content);
            });
        });
    });
}

/// The rail's fill and the inset around its rows.
fn rail_frame(t: Tokens) -> Frame {
    Frame::new().fill(t.file_rail).inner_margin(Margin::symmetric((t.space_8 + t.space_2) as i8, (t.space_12 + t.space_2) as i8))
}

/// The inset around the page content, `inset` points at each side.
fn content_frame(t: Tokens, inset: f32) -> Frame {
    Frame::new().inner_margin(Margin::symmetric(inset as i8, t.file_content_inset_y as i8))
}

/// Allocate a clickable row or tile and label it for assistive technology, with its selected
/// state when it has one. Disabled controls only sense hover, so they never report a click.
fn allocate(ui: &mut Ui, id: &str, width: f32, height: f32, label: &str, enabled: bool, selected: Option<bool>) -> (Rect, Response) {
    let (rect, _) = ui.allocate_exact_size(vec2(width.min(ui.available_width()).max(0.0), height), Sense::hover());
    let response = ui.interact(rect, ui.make_persistent_id(id), if enabled { Sense::click() } else { Sense::hover() });
    response.widget_info(|| match selected {
        Some(selected) => egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label),
        None => egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label),
    });
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

/// The first rail row: "Back to" the open project, or to the empty workspace, with its key.
pub fn back_to(ui: &mut Ui, id: &str, label: &str, shortcut: &str) -> Response {
    let t = Tokens::get();
    let (rect, response) = allocate(ui, id, ui.available_width(), t.file_back_height, label, true, None);
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, t.radius_md, t.file_rail_hover);
    }
    let inner = rect.shrink2(vec2(t.space_12, 0.0));
    let stroke = Stroke::new(1.5, t.file_rail_ink);
    let tip = pos2(inner.left() + t.space_4, inner.center().y);
    ui.painter().line(vec![tip + vec2(t.space_6, -t.space_6), tip, tip + vec2(t.space_6, t.space_6)], stroke);
    let key = ui.painter().layout_no_wrap(shortcut.to_owned(), FontId::monospace(t.font_small), t.file_rail_ink);
    let key_rect =
        Rect::from_center_size(pos2(inner.right() - (key.size().x + t.space_8) / 2.0, inner.center().y), key.size() + vec2(t.space_8, t.space_4));
    ui.painter().rect_stroke(key_rect, t.radius_sm, Stroke::new(1.0, t.file_rail_ink.gamma_multiply(0.5)), StrokeKind::Inside);
    ui.painter().galley(key_rect.center() - key.size() / 2.0, key, t.file_rail_ink);
    let label_rect = Rect::from_min_max(pos2(inner.left() + t.space_16 + t.space_4, inner.top()), pos2(key_rect.left() - t.space_8, inner.bottom()));
    text(ui, label_rect, label, FontId::proportional(t.font_button), t.file_rail_ink);
    ui.add_space(t.space_8);
    response.on_hover_text(label)
}

/// A navigation row with its shortcut at the right. A selected row keeps the hover fill and a
/// heavier label; a disabled row is dimmed and never reports a click.
pub fn nav_row(ui: &mut Ui, id: &str, label: &str, shortcut: &str, selected: bool, enabled: bool) -> Response {
    let t = Tokens::get();
    let (rect, response) = allocate(ui, id, ui.available_width(), t.file_nav_height, label, enabled, Some(selected));
    if selected || (enabled && (response.hovered() || response.has_focus())) {
        ui.painter().rect_filled(rect, t.radius_md, t.file_rail_hover);
    }
    let ink = if enabled { t.file_rail_ink } else { t.file_rail_ink.gamma_multiply(0.5) };
    let inner = rect.shrink2(vec2(t.space_12, 0.0));
    let key_width = ui.painter().layout_no_wrap(shortcut.to_owned(), FontId::monospace(t.font_small), ink).size().x.min(inner.width());
    let font = if selected { fonts::semibold(t.font_heading) } else { FontId::proportional(t.font_heading) };
    let label_right = (inner.right() - key_width - t.space_8).max(inner.left());
    text(ui, Rect::from_min_max(inner.min, pos2(label_right, inner.bottom())), label, font, ink);
    text(ui, Rect::from_min_max(pos2(inner.right() - key_width, inner.top()), inner.max), shortcut, FontId::monospace(t.font_small), ink);
    response
}

/// A faint rule between groups of navigation rows.
pub fn separator(ui: &mut Ui) {
    let t = Tokens::get();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), t.space_16 + t.space_4), Sense::hover());
    let x = (rect.left() + t.space_12)..=(rect.right() - t.space_12).max(rect.left() + t.space_12);
    ui.painter().hline(x, rect.center().y, Stroke::new(1.0, t.file_rail_ink.gamma_multiply(0.25)));
}

/// The page heading and the line that explains it, followed by the gap before the first section.
pub fn heading(ui: &mut Ui, title: &str, subtitle: &str) {
    let t = Tokens::get();
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.label(egui::RichText::new(title).font(fonts::semibold(t.font_start_title)).color(t.text));
    ui.add_space(t.space_6);
    ui.label(egui::RichText::new(subtitle).size(t.font_heading).color(t.file_dim));
    ui.add_space(t.file_section_gap);
}

/// A titled group of tiles or rows.
pub fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get();
    ui.label(egui::RichText::new(title).font(fonts::semibold(t.font_intro)).color(t.text));
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
pub fn grid(ui: &mut Ui, id: &str, count: usize, mut cell: impl FnMut(&mut Ui, usize, f32)) {
    let t = Tokens::get();
    let (columns, width) = grid_geometry(ui.available_width(), count);
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
    let (rect, response) = allocate(ui, id, width, t.file_tile_height, label, enabled, None);
    let hot = enabled && (response.hovered() || response.has_focus());
    let fill = if primary {
        t.file_primary
    } else if hot {
        t.hover
    } else {
        t.file_surface
    };
    let border = if primary || response.has_focus() { Stroke::new(1.5, t.file_primary_border) } else { Stroke::new(1.0, t.file_border) };
    let mut painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
    if !enabled {
        painter.multiply_opacity(0.45);
    }
    painter.rect(rect, t.radius_lg, fill, border, StrokeKind::Inside);
    let inner = rect.shrink(t.space_16);
    let icon_size = t.file_tile_icon.min(inner.height()).min(inner.width());
    let icon_center = pos2(inner.left() + icon_size / 2.0, inner.center().y);
    icon.paint_sized(&painter, icon_center, icon_size * 0.75, t, true);
    let text_rect = Rect::from_min_max(pos2((inner.left() + icon_size + t.space_12).min(inner.right()), inner.top()), inner.max);
    let ink = if primary { t.file_primary_ink } else { t.text };
    let mut title = egui::text::LayoutJob::simple_singleline(label.to_owned(), fonts::semibold(t.font_heading), ink);
    title.wrap.max_width = text_rect.width();
    title.wrap.max_rows = 1;
    let title = painter.layout_job(title);
    let mut detail = egui::text::LayoutJob::simple(caption.to_owned(), FontId::proportional(t.font_label), t.file_dim, text_rect.width());
    detail.wrap.max_rows = 2;
    let detail = painter.layout_job(detail);
    let top = text_rect.center().y - (title.size().y + t.space_2 + detail.size().y) / 2.0;
    let title_height = title.size().y;
    painter.galley(pos2(text_rect.left(), top), title, ink);
    painter.galley(pos2(text_rect.left(), top + title_height + t.space_2), detail, t.file_dim);
    response.on_hover_text(format!("{label}\n{caption}"))
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
