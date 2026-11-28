use super::{PcbApp, ViewId};
use crate::preferences::Navigation;
use egui::{Color32, Painter, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, pos2, vec2};
use pcbcraft_chrome::{Tokens, board, controls};
use pcbcraft_engine::{
    board::fabrication::review::{Finding, LayerRole, Mark},
    schematic::Point,
};

impl PcbApp {
    /// Render only inside this pane. The review never reads the editable board for geometry.
    pub(crate) fn manufacturing_pane(&mut self, ui: &mut Ui, id: ViewId) {
        let navigation = self.preferences.navigation;
        let Some(state) = self.views.view_mut(id).and_then(|view| view.manufacturing.as_mut()) else { return };
        let tokens = Tokens::get();
        let reading = self.manufacturing_files.reading(id, &state.binding);
        let Some(opened) = state.opened.as_ref() else {
            ui.painter().rect_filled(ui.max_rect(), 0, tokens.board_canvas);
            let label = if reading { "Opening fabrication ZIP…" } else { state.failure.as_deref().unwrap_or("Open a PCBcraft fabrication ZIP") };
            ui.centered_and_justified(|ui| {
                ui.add(egui::Label::new(RichText::new(label).color(tokens.board_ink)).wrap());
            });
            return;
        };
        let area = ui.available_rect_before_wrap();
        let compact = area.width() < 510.0;
        let (details, canvas) = if compact {
            let split = (area.height() * 0.34).min(180.0);
            let boundary = area.top() + split;
            (Rect::from_min_max(area.min, pos2(area.right(), boundary)), Rect::from_min_max(pos2(area.left(), boundary), area.max))
        } else {
            let boundary = area.left() + (area.width() * 0.30).clamp(200.0, 320.0);
            (Rect::from_min_max(area.min, pos2(boundary, area.bottom())), Rect::from_min_max(pos2(boundary, area.top()), area.max))
        };
        let focused = ui
            .scope_builder(egui::UiBuilder::new().id_salt("manufacturing_details").max_rect(details), |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(details));
                ui.painter().rect_filled(details, 0, tokens.panel);
                let mut focus = None;
                egui::Frame::new().inner_margin(tokens.space_8).show(ui, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(&opened.name).strong()).truncate()).on_hover_text(&opened.name);
                        ui.label(format!("PCBcraft {} · SHA-256 verified", opened.package.generator_version));
                        ui.label("Source revision unavailable");
                        if reading {
                            ui.label("Opening fabrication ZIP…");
                        }
                        if let Some(error) = &state.failure {
                            ui.add(egui::Label::new(RichText::new(error).color(tokens.error)).wrap());
                        }
                        ui.separator();
                        ui.label(RichText::new("Preflight").strong());
                        if opened.package.findings.is_empty() {
                            ui.label("Required files present. No open outline ends detected.");
                        }
                        for finding in &opened.package.findings {
                            if let Finding::OpenOutline { endpoints, first } = finding {
                                if controls::button(ui, format!("Open outline ({endpoints} ends)")).clicked() {
                                    focus = Some(*first);
                                }
                            } else {
                                ui.label(finding_label(finding));
                            }
                        }

                        ui.separator();
                        ui.label(RichText::new("Layers").strong());
                        for (index, (layer, visible)) in opened.package.layers.iter().zip(&mut state.visible).enumerate() {
                            ui.push_id(index, |ui| {
                                ui.horizontal(|ui| {
                                    board::layer_swatch(ui, layer_color(layer.role, tokens));
                                    let label = if layer.role == LayerRole::InnerCopper { layer.name.as_str() } else { layer.role.label() };
                                    controls::checkbox(ui, visible, label).on_hover_text(&layer.name);
                                });
                                if layer.role.is_drill() {
                                    if layer.tools.is_empty() {
                                        ui.label("No drill hits");
                                    }
                                    for tool in &layer.tools {
                                        ui.label(format!("T{} · {:.3} mm · {} hits", tool.number, tool.diameter_nm as f64 / 1_000_000.0, tool.hits));
                                    }
                                }
                            });
                        }
                    });
                });
                focus
            })
            .inner;
        if let Some(point) = focused {
            for (layer, visible) in opened.package.layers.iter().zip(&mut state.visible) {
                if layer.role == LayerRole::Profile {
                    *visible = true;
                }
            }
            state.camera.center_on(view_point(point));
            state.highlighted = Some(point);
        }
        ui.scope_builder(egui::UiBuilder::new().id_salt("manufacturing_canvas").max_rect(canvas), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(canvas));
            let (_, response) = ui.allocate_exact_size(canvas.size(), Sense::click_and_drag());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::GenericContainer, ui.is_enabled(), "Fabrication canvas"));
            paint(ui, response, state, navigation);
        });
    }
}

