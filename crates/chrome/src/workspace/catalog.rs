//! Movable catalog cards and symbol tiles, adapted from PCBcraft Next's Parts library.
//! The caller owns the catalog data and actions; this module owns presentation and panel geometry.

use egui::{Align, Align2, FontId, Layout, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use super::{FLOATING_GAP, TreeIcon, floating_frame, paint_tree_icon};
use crate::{MenuItem, Tokens, fonts, menu};

const WIDTH: f32 = 260.0;
const HEIGHT: f32 = 490.0;
const COMPACT_HEIGHT: f32 = 340.0;
const HEADER_HEIGHT: f32 = 26.0;
const MIN_DOCK_HEIGHT: f32 = 340.0;
const SNAP_DISTANCE: f32 = 44.0;

/// Anchoring a card to an edge still overlays the canvas; it never reserves a side panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Position {
    #[default]
    Floating,
    Left,
    Right,
}

/// Saved placement excludes a live drag and uses logical coordinates within the host.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelState {
    position: Position,
    at: Option<[f32; 2]>,
    collapsed: bool,
}

impl PanelState {
    /// Bound saved coordinates before restoring; normal panel layout clamps them to the host.
    pub fn valid(&self) -> bool {
        self.at.is_none_or(|at| at.iter().all(|n| n.is_finite() && n.abs() <= 100_000.0))
    }
}

/// Disposable placement state for one floating card. It is never saved in project files.
#[derive(Debug, Default)]
pub struct Panel {
    position: Position,
    at: Option<Pos2>,
    dragging: Option<(Pos2, Pos2, Position)>,
    collapsed: bool,
}

/// Header actions switch presentation or close the catalog without changing project data.
#[derive(Default)]
pub struct PanelAction {
    pub close: bool,
    pub expand: bool,
}

impl Panel {
    /// Start a catalog at an explicit edge; subsequent dragging and position menus retain
    /// the person's chosen placement independently of the expanded chooser.
    pub fn at_position(position: Position) -> Self {
        Self { position, ..Self::default() }
    }

    /// Capture presentation without preserving an in-progress drag.
    pub fn saved_state(&self) -> PanelState {
        PanelState { position: self.position, at: self.at.map(|at| [at.x, at.y]), collapsed: self.collapsed }
    }

    /// Restore validated placement; rendering will constrain it to the current window.
    pub fn from_state(state: &PanelState) -> Self {
        Self { position: state.position, at: state.at.map(|[x, y]| pos2(x, y)), collapsed: state.collapsed, dragging: None }
    }

    /// Reveal the catalog without changing its position or the caller's search/view state.
    pub fn expand(&mut self) {
        self.collapsed = false;
    }

    /// Whether only the draggable header and its controls are visible.
    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    /// Height of the header-only card, including the shared frame's border and padding.
    fn collapsed_height() -> f32 {
        HEADER_HEIGHT + floating_frame().total_margin().sum().y
    }

    /// Limit the Browser's height when this panel is anchored beneath it, leaving room for a
    /// compact catalog. The full canvas rectangle remains unchanged.
    pub fn browser_area(&self, canvas: Rect) -> Rect {
        if self.position == Position::Left {
            let height = if self.collapsed { Self::collapsed_height() } else { MIN_DOCK_HEIGHT };
            Rect::from_min_max(canvas.min, pos2(canvas.right(), (canvas.bottom() - height - FLOATING_GAP).max(canvas.top() + 80.0)))
        } else {
            canvas
        }
    }

    /// Restore the pre-drag position on Escape. Returns whether a panel drag was cancelled.
    pub fn cancel_drag(&mut self) -> bool {
        if let Some((_, at, position)) = self.dragging.take() {
            self.at = Some(at);
            self.position = position;
            return true;
        }
        false
    }

    /// Calculate the card inside the current canvas. Edge anchors follow window resizing;
    /// a floating position is clamped to keep its header and controls reachable.
    fn rect(&self, canvas: Rect, browser: Rect, position: Position) -> Rect {
        let bounds = canvas.shrink(FLOATING_GAP);
        let width = WIDTH.min(bounds.width().max(1.0));
        let bottom = bounds.bottom();
        let top = if position == Position::Left && browser.is_positive() {
            let minimum = if self.collapsed { Self::collapsed_height() } else { 120.0 };
            (browser.bottom() + FLOATING_GAP).min((bottom - minimum).max(bounds.top()))
        } else {
            bounds.top()
        };
        let height = if self.collapsed {
            Self::collapsed_height().min(bottom - top)
        } else if position == Position::Floating {
            HEIGHT.min(bounds.height())
        } else {
            bottom - top
        };
        let size = vec2(width, height.max(1.0));
        let desired = match position {
            Position::Left => pos2(bounds.left(), top),
            Position::Right => pos2(bounds.right() - width, top),
            Position::Floating => self.at.unwrap_or(bounds.min + vec2(280.0, 36.0)),
        };
        let at = pos2(
            desired.x.clamp(bounds.left(), (bounds.right() - size.x).max(bounds.left())),
            desired.y.clamp(bounds.top(), (bounds.bottom() - size.y).max(bounds.top())),
        );
        Rect::from_min_size(at, size)
    }

