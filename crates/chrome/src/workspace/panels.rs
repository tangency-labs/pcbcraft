//! Movable contextual cards share the workspace frame and Parts' edge-docking gestures.
use super::{FLOATING_GAP, FLOATING_HEADER, FLOATING_RADIUS, floating_card_min_height, floating_frame};
use crate::{MenuItem, Tokens, menu};
use egui::{Align2, Id, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

/// Hosts below this width stack a full-width inspector beneath Browser.
pub const COMPACT_HOST_WIDTH: f32 = 600.0;
/// Preferred space for the lower-right filter card before its body needs scrolling.
pub const FILTER_DOCK_HEIGHT: f32 = 280.0;

/// Keep contextual card widths consistent with the shared inspector and floating workspace cards.
pub fn card_width(canvas: Rect, wide_compact: bool) -> f32 {
    if wide_compact { if canvas.width() < COMPACT_HOST_WIDTH { canvas.width() } else { 260.0 } } else { (canvas.width() * 0.4).min(260.0) }
}

/// A card's edge anchor. Floating coordinates are relative to its current host's canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    Left,
    #[default]
    Right,
    Floating,
}

/// Serializable placement excludes live dragging, focus and authored project state.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelState {
    /// Left/right docking or an independently floating card.
    pub position: Position,
    /// Floating offset from the canvas origin, in logical points.
    pub at: Option<[f32; 2]>,
    /// Optional floating dimensions; omitted by older local layout records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[f32; 2]>,
}

impl PanelState {
    /// Reject non-finite or unbounded coordinates and nonpositive dimensions before restoring a layout.
    pub fn valid(&self) -> bool {
        self.at.is_none_or(|at| at.iter().all(|value| value.is_finite() && value.abs() <= 100_000.0))
            && self.size.is_none_or(|size| size.iter().all(|value| value.is_finite() && *value > 0.0 && *value <= 100_000.0))
    }
}

#[derive(Debug)]
struct Drag {
    pointer: Pos2,
    at: Pos2,
    before: PanelState,
}

#[derive(Debug)]
struct Resize {
    pointer: Pos2,
    size: egui::Vec2,
    before: PanelState,
    horizontal: bool,
    leading: bool,
}

/// Runtime card placement; cloning preserves layout but never transfers an active pointer drag.
#[derive(Debug, Default)]
pub struct Panel {
    state: PanelState,
    drag: Option<Drag>,
    resize: Option<Resize>,
}

impl Clone for Panel {
    /// A copied view starts from the current placement without taking the source's drag.
    fn clone(&self) -> Self {
        Self::from_state(self.state)
    }
}

impl Panel {
    /// Start at an edge without a machine-specific absolute window position.
    pub fn new(position: Position) -> Self {
        Self::from_state(PanelState { position, at: None, size: None })
    }

    /// Restore placement after the caller validates its local arrangement record.
    pub fn from_state(state: PanelState) -> Self {
        Self { state, drag: None, resize: None }
    }

    /// Save only presentation data, with no pending drag authority.
    pub fn saved_state(&self) -> PanelState {
        self.state
    }

    /// Inspect the anchor used by the caller's stacking layout.
    pub fn position(&self) -> Position {
        self.state.position
    }

    /// Apply the same validated position change for a menu action or live-editor request.
    pub fn set_position(&mut self, position: Position, at: Option<[f32; 2]>) -> bool {
        let next = PanelState { position, at: if position == Position::Floating { at } else { None }, size: self.state.size };
        if !next.valid() {
            return false;
        }
        self.state = next;
        self.drag = None;
        self.resize = None;
        true
    }

    /// Escape or focus loss restores placement and dimensions from before a header or edge drag.
    pub fn cancel_drag(&mut self) -> bool {
        if let Some(resize) = self.resize.take() {
            self.state = resize.before;
            true
        } else if let Some(drag) = self.drag.take() {
            self.state = drag.before;
            true
        } else {
            false
        }
    }