/// Use the same pan, zoom and Fit semantics as the definition editors, with Y-up output.
fn paint(ui: &Ui, response: egui::Response, state: &mut super::State, navigation: Navigation) {
    let Some(opened) = state.opened.as_ref() else { return };
    let camera = &mut state.camera;
    let visible = &state.visible;
    let rect = response.rect;
    let painter = ui.painter_at(rect);
    let tokens = Tokens::get();
    painter.rect_filled(rect, 0, tokens.board_canvas);
    let bounds = opened.package.bounds.into_iter().flatten().map(view_point);
    camera.navigate_definition(&response, rect, bounds, ui.is_enabled(), navigation);
    let screen = |at| camera.screen(view_point(at), rect);
    let points_per_nm = camera.unit_step() / pcbcraft_engine::schematic::GRID_NM as f32;
    for drill in [false, true] {
        for (layer, _) in opened.package.layers.iter().zip(visible).filter(|(layer, shown)| **shown && layer.role.is_drill() == drill) {
            let color = layer_color(layer.role, tokens);
            for mark in &layer.marks {
                let [a, b] = mark.bounds();
                if Rect::from_two_pos(screen(a), screen(b)).intersects(rect) {
                    paint_mark(&painter, mark, screen, points_per_nm, color, drill.then_some(tokens.board_canvas));
                }
            }
        }
    }
    if let Some(point) = state.highlighted {
        painter.circle_stroke(screen(point), 6.0, Stroke::new(2.0, tokens.board_highlight));
    }
}

/// Plot filled, physical aperture geometry. Drill hits reveal the hole and outline its diameter.
fn paint_mark(p: &Painter, mark: &Mark, screen: impl Fn(Point) -> Pos2, scale: f32, color: Color32, drill: Option<Color32>) {
    match mark {
        Mark::Stroke { ends, diameter_nm } => {
            let [a, b] = ends.map(screen);
            let diameter = *diameter_nm as f32 * scale;
            let radius = diameter / 2.0;
            if a == b {
                if let Some(fill) = drill {
                    let border = radius.min(1.0);
                    p.circle(a, radius - border / 2.0, fill, Stroke::new(border, color));
                } else {
                    p.circle_filled(a, radius, color);
                }
            } else {
                p.line_segment([a, b], Stroke::new(diameter, color));
                p.circle_filled(a, radius, color);
                p.circle_filled(b, radius, color);
            }
        }
        Mark::Strip { ends, width_nm } => {
            let [a, b] = ends.map(screen);
            let edge = (b - a).rot90().normalized() * (*width_nm as f32 * scale / 2.0);
            p.add(Shape::convex_polygon(vec![a + edge, a - edge, b - edge, b + edge], color, Stroke::NONE));
        }
        Mark::Rectangle { center, size_nm: [w, h] } => {
            p.rect_filled(Rect::from_center_size(screen(*center), vec2(*w as f32, *h as f32) * scale), 0, color);
        }
        Mark::Polygon(vertices) => {
            // An unfeathered mesh, as the board canvas draws pours: regions arrive split into
            // adjacent convex pieces, and feathered edges would show seams between them and
            // spikes at the sharp corners of thin pieces.
            let mut mesh = egui::Mesh::default();
            for &vertex in vertices.iter() {
                mesh.colored_vertex(screen(vertex), color);
            }
            for index in 2..mesh.vertices.len() as u32 {
                mesh.add_triangle(0, index - 1, index);
            }
            p.add(Shape::mesh(mesh));
        }
    }
}

fn view_point(point: Point) -> Point {
    Point { y_nm: -point.y_nm, ..point }
}

/// Keep the physical canvas colors legible in both chrome themes.
fn layer_color(role: LayerRole, tokens: Tokens) -> Color32 {
    match role {
        LayerRole::TopCopper => tokens.board_front,
        LayerRole::InnerCopper => tokens.board_inner,
        LayerRole::BottomCopper => tokens.board_back,
        LayerRole::TopMask => Color32::from_rgb(54, 158, 124),
        LayerRole::BottomMask => Color32::from_rgb(61, 148, 179),
        LayerRole::TopPaste => Color32::from_rgb(175, 183, 191),
        LayerRole::BottomPaste => Color32::from_rgb(157, 139, 178),
        LayerRole::TopSilkscreen => tokens.board_silkscreen,
        LayerRole::BottomSilkscreen => Color32::from_rgb(215, 169, 115),
        LayerRole::Profile => tokens.board_ink,
        LayerRole::PlatedDrill => Color32::from_rgb(255, 197, 102),
        LayerRole::NonPlatedDrill => Color32::from_rgb(133, 221, 255),
    }
}

/// Present one finding from the shared preflight report.
fn finding_label(finding: &Finding) -> &'static str {
    match finding {
        Finding::MissingTopCopper => "Top copper file is missing.",
        Finding::MissingBottomCopper => "Bottom copper file is missing.",
        Finding::MissingPlatedDrill => "Plated drill file is missing.",
        Finding::MissingOutline => "Board outline file is missing.",
        Finding::EmptyOutline => "Board outline file contains no edges.",
        Finding::OpenOutline { .. } => "Board outline has open ends.",
    }
}
