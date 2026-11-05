//! Shared workspace surfaces and tool buttons. Command state stays with the caller.

use egui::{
    Align, Align2, Color32, FontId, Frame, Label, Layout, Margin, Panel, Pos2, Rect, Response, RichText, ScrollArea, Sense, Shape, Stroke, Ui,
    WidgetInfo, WidgetType, pos2, vec2,
};

use crate::Tokens;

/// The glyphs used by the first set of schematic ribbon actions.
#[derive(Clone, Copy)]
pub enum ToolIcon {
    Symbol,
    Wire,
    Nets,
    Fit,
}

/// Put a Capture strip above the viewport. At compact widths the workspace title moves into
/// the strip, and the actions can scroll horizontally when they exceed the available space.
pub fn ribbon(ui: &mut Ui, workspace: &str, group: &str, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get();
    let height = t.control_height * 3.0;
    Panel::top("workspace_ribbon")
        .exact_size(height + 2.0 * t.space_6)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::new().fill(t.canvas).inner_margin(Margin::symmetric(t.space_8 as i8, t.space_6 as i8)))
        .show(ui, |ui| {
            let wide = ui.available_width() >= 480.0;
            ui.horizontal_top(|ui| {
                if wide {
                    card(t, t.space_8).show(ui, |ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(t.control_height * 2.5, height - 2.0 * (t.space_8 + 1.0)), Sense::hover());
                        let center = rect.center();
                        let p = ui.painter();
                        p.hline(rect.x_range().shrink(t.space_4), rect.top(), Stroke::new(t.space_2, t.accent));
                        paint_icon(p, ToolIcon::Symbol, center - vec2(0.0, t.space_8), t.space_24, t.accent);
                        p.text(center + vec2(0.0, t.space_12), Align2::CENTER_CENTER, workspace, FontId::proportional(t.font_body), t.text);
                    });
                }

                card(t, t.space_4).show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        ui.set_min_height(height - 2.0 * (t.space_4 + 1.0));
                        let heading = if wide { group.to_owned() } else { format!("{workspace} / {group}") };
                        Frame::new()
                            .fill(t.accent_soft)
                            .corner_radius(t.radius_sm)
                            .inner_margin(Margin::symmetric(t.space_6 as i8, t.space_2 as i8))
                            .show(ui, |ui| {
                                ui.label(RichText::new(heading).size(t.font_small).color(t.text));
                            });
                        ui.separator();
                        ScrollArea::horizontal().id_salt("ribbon_actions").auto_shrink([false, true]).show(ui, |ui| {
                            ui.horizontal_top(add);
                        });
                    });
                });
            });
        });
}

/// Render one ribbon action with a theme-aware glyph and a keyboard-accessible button.
/// The longer description labels the control for assistive technology and serves as its tooltip.
pub fn tool(ui: &mut Ui, icon: ToolIcon, label: &str, description: &str, enabled: bool, active: bool) -> Response {
    let t = Tokens::get();
    ui.add_enabled_ui(enabled, |ui| {
        let (rect, response) = ui.allocate_exact_size(vec2(2.0 * t.control_height, t.control_height + t.space_12), Sense::click());
        response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, active, description));
        if ui.is_rect_visible(rect) {
            let p = ui.painter();
            if active || response.hovered() || response.has_focus() {
                let fill = if active { t.accent_soft } else { t.hover };
                p.rect_filled(rect, t.control_radius, fill);
            }
            let icon_color = if enabled { t.accent } else { t.text_dim };
            paint_icon(p, icon, pos2(rect.center().x, rect.top() + t.space_12), t.space_24, icon_color);
            p.text(
                pos2(rect.center().x, rect.bottom() - t.space_2),
                Align2::CENTER_BOTTOM,
                label,
                FontId::proportional(t.font_small),
                if enabled { t.text } else { t.text_dim },
            );
        }
        response.on_hover_text(description)
    })
    .inner
}