    /// Choose an edge anchor only when the pointer is inside the workspace's vertical bounds.
    fn snap(canvas: Rect, pointer: Pos2) -> Position {
        let bounds = canvas.shrink(FLOATING_GAP);
        if pointer.y < bounds.top() || pointer.y > bounds.bottom() {
            Position::Floating
        } else if pointer.x < bounds.left() + SNAP_DISTANCE {
            Position::Left
        } else if pointer.x > bounds.right() - SNAP_DISTANCE {
            Position::Right
        } else {
            Position::Floating
        }
    }

    /// Drag only from the grip/title, remembering the starting anchor for Escape and snapping
    /// on release. Pointer deltas are always measured from the initial press.
    fn drag(&mut self, response: &Response, rect: Rect, canvas: Rect) {
        if response.drag_started()
            && let Some(origin) = response.ctx.input(|i| i.pointer.press_origin())
        {
            self.dragging = Some((origin, rect.min, self.position));
        }
        if response.dragged()
            && let Some((origin, at, _)) = self.dragging
            && let Some(pointer) = response.interact_pointer_pos()
        {
            self.position = Position::Floating;
            self.at = Some(at + (pointer - origin));
            response.ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if response.drag_stopped()
            && self.dragging.take().is_some()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            self.position = Self::snap(canvas, pointer);
            response.ctx.request_repaint();
        }
    }

    /// Draw a floating card with drag, collapse, position and close controls. A collapsed card
    /// occupies only its header and does not call `add`; otherwise `add` receives whether the
    /// compact list is needed. Inherits the host's enabled state so a project dialog can also
    /// disable cards in another window. Returns its actual rectangle and header actions.
    pub fn show(&mut self, ui: &Ui, canvas: Rect, browser: Rect, add: impl FnOnce(&mut Ui, bool)) -> (Rect, PanelAction) {
        let rect = self.rect(canvas, browser, self.position);
        let t = Tokens::get();
        let shown = egui::Area::new(egui::Id::unique("parts_browser"))
            .enabled(ui.is_enabled())
            .order(egui::Order::Middle)
            .fixed_pos(rect.min)
            .movable(false)
            .constrain(false)
            .fade_in(false)
            .show(ui.ctx(), |ui| {
                ui.set_max_size(rect.size());
                floating_frame()
                    .show(ui, |ui| {
                        // Allocate the whole host once, so scrolling never changes the card bounds.
                        let size = (rect.size() - floating_frame().total_margin().sum()).max(Vec2::ZERO);
                        let (body, _) = ui.allocate_exact_size(size, Sense::hover());
                        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(body));
                        ui.set_clip_rect(ui.clip_rect().intersect(body));
                        ui.spacing_mut().item_spacing.y = t.space_6;
                        ui.spacing_mut().interact_size.y = 24.0;
                        ui.style_mut().text_styles.insert(egui::TextStyle::Small, FontId::proportional(t.font_small));
                        let widgets = &mut ui.visuals_mut().widgets;
                        for widget in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active] {
                            widget.corner_radius = t.radius_md.into();
                        }
                        let mut action = PanelAction::default();
                        ui.horizontal(|ui| {
                            let controls_width = 4.0 * (24.0 + ui.spacing().item_spacing.x);
                            let (header, response) =
                                ui.allocate_exact_size(vec2((ui.available_width() - controls_width).max(1.0), HEADER_HEIGHT), Sense::drag());
                            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Move Parts Catalog"));
                            super::drag_grip(ui.painter(), header);
                            ui.painter().text(
                                header.left_center() + vec2(18.0, 0.0),
                                Align2::LEFT_CENTER,
                                "Parts Catalog",
                                fonts::interface_heading(ui, t.font_body),
                                t.text,
                            );
                            self.drag(&response, rect, canvas);
                            response
                                .on_hover_cursor(egui::CursorIcon::Grab)
                                .on_hover_text("Drag to float. Release at the left or right edge to dock.");
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                action.close = icon_button(ui, "Close Parts Catalog", HeaderIcon::Close).clicked();
                                action.expand = icon_button(ui, "Expand Parts Catalog", HeaderIcon::Full).clicked();
                                let label = if self.collapsed { "Expand catalog panel" } else { "Collapse catalog panel" };
                                if icon_button(ui, label, HeaderIcon::Collapse(self.collapsed)).clicked() {
                                    self.collapsed = !self.collapsed;
                                    ui.ctx().request_repaint();
                                }
                                let more = icon_button(ui, "Parts Catalog position", HeaderIcon::More);
                                egui::Popup::menu(&more).frame(menu::frame()).show(|ui| {
                                    ui.set_width(180.0);
                                    ui.label(crate::shell::dim("PANEL POSITION"));
                                    for (position, label) in
                                        [(Position::Floating, "Float"), (Position::Left, "Dock left"), (Position::Right, "Dock right")]
                                    {
                                        if MenuItem::new(label).choice(self.position == position).show(ui) {
                                            self.position = position;
                                            self.at = None;
                                            self.dragging = None;
                                            ui.close();
                                            ui.ctx().request_repaint();
                                        }
                                    }
                                });
                            });
                        });
                        if !self.collapsed {
                            ui.separator();
                            add(&mut ui, rect.height() < COMPACT_HEIGHT);
                        }
                        action
                    })
                    .inner
            });
        if self.dragging.is_some()
            && let Some(pointer) = ui.ctx().input(|i| i.pointer.interact_pos())
        {
            let position = Self::snap(canvas, pointer);
            if position != Position::Floating {
                let target = self.rect(canvas, browser, position);
                let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("parts_snap_preview")));
                painter.rect_filled(target, super::FLOATING_RADIUS, t.accent_soft.gamma_multiply(0.7));
                painter.rect_stroke(target, super::FLOATING_RADIUS, Stroke::new(2.0, t.accent), StrokeKind::Inside);
                painter.text(target.center(), Align2::CENTER_CENTER, "Release to dock", fonts::interface_heading(ui, t.font_body), t.text);
            }
        }
        (shown.response.rect, shown.inner)
    }
}

