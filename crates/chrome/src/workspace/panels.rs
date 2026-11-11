//! Movable contextual cards share the workspace frame and Parts' edge-docking gestures.
use super::{FLOATING_GAP, FLOATING_HEADER, FLOATING_RADIUS, floating_card_min_height, floating_frame};
use crate::{MenuItem, Tokens, fonts, menu};
use egui::{Align2, Id, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};

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
}

impl PanelState {
    /// Reject non-finite or unbounded saved coordinates before restoring a layout.
    pub fn valid(&self) -> bool {
        self.at.is_none_or(|at| at.iter().all(|value| value.is_finite() && value.abs() <= 100_000.0))
    }
}

#[derive(Debug)]
struct Drag {
    pointer: Pos2,
    at: Pos2,
    before: PanelState,
}

/// Runtime card placement; cloning preserves layout but never transfers an active pointer drag.
#[derive(Debug, Default)]
pub struct Panel {
    state: PanelState,
    drag: Option<Drag>,
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
        Self::from_state(PanelState { position, at: None })
    }

    /// Restore placement after the caller validates its local arrangement record.
    pub fn from_state(state: PanelState) -> Self {
        Self { state, drag: None }
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
        let next = PanelState { position, at: if position == Position::Floating { at } else { None } };
        if !next.valid() {
            return false;
        }
        self.state = next;
        self.drag = None;
        true
    }

    /// Escape or focus loss restores the exact placement from before the header drag.
    pub fn cancel_drag(&mut self) -> bool {
        if let Some(drag) = self.drag.take() {
            self.state = drag.before;
            true
        } else {
            false
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
        if !ui.is_enabled() || ui.ctx().input(|i| i.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false)))) {
            self.cancel_drag();
        }
        let floating = self.state.position == Position::Floating;
        let area = if floating { canvas } else { dock };
        if area.height() < floating_card_min_height() || !area.is_positive() {
            return (Rect::NOTHING, false);
        }
        let width = width.min((canvas.width() - 2.0 * FLOATING_GAP).max(1.0));
        let height = if floating { 440.0_f32.min(area.height() - 2.0 * FLOATING_GAP) } else { area.height() - 2.0 * FLOATING_GAP };
        let body_height = (height - floating_frame().total_margin().sum().y - FLOATING_HEADER - Tokens::get().space_8).max(0.0);
        let bounds = canvas.shrink(FLOATING_GAP);
        let desired = if floating {
            self.state.at.map(|[x, y]| canvas.min + vec2(x, y)).unwrap_or(bounds.min + vec2(32.0, 32.0))
        } else {
            pos2(
                if self.state.position == Position::Left { area.left() + FLOATING_GAP } else { area.right() - width - FLOATING_GAP },
                area.top() + FLOATING_GAP,
            )
        };
        let at = pos2(
            desired.x.clamp(bounds.left(), (bounds.right() - width).max(bounds.left())),
            desired.y.clamp(bounds.top(), (bounds.bottom() - floating_card_min_height() + 2.0 * FLOATING_GAP).max(bounds.top())),
        );
        let body_height =
            body_height.min((bounds.bottom() - at.y - floating_frame().total_margin().sum().y - FLOATING_HEADER - Tokens::get().space_8).max(0.0));
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
                floating_frame()
                    .show(ui, |ui| {
                        ui.set_width(width - floating_frame().total_margin().sum().x);
                        ui.spacing_mut().item_spacing.y = t.space_2;
                        let mut collapse = false;
                        ui.horizontal(|ui| {
                            let header_width = (width - floating_frame().total_margin().sum().x).min(ui.available_width());
                            let remaining = (header_width - 44.0 - 2.0 * ui.spacing().item_spacing.x).max(1.0);
                            let (header, response) = ui.allocate_exact_size(vec2(remaining, FLOATING_HEADER), Sense::drag());
                            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), format!("Move {title}")));
                            super::drag_grip(ui.painter(), header);
                            let galley = super::elided(
                                ui.painter(),
                                title,
                                fonts::interface_heading(ui, t.font_body),
                                t.text,
                                (header.width() - 20.0).max(0.0),
                            );
                            ui.painter().galley(egui::pos2(header.left() + 18.0, header.center().y - galley.size().y / 2.0), galley, t.text);
                            self.drag_header(&response, Rect::from_min_size(at, vec2(width, height)), canvas);
                            response.on_hover_cursor(egui::CursorIcon::Grab);
                            let (more_rect, more) = ui.allocate_exact_size(vec2(22.0, FLOATING_HEADER), Sense::click());
                            if more.hovered() || more.has_focus() {
                                ui.painter().rect_filled(more_rect, t.radius_sm, t.hover);
                            }
                            ui.painter().text(more_rect.center(), Align2::CENTER_CENTER, "…", egui::FontId::proportional(t.font_button), t.text_dim);
                            more.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), format!("{title} position")));
                            egui::Popup::menu(&more).frame(menu::frame()).show(|ui| {
                                for (position, label) in
                                    [(Position::Floating, "Float"), (Position::Left, "Dock left"), (Position::Right, "Dock right")]
                                {
                                    if MenuItem::new(label).choice(self.state.position == position).show(ui) {
                                        let offset = at - canvas.min;
                                        self.set_position(position, Some([offset.x, offset.y]));
                                        ui.close();
                                        ui.ctx().request_repaint();
                                    }
                                }
                            });
                            let (rect, response) = ui.allocate_exact_size(vec2(22.0, FLOATING_HEADER), Sense::click());
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    ui.is_enabled(),
                                    format!("{} {title}", if collapsed { "Expand" } else { "Collapse" }),
                                )
                            });
                            if response.hovered() || response.has_focus() {
                                ui.painter().rect_filled(rect, t.radius_sm, t.hover);
                            }
                            let c = rect.center();
                            let sign = if collapsed { -1.0 } else { 1.0 };
                            ui.painter().add(Shape::line(
                                vec![c + vec2(-4.0, -2.0 * sign), c + vec2(0.0, 2.0 * sign), c + vec2(4.0, -2.0 * sign)],
                                Stroke::new(1.3, t.text_dim),
                            ));
                            collapse = response.clicked();
                        });
                        if !collapsed && body_height > 0.0 {
                            ui.separator();
                            if body_height < toolbar_min_height {
                                egui::ScrollArea::vertical().max_height(body_height).min_scrolled_height(0.0).auto_shrink([false, true]).show(
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
                                    .auto_shrink([false, true])
                                    .show(ui, add);
                            }
                        }
                        collapse
                    })
                    .inner
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
