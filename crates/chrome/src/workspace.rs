//! Shared workspace surfaces and tool buttons. Command state stays with the caller.

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Label, Layout, Margin, Panel, Pos2, Rect, Response, RichText, ScrollArea, Sense, Shape,
    Stroke, Ui, WidgetInfo, WidgetType, pos2, vec2,
};

use crate::{Tokens, fonts};

const RIBBON_HEIGHT: f32 = 117.0;
const WORKSPACE_WIDTH: f32 = 88.0;
const RIBBON_TOOL_HEIGHT: f32 = 60.0;

/// The glyphs used by the first set of schematic ribbon actions.
#[derive(Clone, Copy)]
pub enum ToolIcon {
    Symbol,
    Wire,
    Nets,
    Fit,
}

/// The drawing on a workspace's tile in the picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceIcon {
    Schematic,
    Layout,
    Render,
    Simulate,
}

/// One row of the workspace picker. The app owns the list; the picker only draws it.
#[derive(Clone, Copy, Debug)]
pub struct Workspace<'a> {
    pub title: &'a str,
    pub description: &'a str,
    /// The ribbon tabs the workspace offers. `None` marks a planned workspace, which is drawn
    /// dimmed with a Planned badge and can't be picked.
    pub tabs: Option<&'a str>,
    pub icon: WorkspaceIcon,
    /// The workspace on screen, drawn with a Current badge.
    pub current: bool,
}

/// What the workspace tile offers: the workspaces in order and a note under them.
#[derive(Clone, Copy, Debug)]
pub struct Workspaces<'a> {
    pub entries: &'a [Workspace<'a>],
    pub footer: &'a str,
}

/// Put a Capture strip above the viewport. At compact widths the workspace title moves into
/// the strip, and the actions can scroll horizontally when they exceed the available space.
///
/// The workspace tile opens a picker listing `workspaces`. Returns the index of an available
/// workspace other than the current one when it is picked; picking the current one just closes
/// the picker.
pub fn ribbon(ui: &mut Ui, workspace: &str, group: &str, workspaces: Workspaces<'_>, add: impl FnOnce(&mut Ui)) -> Option<usize> {
    let t = Tokens::get();
    let height = RIBBON_HEIGHT;
    // Let the app bar color show behind the two rounded workspace corners.
    let top = ui.available_rect_before_wrap();
    ui.painter().rect_filled(Rect::from_min_size(top.min, vec2(top.width(), f32::from(t.radius_lg))), 0, t.app_bar);
    Panel::top("workspace_ribbon")
        .exact_size(height + 2.0 * t.space_6)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(t.canvas)
                .corner_radius(CornerRadius { nw: t.radius_lg, ne: t.radius_lg, sw: 0, se: 0 })
                .inner_margin(Margin::symmetric(t.space_8 as i8, t.space_6 as i8)),
        )
        .show(ui, |ui| {
            let wide = ui.available_width() >= 480.0;
            let mut picked = None;
            ui.horizontal_top(|ui| {
                if wide {
                    let tile = card(t, t.space_8).show(ui, |ui| workspace_tile(ui, workspace, height)).inner;
                    picked = picker(ui, &tile, workspaces);
                }

                card(t, t.space_4).show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        ui.set_min_height(height - 2.0 * (t.space_4 + 1.0));
                        ui.spacing_mut().item_spacing.y = 1.0;
                        let heading = if wide { group.to_owned() } else { format!("{workspace} / {group}") };
                        Frame::new()
                            .fill(t.accent_soft)
                            .corner_radius(t.radius_sm)
                            .inner_margin(Margin::symmetric(t.space_12 as i8, t.space_3 as i8))
                            .show(ui, |ui| {
                                ui.label(RichText::new(heading).font(fonts::interface_heading(ui, t.font_button)).color(t.ribbon_tab_text));
                            });
                        ui.separator();
                        ScrollArea::horizontal().id_salt("ribbon_actions").auto_shrink([false, true]).show(ui, |ui| {
                            ui.horizontal_top(add);
                        });
                    });
                });
            });
            picked
        })
        .inner
}