enum HeaderIcon {
    Close,
    More,
    Collapse(bool),
    Full,
}

/// Quiet 24-point header controls, with explicit accessible names rather than glyph labels.
fn icon_button(ui: &mut Ui, label: &str, icon: HeaderIcon) -> Response {
    let t = Tokens::get();
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    if matches!(icon, HeaderIcon::More) || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, t.radius_md, if response.hovered() || response.has_focus() { t.hover } else { t.panel_header });
    }
    match icon {
        HeaderIcon::Full => {
            let c = rect.center();
            let stroke = Stroke::new(1.1, t.text);
            for sign in [-1., 1.] {
                ui.painter().line_segment([c + vec2(-1., 1.) * sign, c + vec2(-5., 5.) * sign], stroke);
                ui.painter().add(egui::Shape::line(vec![c + vec2(-5., 1.) * sign, c + vec2(-5., 5.) * sign, c + vec2(-1., 5.) * sign], stroke));
            }
        }
        HeaderIcon::More => {
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "…", FontId::proportional(t.font_body), t.text);
        }
        HeaderIcon::Close => {
            for sign in [-1.0, 1.0] {
                ui.painter().line_segment([rect.center() + vec2(-3.5, -3.5 * sign), rect.center() + vec2(3.5, 3.5 * sign)], Stroke::new(1.0, t.text));
            }
        }
        HeaderIcon::Collapse(collapsed) => {
            let c = rect.center();
            let sign = if collapsed { -1.0 } else { 1.0 };
            ui.painter().add(egui::Shape::line(
                vec![c + vec2(-4.0, -2.0 * sign), c + vec2(0.0, 2.0 * sign), c + vec2(4.0, -2.0 * sign)],
                Stroke::new(1.3, t.text_dim),
            ));
        }
    }
    response.on_hover_text(label)
}

/// Catalog preview density. A short dock uses List without replacing the person's preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum View {
    Small,
    #[default]
    Medium,
    Large,
    List,
}

impl View {
    /// Shared fixed tile height keeps virtual scrolling and the painter in agreement.
    fn tile_height(self) -> f32 {
        (match self {
            Self::Small => 94.,
            Self::Medium => 126.,
            Self::Large => 180.,
            Self::List => 58.,
        }) * (Tokens::get().font_body / 12.).max(1.)
    }
    /// Number of equal-width tiles in a catalog row.
    fn columns(self) -> usize {
        match self {
            Self::Small => 3,
            Self::Medium => 2,
            Self::Large | Self::List => 1,
        }
    }
}