/// Draw the current sheet name in a band outside the canvas hit area.
pub fn sheet_header(ui: &mut Ui, title: &str) {
    let t = Tokens::get();
    Panel::top("workspace_sheet")
        .exact_size(t.control_height)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::new().fill(t.canvas).inner_margin(Margin::symmetric(t.space_12 as i8, 0)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add(Label::new(RichText::new(title).size(t.font_label)).truncate()).on_hover_text(title);
            });
        });
}

/// Dock a card at the right edge of the canvas and keep its body scrollable.
/// Reserve at most two fifths of the workspace width and return whether Close was clicked.
pub fn right_card(ui: &mut Ui, id: &'static str, title: &str, add: impl FnOnce(&mut Ui)) -> bool {
    let t = Tokens::get();
    let width = (ui.available_width() * 0.4).min(260.0);
    Panel::right(id)
        .exact_size(width)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::new().fill(t.canvas).inner_margin(Margin::same(t.space_8 as i8)))
        .show(ui, |ui| {
            card(t, t.space_8)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let header_size = vec2(ui.available_width(), t.control_height);
                    let close = ui
                        .allocate_ui_with_layout(header_size, Layout::right_to_left(Align::Center), |ui| {
                            let close = ui.small_button("Close").clicked();
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.add(Label::new(RichText::new(title).font(FontId::proportional(t.font_label))).truncate()).on_hover_text(title);
                            });
                            close
                        })
                        .inner;
                    ui.separator();
                    ScrollArea::vertical().auto_shrink([false, true]).show(ui, add);
                    close
                })
                .inner
        })
        .inner
}

/// Shared bordered surface for the ribbon and inspector.
fn card(t: Tokens, inset: f32) -> Frame {
    Frame::new().fill(t.panel).stroke(Stroke::new(1.0, t.border)).corner_radius(t.radius_md).inner_margin(Margin::same(inset as i8))
}

/// Draw scalable line art in the current theme's ink. Symbol and Wire use PCBcraft Next's glyph paths.
fn paint_icon(p: &egui::Painter, icon: ToolIcon, center: Pos2, size: f32, color: Color32) {
    let at = |x: f32, y: f32| center + vec2(x - 32.0, y - 32.0) * (size / 64.0);
    let stroke = Stroke::new(size / 16.0, color);
    let line = |points: &[(f32, f32)]| p.add(Shape::line(points.iter().map(|&(x, y)| at(x, y)).collect(), stroke));
    match icon {
        ToolIcon::Symbol => {
            line(&[(4.0, 32.0), (14.0, 32.0), (19.0, 21.0), (27.0, 43.0), (35.0, 21.0), (43.0, 43.0), (49.0, 32.0), (60.0, 32.0)]);
            line(&[(4.0, 26.0), (4.0, 38.0)]);
            line(&[(60.0, 26.0), (60.0, 38.0)]);
        }
        ToolIcon::Wire => {
            line(&[(10.0, 49.0), (28.0, 49.0), (28.0, 15.0), (54.0, 15.0)]);
            p.circle_filled(at(10.0, 49.0), size / 16.0, color);
            p.circle_filled(at(54.0, 15.0), size / 16.0, color);
        }
        ToolIcon::Nets => {
            for y in [17.0, 32.0, 47.0] {
                p.circle_filled(at(13.0, y), size / 16.0, color);
                p.line_segment([at(25.0, y), at(54.0, y)], stroke);
            }
        }
        ToolIcon::Fit => {
            for (x, y, dx, dy) in [(10.0, 10.0, 1.0, 1.0), (54.0, 10.0, -1.0, 1.0), (54.0, 54.0, -1.0, -1.0), (10.0, 54.0, 1.0, -1.0)] {
                line(&[(x + dx * 13.0, y), (x, y), (x, y + dy * 13.0)]);
            }
            p.rect_stroke(Rect::from_min_max(at(24.0, 24.0), at(40.0, 40.0)), 0, stroke, egui::StrokeKind::Inside);
        }
    }
}