    /// Floating workspace cards reuse the toolkit's accessible edge sashes. Changes
    /// are bounded to this host and cancel through the same lifecycle as header dragging.
    fn resize_edges(&mut self, ui: &mut Ui, id: Id, title: &str, rect: Rect, canvas: Rect) {
        let t = Tokens::get();
        let bounds = canvas.shrink(FLOATING_GAP);
        let min = vec2(240.0, 180.0).min(bounds.size());
        let max = bounds.size();
        for (horizontal, leading) in [(false, true), (false, false), (true, false)] {
            let bounds = if horizontal {
                Rect::from_min_max(rect.left_bottom() - vec2(0.0, t.space_6), rect.right_bottom())
            } else if leading {
                Rect::from_min_max(
                    pos2(rect.left(), rect.top() + FLOATING_HEADER + t.space_24),
                    pos2(rect.left() + t.space_6, rect.bottom() - t.space_6),
                )
            } else {
                Rect::from_min_max(
                    pos2(rect.right() - t.space_6, rect.top() + FLOATING_HEADER + t.space_24),
                    rect.right_bottom() - vec2(0.0, t.space_6),
                )
            };
            let label = format!(
                "Resize {title} {}",
                if horizontal {
                    "height"
                } else if leading {
                    "width from left"
                } else {
                    "width"
                }
            );
            let response = solidcraft_ui_kit::panel_host_sash(
                ui,
                id.with(("resize", horizontal, leading)),
                bounds,
                &label,
                horizontal,
                if horizontal { rect.height() } else { rect.width() },
            );
            if response.clicked() || response.drag_started() {
                response.request_focus();
            }
            if response.drag_started()
                && let Some(pointer) = ui.input(|i| i.pointer.press_origin())
            {
                self.resize = Some(Resize { pointer, size: rect.size(), before: self.state, horizontal, leading });
            }
            let mut next = None;
            if response.dragged()
                && let Some(resize) = &self.resize
                && resize.horizontal == horizontal
                && resize.leading == leading
                && let Some(pointer) = response.interact_pointer_pos()
            {
                let delta = pointer - resize.pointer;
                next = Some(resize.size + if horizontal { vec2(0.0, delta.y) } else { vec2(delta.x * if leading { -1.0 } else { 1.0 }, 0.0) });
            }
            if response.has_focus() {
                let delta = ui.input_mut(|i| {
                    let (less, more) =
                        if horizontal { (egui::Key::ArrowUp, egui::Key::ArrowDown) } else { (egui::Key::ArrowLeft, egui::Key::ArrowRight) };
                    (i.consume_key(egui::Modifiers::NONE, more) as i8 - i.consume_key(egui::Modifiers::NONE, less) as i8) as f32 * t.space_16
                });
                if delta != 0.0 {
                    next = Some(rect.size() + if horizontal { vec2(0.0, delta) } else { vec2(delta * if leading { -1.0 } else { 1.0 }, 0.0) });
                }
            }
            if let Some(size) = next {
                let size = size.clamp(min, max);
                if self.state.size != Some([size.x, size.y]) {
                    self.state.size = Some([size.x, size.y]);
                    if leading {
                        self.state.at = Some([rect.right() - size.x - canvas.left(), rect.top() - canvas.top()]);
                    }
                    ui.ctx().request_repaint();
                }
            }
            if response.drag_stopped() {
                self.resize = None;
            }
        }
    }

    /// Snap only inside the canvas' vertical span, using Parts' 44-point edge target.
    fn snap(canvas: Rect, pointer: Pos2) -> Position {
        let bounds = canvas.shrink(FLOATING_GAP);
        if pointer.y < bounds.top() || pointer.y > bounds.bottom() {
            Position::Floating
        } else if pointer.x <= bounds.left() + 44.0 {
            Position::Left
        } else if pointer.x >= bounds.right() - 44.0 {
            Position::Right
        } else {
            Position::Floating
        }
    }