/// Show the source and match count when the body has room above search and the compact list.
/// Very short docks omit this row, as in the prototype's cards below 240 points tall.
pub fn source(ui: &mut Ui, label: &str, count: usize) {
    if ui.available_height() < 180.0 {
        return;
    }
    let t = Tokens::get();
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        paint_tree_icon(ui.painter(), TreeIcon::Folder, rect.center(), 14.0, t);
        ui.label(crate::shell::dim(label).size(t.font_small));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(crate::shell::dim(format!("{count} {}", if count == 1 { "part" } else { "parts" })).size(t.font_small));
        });
    });
}

/// Shared full-width parts search. The caller manages query changes, focus and Enter.
pub fn search(ui: &mut Ui, query: &mut String) -> Response {
    solidcraft_ui_kit::search_field(ui, ui.make_persistent_id("parts_search"), query, "Search parts")
}

/// Shared equal-width density segments. Short panes use List while retaining the saved choice.
pub fn view_picker(ui: &mut Ui, view: &mut View, compact: bool) -> View {
    if compact {
        return View::List;
    }
    let views = [View::Small, View::Medium, View::Large, View::List];
    let mut selected = views.iter().position(|candidate| candidate == view).unwrap_or_default();
    solidcraft_ui_kit::segmented(
        ui,
        &mut selected,
        &["Small", "Medium", "Large", "List"],
        solidcraft_ui_kit::Segments { small: true, compact: true, ..Default::default() },
    );
    *view = views[selected];
    *view
}

/// Scroll equal-width rows inside the remaining body and keep a single footer visible.
/// An explicit reveal scrolls once to that item's row, including in virtualized catalogs.
/// Small catalogs retain all accessibility targets; large catalogs paint only visible rows.
pub fn grid<T>(ui: &mut Ui, view: View, items: &[T], reveal: Option<usize>, mut add: impl FnMut(&mut Ui, &T, f32)) {
    let columns = view.columns();
    ui.add_space(3.0);
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt("library_parts")
        .max_height((ui.available_height() - 36.0).max(0.0))
        .min_scrolled_height(0.0)
        .auto_shrink([false, false]);
    if let Some(index) = reveal.filter(|index| *index < items.len()) {
        scroll = scroll.vertical_scroll_offset((index / columns) as f32 * (view.tile_height() + ui.spacing().item_spacing.y));
    }
    let rows = items.len().div_ceil(columns).max(1);
    let mut render = |ui: &mut Ui, rows: std::ops::Range<usize>| {
        if items.is_empty() {
            ui.label(crate::shell::dim("No matching parts"));
            return;
        }
        let width = (ui.available_width() - 8.0 * (columns - 1) as f32) / columns as f32;
        for index in rows {
            let start = index * columns;
            let row = &items[start..(start + columns).min(items.len())];
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.;
                for item in row {
                    add(ui, item, width);
                }
            });
        }
    };
    // Small catalogs retain off-screen accessibility targets for direct keyboard navigation.
    // Large external sources bound geometry preparation to the visible row range.
    if items.len() <= 64 {
        scroll.show(ui, |ui| render(ui, 0..rows));
    } else {
        scroll.show_rows(ui, view.tile_height(), rows, render);
    }
}

/// Labels for a catalog item. The item data and symbol geometry stay in the application.
pub struct Item<'a> {
    pub name: &'a str,
    pub value: &'a str,
    /// Source-qualified identity disambiguates equally named external items for accessibility.
    pub identity: Option<&'a str>,
    pub selected: bool,
}