/// Draw the workspace tile: the accent rule, icon, workspace name and a "Workspace" caption with
/// a caret that points up while the picker is open. The tile is a button that toggles the picker.
fn workspace_tile(ui: &mut Ui, workspace: &str, height: f32) -> Response {
    let t = Tokens::get();
    let inset = t.space_8 + 1.0;
    let (rect, response) = ui.allocate_exact_size(vec2(WORKSPACE_WIDTH - 2.0 * inset, height - 2.0 * inset), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Choose workspace"));
    let open = egui::Popup::is_id_open(ui.ctx(), picker_id());
    let center = rect.center();
    let p = ui.painter();
    if open || response.hovered() || response.has_focus() {
        p.rect_filled(rect.expand(inset - 1.0), t.radius_md, t.hover);
    }
    p.hline(rect.x_range(), rect.top() - t.space_6, Stroke::new(t.space_2, t.accent));
    paint_workspace_glyph(p, WorkspaceIcon::Schematic, center - vec2(0.0, t.space_16), t.control_height + t.space_2, t, true);
    p.text(center + vec2(0.0, t.space_12), Align2::CENTER_CENTER, workspace, fonts::interface_heading(ui, t.font_button), t.text);
    let caption = p.layout_no_wrap("Workspace".to_owned(), FontId::proportional(t.font_small), t.text_dim);
    let caret = t.space_4 + t.space_3;
    let caption_center = center + vec2(-caret / 2.0, t.space_24 + t.space_8);
    let tip = caption_center + vec2(caption.size().x / 2.0 + caret, 0.0);
    p.galley(caption_center - caption.size() / 2.0, caption, t.text_dim);
    let direction = if open { -1.0 } else { 1.0 };
    let arm = t.space_3;
    p.add(Shape::line(
        vec![tip + vec2(-arm, -arm / 2.0 * direction), tip + vec2(0.0, arm / 2.0 * direction), tip + vec2(arm, -arm / 2.0 * direction)],
        Stroke::new(1.2, t.text_dim),
    ));
    response.on_hover_text("Choose workspace")
}

fn picker_id() -> egui::Id {
    egui::Id::new("workspace_picker")
}

/// Show the workspace picker under the tile while it's open. Clicking an available row, or
/// Enter or Space for the current one, closes it; a planned row never responds. Returns the
/// index of an available workspace other than the current one when it is picked.
fn picker(ui: &Ui, tile: &Response, workspaces: Workspaces<'_>) -> Option<usize> {
    let t = Tokens::get();
    // SolidCraft's picker width, kept inside narrow windows.
    let width =
        (t.form_card_width + t.form_label_width * 2.0 + t.space_24 + t.space_16 + t.space_6).min(ui.ctx().content_rect().width() - t.space_24);
    let height = (ui.ctx().content_rect().bottom() - tile.rect.bottom() - t.space_4).max(160.0);
    egui::Popup::from_toggle_button_response(tile)
        .id(picker_id())
        .kind(egui::PopupKind::Menu)
        .align(egui::RectAlign::BOTTOM_START)
        .align_alternatives(&[])
        .gap(t.space_4)
        .width(width)
        .layout(Layout::top_down(Align::Min))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .frame(crate::menu::frame().inner_margin(Margin::same((t.space_8 + 1.0) as i8)))
        .show(|ui| picker_rows(ui, workspaces, width, height))
        .and_then(|shown| shown.inner)
}

/// The picker's heading, rows and footer.
fn picker_rows(ui: &mut Ui, workspaces: Workspaces<'_>, width: f32, height: f32) -> Option<usize> {
    let t = Tokens::get();
    ui.set_width(width);
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.add_space(t.space_4);
    ui.label(RichText::new("WORKSPACE").font(fonts::interface_heading(ui, t.font_small)).color(t.text_dim));
    ui.add_space(t.space_8);
    // Keys stay in the picker so they don't reach the canvas underneath.
    let (navigate, activate) = ui.input_mut(|i| {
        let navigate = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
        let activate = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) || i.consume_key(egui::Modifiers::NONE, egui::Key::Space);
        (navigate, activate)
    });
    let mut picked = None;
    ScrollArea::vertical().id_salt("workspace_rows").max_height((height - 116.0).max(60.0)).show(ui, |ui| {
        for (index, entry) in workspaces.entries.iter().enumerate() {
            let response = workspace_row(ui, entry, width);
            if entry.current && navigate {
                response.request_focus();
            }
            let chosen = response.clicked() || (activate && (response.has_focus() || entry.current));
            if entry.tabs.is_some() && chosen {
                if !entry.current {
                    picked = Some(index);
                }
                ui.close();
            }
        }
    });
    ui.add_space(t.space_6);
    ui.separator();
    ui.add_space(t.space_6);
    ui.add(Label::new(RichText::new(workspaces.footer).size(t.font_small).color(t.text_dim)).wrap());
    ui.add_space(t.space_4);
    picked
}