    /// Drag deltas are measured from the initial press; release chooses an edge or leaves a float.
    fn drag_header(&mut self, response: &Response, rect: Rect, canvas: Rect) {
        if response.drag_started()
            && let Some(pointer) = response.ctx.input(|i| i.pointer.press_origin())
        {
            self.drag = Some(Drag { pointer, at: rect.min, before: self.state });
        }
        if response.dragged()
            && let Some(drag) = &self.drag
            && let Some(pointer) = response.interact_pointer_pos()
        {
            self.state.position = Position::Floating;
            let at = drag.at + (pointer - drag.pointer) - canvas.min.to_vec2();
            self.state.at = Some([at.x, at.y]);
        }
        if response.drag_stopped()
            && self.drag.take().is_some()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let position = Self::snap(canvas, pointer);
            if position != Position::Floating {
                self.set_position(position, None);
            }
            response.ctx.request_repaint();
        }
    }

    /// Render a card inside its dock slot or clamp its floating header to the host. Content
    /// scrolls within the available body; the returned rectangle participates in Fit and hit exclusion.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &Ui,
        id: Id,
        title: &str,
        canvas: Rect,
        dock: Rect,
        collapsed: bool,
        width: f32,
        add: impl FnOnce(&mut Ui),
    ) -> (Rect, bool) {
        self.show_with_toolbar(ui, id, title, canvas, dock, collapsed, width, 0., |_| {}, add)
    }

    /// Keep a card's actions above its scrolling body so revealing content cannot hide them.
    /// The toolbar shares collapse, docking and input blocking with the existing card shell.
    /// Below its declared minimum height, actions join the scrolling content to stay in bounds.
    #[allow(clippy::too_many_arguments)]
    pub fn show_with_toolbar(
        &mut self,
        ui: &Ui,
        id: Id,
        title: &str,
        canvas: Rect,
        dock: Rect,
        collapsed: bool,
        width: f32,
        toolbar_min_height: f32,
        toolbar: impl FnOnce(&mut Ui),
        add: impl FnOnce(&mut Ui),
    ) -> (Rect, bool) {
        self.show_card(ui, id, title, canvas, dock, collapsed, width, toolbar_min_height, false, toolbar, add)
    }

    /// Use an exact, reserved dock slot for a workspace column. Floating remains opt-in
    /// through the same header drag and position menu as contextual canvas cards.
    /// Floating slots offer shared resize sashes, full-host expansion and default-size reset.
    #[allow(clippy::too_many_arguments)]
    pub fn show_in_slot(
        &mut self,
        ui: &Ui,
        id: Id,
        title: &str,
        canvas: Rect,
        dock: Rect,
        collapsed: bool,
        add: impl FnOnce(&mut Ui),
    ) -> (Rect, bool) {
        self.show_card(ui, id, title, canvas, dock, collapsed, dock.width(), 0.0, true, |_| {}, add)
    }

    /// Preserve contextual-card spacing while allowing filled workspace columns to align
    /// exactly with their neighbors. The same drag state and controls serve both layouts.
    #[allow(clippy::too_many_arguments)]
    fn show_card(
        &mut self,
        ui: &Ui,
        id: Id,
        title: &str,
        canvas: Rect,
        dock: Rect,
        collapsed: bool,
        width: f32,
        toolbar_min_height: f32,
        slot: bool,
        toolbar: impl FnOnce(&mut Ui),
        add: impl FnOnce(&mut Ui),
    ) -> (Rect, bool) {
        if !ui.is_enabled()
            || ui.ctx().input(|i| i.key_pressed(egui::Key::Escape) || i.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))))
        {
            self.cancel_drag();
        }
        let floating = self.state.position == Position::Floating;
        let area = if floating { canvas } else { dock };
        let fill_slot = slot && !floating;
        let gap = if fill_slot { 0.0 } else { FLOATING_GAP };
        if area.height() < floating_card_min_height() || !area.is_positive() {
            self.cancel_drag();
            return (Rect::NOTHING, false);
        }
        let requested = if slot && floating { self.state.size.unwrap_or([width, 440.0]) } else { [width, 440.0] };
        let room = (canvas.size() - vec2(2.0 * gap, 2.0 * gap)).max(vec2(1.0, 1.0));
        let width = if slot && floating { requested[0].clamp(240.0_f32.min(room.x), room.x) } else { requested[0].min(room.x) };
        let height = if slot && floating {
            requested[1].clamp(180.0_f32.min(room.y), room.y)
        } else if floating {
            requested[1].min(room.y)
        } else {
            area.height() - 2.0 * gap
        };
        let body_height = (height - floating_frame(ui.ctx()).total_margin().sum().y - FLOATING_HEADER - Tokens::get().space_8).max(0.0);
        let bounds = canvas.shrink(gap);
        let desired = if floating {
            self.state.at.map(|[x, y]| canvas.min + vec2(x, y)).unwrap_or(bounds.min + vec2(32.0, 32.0))
        } else {
            pos2(if self.state.position == Position::Left { area.left() + gap } else { area.right() - width - gap }, area.top() + gap)
        };
        let at = pos2(
            desired.x.clamp(bounds.left(), (bounds.right() - width).max(bounds.left())),
            desired.y.clamp(
                bounds.top(),
                (bounds.bottom() - if slot && floating { height } else { floating_card_min_height() - 2.0 * FLOATING_GAP }).max(bounds.top()),
            ),
        );
        let body_height = body_height
            .min((bounds.bottom() - at.y - floating_frame(ui.ctx()).total_margin().sum().y - FLOATING_HEADER - Tokens::get().space_8).max(0.0));
        let t = Tokens::get();
        let shown = egui::Area::new(id)
            .enabled(ui.is_enabled())
            .order(egui::Order::Middle)
            .fixed_pos(at)
            .movable(false)
            .constrain(false)
            .fade_in(false)
            .show(ui.ctx(), |ui| {
                ui.set_max_height(height);
                let frame = floating_frame(ui.ctx()).show(ui, |ui| {
                    ui.set_width(width - floating_frame(ui.ctx()).total_margin().sum().x);
                    if slot && !collapsed {
                        ui.set_min_height((height - floating_frame(ui.ctx()).total_margin().sum().y).max(0.0));
                    }
                    ui.spacing_mut().item_spacing.y = t.space_2;
                    let mut collapse = false;
                    ui.horizontal(|ui| {
                        let header_width = (width - floating_frame(ui.ctx()).total_margin().sum().x).min(ui.available_width());
                        let remaining = (header_width - 44.0 - 2.0 * ui.spacing().item_spacing.x).max(1.0);
                        let (id, header) = ui.allocate_space(vec2(remaining, FLOATING_HEADER));
                        let response = super::panel_header_control(ui, id, header, title, true);
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), format!("Move {title}")));
                        self.drag_header(&response, Rect::from_min_size(at, vec2(width, height)), canvas);
                        response.on_hover_cursor(egui::CursorIcon::Grab);
                        let (more_rect, more) = ui.allocate_exact_size(vec2(22.0, FLOATING_HEADER), Sense::click());
                        if more.hovered() || more.has_focus() {
                            ui.painter().rect_filled(more_rect, t.radius_sm, t.hover);
                        }
                        ui.painter().text(more_rect.center(), Align2::CENTER_CENTER, "…", egui::FontId::proportional(t.font_button), t.text_dim);
                        more.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), format!("{title} position")));
                        egui::Popup::menu(&more).frame(menu::frame()).show(|ui| {
                            for (position, label) in [(Position::Floating, "Float"), (Position::Left, "Dock left"), (Position::Right, "Dock right")] {
                                if MenuItem::new(label).choice(self.state.position == position).show(ui) {
                                    let offset = at - canvas.min;
                                    self.set_position(position, Some([offset.x, offset.y]));
                                    ui.close();
                                    ui.ctx().request_repaint();
                                }
                            }
                            if slot && floating {
                                menu::separator(ui);
                                if MenuItem::new(&format!("Expand {title}")).show(ui) {
                                    self.state.size = Some([bounds.width(), bounds.height()]);
                                    self.state.at = Some([FLOATING_GAP, FLOATING_GAP]);
                                    ui.ctx().request_repaint();
                                }
                                if MenuItem::new(&format!("Reset {title} size")).show(ui) {
                                    self.state.size = None;
                                    ui.ctx().request_repaint();
                                }
                            }
                        });
                        let (id, rect) = ui.allocate_space(vec2(22.0, FLOATING_HEADER));
                        let label = format!("{} {title}", if collapsed { "Expand" } else { "Collapse" });
                        let response = solidcraft_ui_kit::panel_host_disclosure(ui, id, rect, &label, ui.is_enabled(), collapsed, true);
                        collapse = response.clicked();
                    });
                    if !collapsed && body_height > 0.0 {
                        ui.separator();
                        if body_height < toolbar_min_height {
                            egui::ScrollArea::vertical().max_height(body_height).min_scrolled_height(0.0).auto_shrink([false, !slot]).show(
                                ui,
                                |ui| {
                                    toolbar(ui);
                                    add(ui);
                                },
                            );
                        } else {
                            let start = ui.next_widget_position().y;
                            toolbar(ui);
                            let toolbar_height = (ui.next_widget_position().y - start).max(0.0);
                            egui::ScrollArea::vertical()
                                .max_height((body_height - toolbar_height).max(0.0))
                                .min_scrolled_height(0.0)
                                .auto_shrink([false, !slot])
                                .show(ui, add);
                        }
                    }
                    collapse
                });
                if slot && floating && !collapsed {
                    self.resize_edges(ui, id, title, frame.response.rect, canvas);
                }
                frame.inner
            });
        if self.drag.is_some()
            && let Some(pointer) = ui.ctx().input(|i| i.pointer.interact_pos())
        {
            let side = Self::snap(canvas, pointer);
            if side != Position::Floating {
                let target = Rect::from_min_size(
                    pos2(if side == Position::Left { bounds.left() } else { bounds.right() - width }, bounds.top()),
                    vec2(width, bounds.height()),
                );
                let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, id.with("dock_preview")));
                p.rect_filled(target, FLOATING_RADIUS, t.accent_soft.gamma_multiply(0.7));
                p.rect_stroke(target, FLOATING_RADIUS, Stroke::new(2.0, t.accent), StrokeKind::Inside);
            }
        }
        (shown.response.rect, shown.inner)
    }
}