/// Draw a raised preview with two label lines and a SYM badge (except in List).
/// The preview painter is clipped to its tile even when scrolled partly out of the card.
pub fn tile(ui: &mut Ui, view: View, width: f32, item: Item<'_>, paint: impl FnOnce(&egui::Painter, Rect, egui::Color32)) -> Response {
    let t = Tokens::get();
    let height = view.tile_height();
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
    let accessible = item.identity.map_or_else(|| item.name.to_owned(), |identity| format!("{} · {identity}", item.name));
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), item.selected, accessible.as_str()));
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let preview = if view == View::List {
        Rect::from_min_size(rect.min, vec2(66.0, height - 4.0))
    } else {
        Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - 35.0))
    };
    painter.rect_filled(preview, t.radius_md, if item.selected { t.accent_soft } else { t.panel_header });
    if item.selected || response.hovered() || response.has_focus() {
        painter.rect_stroke(
            preview.shrink(0.5),
            t.radius_md,
            Stroke::new(1.0, if item.selected || response.has_focus() { t.accent } else { t.border }),
            StrokeKind::Inside,
        );
    }
    paint(&painter, preview.shrink2(vec2(10.0, 12.5)).translate(vec2(0.0, 3.0)), t.catalog_symbol);
    if view != View::List {
        let badge = Rect::from_min_size(preview.min + vec2(5.0, 5.0), vec2(28.0, 13.0));
        painter.rect_filled(badge, 3, t.panel);
        painter.text(badge.center(), Align2::CENTER_CENTER, "SYM", FontId::proportional(8.0), t.text_dim);
    }
    let (at, max_width) =
        if view == View::List { (rect.min + vec2(76.0, 9.0), width - 80.0) } else { (pos2(rect.left() + 2.0, preview.bottom() + 4.0), width - 4.0) };
    for (label, offset, font, ink) in
        [(item.name, 0.0, fonts::interface_heading(ui, t.font_small), t.text), (item.value, 16.0, FontId::proportional(t.font_small), t.text_dim)]
    {
        painter.galley(at + vec2(0.0, offset), super::elided(&painter, label, font, ink, max_width.max(0.0)), ink);
    }
    response.on_hover_text(format!("{} · {}\nDouble-click to place or drag onto the sheet.", item.name, item.value))
}

/// Find the largest rectangular space clear of every visible card, including cards floating
/// in the middle. Candidate horizontal edges come from the canvas and obstacle boundaries;
/// scanning their vertical gaps finds the largest empty rectangle without a pixel grid.
pub fn uncovered(canvas: Rect, cards: &[Rect]) -> Rect {
    let cards: Vec<_> = cards.iter().map(|r| r.intersect(canvas)).filter(Rect::is_positive).collect();
    let mut xs = vec![canvas.left(), canvas.right()];
    for card in &cards {
        xs.extend([card.left(), card.right()]);
    }
    xs.sort_by(f32::total_cmp);
    xs.dedup();
    let mut best = Rect::from_min_size(canvas.center(), Vec2::ZERO);
    for (i, &left) in xs.iter().enumerate() {
        for &right in &xs[i + 1..] {
            let mut blocks: Vec<_> = cards.iter().filter(|r| r.left() < right && r.right() > left).collect();
            blocks.sort_by(|a, b| a.top().total_cmp(&b.top()));
            let mut top = canvas.top();
            let end = Rect::from_min_max(pos2(left, canvas.bottom()), pos2(right, canvas.bottom()));
            for block in blocks.into_iter().chain(std::iter::once(&end)) {
                let candidate = Rect::from_min_max(pos2(left, top), pos2(right, block.top()));
                if candidate.is_positive() && candidate.area() > best.area() {
                    best = candidate;
                }
                top = top.max(block.bottom());
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Large source catalogs must not decode and paint every off-screen symbol on each frame.
    #[test]
    fn catalog_grid_only_paints_visible_rows() {
        let ctx = egui::Context::default();
        let mut count = 0;
        let items: Vec<_> = (0..1000).collect();
        let output =
            ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300., 500.))), ..Default::default() }, |ui| {
                grid(ui, View::Large, &items, None, |ui, _, width| {
                    count += 1;
                    ui.allocate_exact_size(vec2(width, View::Large.tile_height()), Sense::hover());
                })
            });
        output.drop_without_applying_deltas();
        assert!(count > 0 && count < 20, "painted {count} items");
    }

    /// The available space must handle overlapping cards, equal top edges and hidden cards.
    #[test]
    fn clear_rectangle_handles_overlapping_and_hidden_cards() {
        let rect = |x1, y1, x2, y2| Rect::from_min_max(pos2(x1, y1), pos2(x2, y2));
        let canvas = rect(0.0, 0.0, 1000.0, 800.0);
        assert_eq!(uncovered(canvas, &[Rect::NOTHING]), canvas);
        let cards = [rect(0.0, 0.0, 260.0, 200.0), rect(0.0, 0.0, 260.0, 700.0), rect(740.0, 0.0, 1000.0, 800.0)];
        assert_eq!(uncovered(canvas, &cards), rect(260.0, 0.0, 740.0, 800.0));
        let middle = rect(300.0, 100.0, 560.0, 590.0);
        assert_eq!(uncovered(canvas, &[cards[0], middle]), rect(560.0, 0.0, 1000.0, 800.0));
    }
}