/// Draw one workspace row, as SolidCraft's picker does: an accent bar, the icon tile, the title
/// with its Current or Planned badge, the description, and the tabs for available workspaces.
/// Planned rows are dimmed, only respond to hover, and report themselves disabled.
fn workspace_row(ui: &mut Ui, entry: &Workspace<'_>, width: f32) -> Response {
    let t = Tokens::get();
    let available = entry.tabs.is_some();
    let accent = if available { t.accent } else { t.text_dim };
    let soft = t.panel.lerp_to_gamma(accent, 0.10);
    let row_height = if available { t.space_24 * 3.0 } else { t.space_24 * 2.0 + t.space_12 };
    let (rect, response) = ui.allocate_exact_size(vec2(width, row_height), if available { Sense::click() } else { Sense::hover() });
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, available, entry.current, entry.title));
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    if entry.current || (available && (response.hovered() || response.has_focus())) {
        painter.rect_filled(rect, t.control_radius, soft);
    }
    if available && (response.hovered() || response.has_focus()) {
        painter.rect_stroke(rect.shrink(0.5), t.control_radius, Stroke::new(1.0, accent), egui::StrokeKind::Inside);
    }
    let bar = Rect::from_min_max(rect.min + vec2(t.space_2, t.space_6 + 1.0), pos2(rect.left() + t.space_6, rect.bottom() - t.space_6 - 1.0));
    painter.rect_filled(bar, t.radius_sm / 2, accent);
    let side = t.space_24 + t.space_16 + t.space_4;
    let tile = Rect::from_center_size(pos2(rect.left() + 40.0, rect.center().y), vec2(side, side));
    painter.rect_filled(tile, t.radius_md, if available { soft } else { t.panel_header });
    paint_workspace_glyph(&painter, entry.icon, tile.center(), side - 2.0 * (t.space_6 + 1.0), t, available);

    let ink = if available { t.text } else { t.text_dim };
    let x = rect.left() + 74.0;
    let y = rect.top() + 17.0;
    let title = painter.layout_no_wrap(entry.title.to_owned(), fonts::interface_heading(ui, t.font_heading), ink);
    let badge_x = x + title.size().x + t.space_8 + 1.0;
    painter.galley(pos2(x, y - title.size().y / 2.0), title, ink);
    let (badge_fill, badge_ink) = if entry.current { (t.badge, t.badge_ink) } else { (soft, accent) };
    let label = if entry.current {
        "Current"
    } else if available {
        "Available"
    } else {
        "Planned"
    };
    let badge = painter.layout_no_wrap(label.to_owned(), FontId::proportional(t.font_small), badge_ink);
    let badge_rect = Rect::from_min_size(pos2(badge_x, y - t.space_8), badge.size() + vec2(t.space_12, t.space_3));
    painter.rect_filled(badge_rect, t.radius_lg, badge_fill);
    painter.galley(badge_rect.min + vec2(t.space_6, t.space_3 / 2.0), badge, badge_ink);
    let line = |text: &str, y: f32| {
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), FontId::proportional(t.font_body), ink);
        job.wrap.max_width = (rect.right() - x - t.space_6).max(0.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = false;
        painter.galley(pos2(x, y), painter.layout_job(job), ink);
    };
    line(entry.description, rect.top() + 29.0);
    if let Some(tabs) = entry.tabs {
        line(tabs, rect.top() + 47.0);
    }
    if available && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// Lay out related tools together with a quiet group label below their buttons.
pub fn ribbon_group(ui: &mut Ui, title: &str, tool_count: u8, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get();
    let count = f32::from(tool_count);
    let width = count * (2.0 * t.control_height) + (count - 1.0) * ui.spacing().item_spacing.x;
    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.y = t.space_2;
        ui.horizontal_top(add);
        ui.label(RichText::new(title).size(t.font_small).color(t.text_dim));
    });
}

/// Render one ribbon action with a theme-aware glyph and a keyboard-accessible button.
/// The longer description labels the control for assistive technology and serves as its tooltip.
pub fn tool(ui: &mut Ui, icon: ToolIcon, label: &str, description: &str, enabled: bool, active: bool) -> Response {
    let t = Tokens::get();
    ui.add_enabled_ui(enabled, |ui| {
        let (rect, response) = ui.allocate_exact_size(vec2(2.0 * t.control_height, RIBBON_TOOL_HEIGHT), Sense::click());
        response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, active, description));
        if ui.is_rect_visible(rect) {
            let p = ui.painter();
            if active || response.hovered() || response.has_focus() {
                let fill = if active { t.accent_soft } else { t.hover };
                p.rect_filled(rect, t.control_radius, fill);
            }
            let icon_color = if enabled { t.accent } else { t.text_dim };
            paint_icon(p, icon, pos2(rect.center().x, rect.top() + t.space_16), t.space_24, icon_color);
            p.text(
                pos2(rect.center().x, rect.bottom() - t.space_8),
                Align2::CENTER_BOTTOM,
                label,
                FontId::proportional(t.font_body),
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
/// Reserve at most two fifths of the workspace width and leave room below the
/// card for a floating launcher. Return whether Close was clicked.
pub fn right_card(ui: &mut Ui, id: &'static str, title: &str, bottom_clearance: f32, add: impl FnOnce(&mut Ui)) -> bool {
    let t = Tokens::get();
    let width = (ui.available_width() * 0.4).min(260.0);
    let clearance = bottom_clearance.min(ui.available_height() * 0.4);
    let margin = Margin {
        right: if bottom_clearance > 0.0 { t.space_12 } else { t.space_8 } as i8,
        bottom: (t.space_8 + clearance) as i8,
        ..Margin::same(t.space_8 as i8)
    };
    Panel::right(id)
        .exact_size(width)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::new().fill(t.canvas).inner_margin(margin))
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

/// Draw a workspace's icon, `size` points square, in the theme's icon colour roles. Planned
/// workspaces (`available` false) keep their colours at about a third of their strength, as
/// PCBcraft Next fades them. Geometry follows the 64-unit artwork: SolidCraft's schematic
/// sheet and render sphere, and PCBcraft Next's original board and simulation glyphs.
fn paint_workspace_glyph(p: &egui::Painter, icon: WorkspaceIcon, center: Pos2, size: f32, t: Tokens, available: bool) {
    let fade = |color: Color32| if available { color } else { color.gamma_multiply(0.35) };
    let [ink, face_1, face_2, face_3, active_1, active_2, active_3] =
        [t.icon_ink, t.icon_face_1, t.icon_face_2, t.icon_face_3, t.icon_active_1, t.icon_active_2, t.icon_active_3].map(fade);
    let scale = size / 64.0;
    let at = |x: f32, y: f32| center + vec2(x - 32.0, y - 32.0) * scale;
    let rect = |x1, y1, x2, y2| Rect::from_min_max(at(x1, y1), at(x2, y2));
    let stroke = |width: f32, color| Stroke::new(width * scale, color);
    let line = |points: &[(f32, f32)], stroke: Stroke| p.add(Shape::line(points.iter().map(|&(x, y)| at(x, y)).collect(), stroke));
    match icon {
        WorkspaceIcon::Schematic => {
            let outline = stroke(2.55, ink);
            let accent = stroke(3.0, active_2);
            p.rect(rect(10.0, 6.0, 54.0, 58.0), 2, face_1, outline, egui::StrokeKind::Middle);
            p.rect(rect(17.0, 14.0, 36.0, 30.0), 0, face_2, outline, egui::StrokeKind::Middle);
            p.rect(rect(33.0, 46.0, 48.0, 53.0), 1, active_1, stroke(2.2, ink), egui::StrokeKind::Middle);
            line(&[(17.0, 38.0), (36.0, 38.0)], accent);
            line(&[(17.0, 35.0), (17.0, 41.0)], accent);
            line(&[(36.0, 35.0), (36.0, 41.0)], accent);
            line(&[(44.0, 14.0), (44.0, 30.0)], accent);
        }
        WorkspaceIcon::Layout => {
            p.rect(rect(8.0, 9.0, 56.0, 55.0), 3, active_3, stroke(2.5, ink), egui::StrokeKind::Middle);
            let trace = stroke(2.5, active_1);
            line(&[(16.0, 23.0), (27.0, 23.0), (27.0, 39.0), (48.0, 39.0)], trace);
            line(&[(16.0, 44.0), (23.0, 44.0), (23.0, 17.0), (48.0, 17.0)], trace);
            p.rect(rect(31.0, 24.0, 44.0, 37.0), 1, ink, stroke(2.0, face_3), egui::StrokeKind::Middle);
            for (x, y) in [(16.0, 23.0), (48.0, 39.0), (16.0, 44.0), (48.0, 17.0)] {
                p.circle(at(x, y), 3.0 * scale, face_1, stroke(1.5, ink));
            }
        }
        WorkspaceIcon::Render => {
            // Shaded faces stacked toward the light, inside the sphere's outline.
            p.circle_filled(at(28.0, 38.0), 21.0 * scale, face_3);
            p.circle_filled(at(25.5, 35.0), 17.5 * scale, face_2);
            p.circle_filled(at(22.5, 31.5), 12.0 * scale, face_1);
            p.circle_filled(at(19.0, 27.0), 4.0 * scale, active_2);
            p.circle_stroke(at(28.0, 38.0), 21.0 * scale, stroke(2.55, ink));
            let spark = [(50.0, 5.0), (52.8, 11.2), (59.0, 14.0), (52.8, 16.8), (50.0, 23.0), (47.2, 16.8), (41.0, 14.0), (47.2, 11.2)];
            for pair in spark.windows(2).chain(std::iter::once(&[spark[7], spark[0]][..])) {
                if let [(x1, y1), (x2, y2)] = pair {
                    p.add(Shape::convex_polygon(vec![at(50.0, 14.0), at(*x1, *y1), at(*x2, *y2)], active_3, Stroke::NONE));
                }
            }
        }
        WorkspaceIcon::Simulate => {
            line(&[(9.0, 9.0), (9.0, 55.0), (57.0, 55.0)], stroke(3.0, face_3));
            // Two cubic arcs through the axis line, then a short rise.
            let cubic = |a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)| {
                (0..=12).map(move |k| {
                    let u = k as f32 / 12.0;
                    let v = 1.0 - u;
                    let mix = |a: f32, b: f32, c: f32, d: f32| v * v * v * a + 3.0 * v * v * u * b + 3.0 * v * u * u * c + u * u * u * d;
                    (mix(a.0, b.0, c.0, d.0), mix(a.1, b.1, c.1, d.1))
                })
            };
            let mut wave: Vec<(f32, f32)> = cubic((12.0, 34.0), (18.0, 6.0), (24.0, 6.0), (30.0, 34.0)).collect();
            wave.extend(cubic((30.0, 34.0), (36.0, 62.0), (42.0, 62.0), (48.0, 34.0)).skip(1));
            wave.push((54.0, 16.0));
            line(&wave, stroke(4.0, active_3));
        }
    }
}

/// Draw scalable line art in the current theme's ink. Symbol and Wire use PCBcraft Next's glyph paths.
fn paint_icon(p: &egui::Painter, icon: ToolIcon, center: Pos2, size: f32, color: Color32) {
    let at = |x: f32, y: f32| center + vec2(x - 32.0, y - 32.0) * (size / 64.0);
    let stroke = Stroke::new(size / 16.0, color);
    let line = |points: &[(f32, f32)]| p.add(Shape::line(points.iter().map(|&(x, y)| at(x, y)).collect(), stroke));
    match icon {
        ToolIcon::Symbol => {
            line(&[(21.0, 8.0), (21.0, 56.0), (55.0, 32.0), (21.0, 8.0)]);
            line(&[(7.0, 22.0), (21.0, 22.0)]);
            line(&[(7.0, 42.0), (21.0, 42.0)]);
            line(&[(55.0, 32.0), (62.0, 32.0)]);
            line(&[(26.0, 22.0), (34.0, 22.0)]);
            line(&[(30.0, 18.0), (30.0, 26.0)]);
            line(&[(26.0, 38.0), (34.0, 38.0)]);
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
