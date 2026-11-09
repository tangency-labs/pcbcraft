//! Board inspection primitives; physical geometry and selection ownership stay with callers.
use crate::Tokens;
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, vec2};

/// Paint a layer graphic in its effective color; selection, previews and handles use the shared accent.
pub fn drawing(p: &Painter, points: Vec<Pos2>, closed: bool, selected: bool, preview: bool, color: Color32) {
    let t = Tokens::get();
    let color = if selected || preview { t.accent } else { color };
    p.add(egui::Shape::Path(egui::epaint::PathShape {
        points: points.clone(),
        closed,
        fill: Color32::TRANSPARENT,
        stroke: Stroke::new(1.5, color).into(),
    }));
    if selected {
        for point in points {
            p.rect_filled(Rect::from_center_size(point, vec2(7., 7.)), 0, t.accent);
        }
    }
}

/// Distinguish the snapped drawing target from authored vertices without changing its position.
pub fn drawing_target(p: &Painter, point: Pos2) {
    p.circle_stroke(point, 5., Stroke::new(1., Tokens::get().accent));
}

/// Return the shared palette's copper colors; `None` denotes an internal layer.
pub fn copper_color(front: Option<bool>) -> Color32 {
    let t = Tokens::get();
    match front {
        Some(true) => t.board_front,
        Some(false) => t.board_back,
        None => t.board_inner,
    }
}

/// Screen-space hit area for a placement's pad-center markers, with screen-space padding for reliable pointer selection.
/// These sizes are UI affordances, not pad dimensions or a manufactured footprint outline.
pub fn marker_bounds(pads: &[Pos2], center: Pos2) -> Rect {
    let bounds = pads.iter().fold(Rect::from_pos(center), |bounds, point| bounds.union(Rect::from_pos(*point)));
    bounds.expand2(if bounds.height() > bounds.width() { vec2(18.0, 12.0) } else { vec2(12.0, 18.0) })
}

/// A projected copper pad or an explicitly shape-less legacy marker.
#[derive(Clone, Debug)]
pub struct PadDrawing {
    pub number: String,
    pub center: Pos2,
    pub copper: Option<[Pos2; 4]>,
    /// Resolved display color for this pad's actual visible copper layer; None for legacy markers.
    pub color: Option<Color32>,
    pub contour: PadContour,
    pub drill_radius: Option<f32>,
    pub through_hole: bool,
    pub net: Option<String>,
}

/// Physical land contour, with screen-projected radius independent of UI corner tokens.
#[derive(Clone, Copy, Debug)]
pub enum PadContour {
    Rectangle,
    RoundedRectangle { radius: f32 },
    Circle,
    Oval,
}

/// Tessellate a physical contour in its own rotated axes without inflating its copper bounds.
pub fn pad_contour(pad: &PadDrawing) -> Vec<Pos2> {
    let Some(corners) = pad.copper else { return Vec::new() };
    let u = corners[1] - corners[0];
    let v = corners[3] - corners[0];
    let width = u.length();
    let height = v.length();
    if width <= f32::EPSILON || height <= f32::EPSILON {
        return corners.to_vec();
    }
    let at = |x: f32, y: f32| corners[0] + u * (x / width) + v * (y / height);
    match pad.contour {
        PadContour::Rectangle => corners.to_vec(),
        PadContour::Circle => (0..64)
            .map(|i| {
                let angle = i as f32 * std::f32::consts::TAU / 64.;
                at(width * (1. + angle.cos()) / 2., height * (1. + angle.sin()) / 2.)
            })
            .collect(),
        PadContour::RoundedRectangle { .. } | PadContour::Oval => {
            let radius = match pad.contour {
                PadContour::RoundedRectangle { radius } => radius.min(width.min(height) / 2.),
                _ => width.min(height) / 2.,
            };
            [(width - radius, radius), (width - radius, height - radius), (radius, height - radius), (radius, radius)]
                .into_iter()
                .enumerate()
                .flat_map(|(corner, (x, y))| {
                    (0..=8).map(move |step| {
                        let angle = (corner as f32 - 1. + step as f32 / 8.) * std::f32::consts::FRAC_PI_2;
                        at(x + radius * angle.cos(), y + radius * angle.sin())
                    })
                })
                .collect()
        }
    }
}

/// Size and orient pad labels within the land. Snapshot-local net labels appear only when both
/// lines fit; zooming out removes text before it obscures copper or protrudes into a gap.
/// Bright copper uses dark labels so personal color choices do not erase pad annotations.
fn pad_label(p: &Painter, pad: &PadDrawing, corners: [Pos2; 4], copper: Color32) {
    let mut axis = corners[1] - corners[0];
    let mut width = axis.length();
    let mut height = (corners[3] - corners[0]).length();
    if height > width * 1.3 {
        std::mem::swap(&mut width, &mut height);
        axis = corners[3] - corners[0];
    }
    if width < 12. || height < 12. {
        return;
    }
    let size_for = |text: &str, lines: f32| {
        ((width - 5.) / (text.lines().map(str::len).max().unwrap_or(1) as f32 * 0.65)).min((height - 3.) / (1.2 * lines)).min(28.)
    };
    let both = pad.net.as_ref().map(|net| format!("{}\n{net}", pad.number));
    let (text, size) = match both {
        Some(text) if size_for(&text, 2.) >= 7. => {
            let size = size_for(&text, 2.);
            (text, size)
        }
        _ => (pad.number.clone(), size_for(&pad.number, 1.)),
    };
    if size < 7. {
        return;
    }
    let mut angle = axis.y.atan2(axis.x);
    if angle > std::f32::consts::FRAC_PI_2 {
        angle -= std::f32::consts::PI;
    }
    if angle < -std::f32::consts::FRAC_PI_2 {
        angle += std::f32::consts::PI;
    }
    let [r, g, b, _] = copper.to_normalized_gamma_f32();
    let label_ink = if 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.65 { Tokens::get().board_canvas } else { Color32::WHITE };
    let mut job = egui::text::LayoutJob::simple(text, FontId::monospace(size), label_ink, f32::INFINITY);
    job.halign = egui::Align::Center;
    let galley = p.layout_job(job);
    let pos = pad.center - galley.rect.center().to_vec2();
    p.add(egui::epaint::TextShape::new(pos, galley, label_ink).with_angle_and_anchor(angle, Align2::CENTER_CENTER));
}

/// The physical purpose of a projected technical rectangle, independent of copper color.
#[derive(Clone, Copy, Debug)]
pub enum GraphicStyle {
    Fabrication,
    Silkscreen,
    Courtyard,
}

/// Shared technical-layer defaults used by both the layer legend and the physical painter.
pub fn graphic_color(style: GraphicStyle) -> Color32 {
    let t = Tokens::get();
    let color = match style {
        GraphicStyle::Fabrication => t.board_ink.gamma_multiply(0.55),
        GraphicStyle::Silkscreen => t.board_silkscreen,
        GraphicStyle::Courtyard => t.board_ink.gamma_multiply(0.3),
    };
    Color32::from_rgb(color.r(), color.g(), color.b())
}

/// A projected, unfilled rectangle with a physical layer role.
#[derive(Clone, Debug)]
pub struct GraphicDrawing {
    pub points: Vec<Pos2>,
    pub style: GraphicStyle,
    /// Effective visible technical-layer color; None keeps the shared library-preview default.
    pub color: Option<Color32>,
    pub closed: bool,
    pub stroke_width: Option<f32>,
}

/// Bounds include real pad extents and visible technical graphics. Legacy markers retain their
/// generous selection affordance; physical dimensions are never inflated to match that affordance.
pub fn footprint_bounds(pads: &[PadDrawing], graphics: &[GraphicDrawing], center: Pos2) -> Rect {
    if pads.iter().all(|p| p.copper.is_none()) && graphics.is_empty() {
        return marker_bounds(&pads.iter().map(|p| p.center).collect::<Vec<_>>(), center);
    }
    let mut bounds = Rect::from_pos(center);
    for pad in pads {
        bounds = bounds.union(Rect::from_pos(pad.center));
        if let Some(points) = pad.copper {
            for point in points {
                bounds = bounds.union(Rect::from_pos(point));
            }
        }
    }
    for graphic in graphics {
        let extent = graphic.points.iter().fold(Rect::NOTHING, |rect, point| rect.union(Rect::from_pos(*point)));
        bounds = bounds.union(extent.expand(graphic.stroke_width.unwrap_or(0.) / 2.));
    }
    bounds.expand(4.0)
}

/// Paint physical land contours and plated drills at their projected dimensions, with separate
/// technical graphics and selection ink. Only legacy definitions use center markers. Small
/// lands omit labels before they become illegible; physical dimensions never depend on zoom.
#[allow(clippy::too_many_arguments)]
pub fn footprint(
    p: &Painter,
    pads: &[PadDrawing],
    graphics: &[GraphicDrawing],
    reference: &str,
    center: Pos2,
    copper: Color32,
    selected: bool,
    highlighted: bool,
) {
    let t = Tokens::get();
    let ink = if selected {
        t.accent
    } else if highlighted {
        t.board_highlight
    } else {
        copper
    };
    let bounds = footprint_bounds(pads, graphics, center);
    for graphic in graphics {
        let width = match graphic.style {
            GraphicStyle::Fabrication => 0.8,
            GraphicStyle::Silkscreen => 1.2,
            GraphicStyle::Courtyard => 0.7,
        };
        let color = graphic.color.unwrap_or_else(|| graphic_color(graphic.style));
        let stroke = Stroke::new(graphic.stroke_width.unwrap_or(width), color);
        p.add(if graphic.closed {
            egui::Shape::closed_line(graphic.points.clone(), stroke)
        } else {
            egui::Shape::line(graphic.points.clone(), stroke)
        });
        if let Some(width) = graphic.stroke_width {
            for point in &graphic.points {
                p.circle_filled(*point, width / 2., color);
            }
        }
    }
    if selected || highlighted {
        p.rect_stroke(bounds, t.radius_sm, Stroke::new(1.5, ink), egui::StrokeKind::Inside);
    }
    let extent = pads.iter().fold(Rect::from_pos(center), |r, p| r.union(Rect::from_pos(p.center)));
    let (offset, align) = if extent.height() > extent.width() { (vec2(9., 0.), Align2::LEFT_CENTER) } else { (vec2(0., 9.), Align2::CENTER_TOP) };
    for pad in pads {
        if let Some(points) = pad.copper {
            let color = if selected || highlighted { ink } else { pad.color.unwrap_or(copper) };
            p.add(egui::Shape::convex_polygon(pad_contour(pad), color, Stroke::NONE));
            if let Some(radius) = pad.drill_radius {
                p.circle_filled(pad.center, radius, t.board_canvas);
                p.circle_stroke(pad.center, radius, Stroke::new(0.7, t.board_ink.gamma_multiply(0.5)));
            }
            pad_label(p, pad, points, color);
        } else {
            p.circle_stroke(pad.center, 5., Stroke::new(1.5, ink));
            p.line_segment([pad.center - vec2(2., 0.), pad.center + vec2(2., 0.)], Stroke::new(1., ink));
            p.line_segment([pad.center - vec2(0., 2.), pad.center + vec2(0., 2.)], Stroke::new(1., ink));
            p.text(pad.center + offset, align, &pad.number, FontId::proportional(t.font_small), t.board_ink);
        }
    }
    p.text(
        bounds.center_top() - vec2(0., t.space_2),
        Align2::CENTER_BOTTOM,
        reference,
        FontId::proportional(t.font_body),
        if selected || highlighted { ink } else { t.board_ink },
    );
}

/// Explain an empty board once in the canvas; normal populated views need no instructional copy.
pub fn empty(p: &Painter, rect: Rect) {
    let t = Tokens::get();
    p.text(rect.center(), Align2::CENTER_CENTER, "No footprints placed", FontId::proportional(t.font_body), t.board_ink);
}

/// Intent from the shared layer-color form; the host validates and persists appearance separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerColorAction {
    None,
    /// Let the host validate the typed hex and synchronize only the disposable preview.
    HexChanged,
    Apply,
    Reset,
    Cancel,
}

/// Use the existing modal surface and egui color picker for an opaque copper color draft.
/// Hex edits stay pending until Apply; Cancel never changes any layer appearance.
pub fn layer_color_dialog(ctx: &egui::Context, name: &str, rgb: &mut [u8; 3], hex: &mut String, error: Option<&str>) -> LayerColorAction {
    let mut action = LayerColorAction::None;
    let closing = crate::shell::scrolling_dialog(ctx, "layer_color", "Layer Color", |ui| {
        ui.label(name);
        let mut color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
        let changed = ui
            .scope(|ui| {
                let t = Tokens::get();
                ui.spacing_mut().slider_width =
                    ui.available_width().min((ctx.content_rect().height() - t.control_height * 10.0 - t.space_8).max(t.control_height * 4.0));
                egui::color_picker::color_picker_color32(ui, &mut color, egui::color_picker::Alpha::Opaque)
            })
            .inner;
        if changed {
            let [r, g, b, _] = color.to_array();
            *rgb = [r, g, b];
            *hex = format!("#{r:02X}{g:02X}{b:02X}");
        }
        if crate::properties::text(ui, "Hex", hex).changed() {
            action = LayerColorAction::HexChanged;
        }
        if let Some(error) = error {
            ui.label(crate::shell::error(error));
        }
        ui.horizontal(|ui| {
            if ui.button("Apply color").clicked() {
                action = LayerColorAction::Apply;
            }
            if ui.button("Reset to default").clicked() {
                action = LayerColorAction::Reset;
            }
            if ui.button("Cancel").clicked() {
                action = LayerColorAction::Cancel;
            }
        });
    });
    if closing { LayerColorAction::Cancel } else { action }
}

/// FigCraft's layer visibility control: an always-visible eye outline, with a pupil when
/// shown or a slash when hidden. The quiet button shares the app's palette and focus styling.
/// Its toggle semantics remain available to keyboard and assistive-technology users.
/// Returns a click intent; the caller changes visibility through its shared state handler.
pub fn layer_visibility(ui: &mut egui::Ui, name: &str, visible: bool) -> egui::Response {
    visibility(ui, &format!("layer {name}"), visible)
}

/// Shared eye toggle for authored field visibility and canvas layers, with explicit accessibility.
/// `name` identifies the object, such as "layer F.Cu" or "Reference", without an action prefix.
pub fn visibility(ui: &mut egui::Ui, name: &str, visible: bool) -> egui::Response {
    let t = Tokens::get();
    let (rect, response) = ui.allocate_exact_size(vec2(t.space_24, t.space_24), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), visible, format!("Show {name}")));
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if response.hovered() || response.has_focus() {
            p.rect_filled(rect, t.radius_sm, t.hover);
        }
        let ink = if visible { t.icon_line } else { t.text_dim };
        let at = |x: f32, y: f32| rect.center() + vec2(x - 11.0, y - 11.0) * (t.space_16 / 14.0);
        let outline = [(4., 11.), (8., 7.), (14., 7.), (18., 11.), (14., 15.), (8., 15.), (4., 11.)];
        p.add(egui::Shape::line(outline.into_iter().map(|(x, y)| at(x, y)).collect(), Stroke::new(1.0, ink)));
        if visible {
            p.circle_filled(at(11., 11.), t.space_2, ink);
        } else {
            p.line_segment([at(4., 17.), at(18., 5.)], Stroke::new(1.0, ink));
        }
    }
    response.on_hover_text(format!("{} {name}", if visible { "Hide" } else { "Show" }))
}

/// Technical layer keys use existing theme roles, with board-edge ink distinct from details.
pub fn technical_layer_color(edge: bool) -> Color32 {
    if edge { Tokens::get().text } else { Tokens::get().text_dim }
}

/// Paint a layer-color key without depending on optional font glyph coverage.
pub fn layer_swatch(ui: &mut egui::Ui, color: Color32) {
    let size = Tokens::get().space_8;
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), size / 2.0, color);
}

/// Reserve a bounded upper-left Browser above Layers, keeping its header visible in short areas.
pub fn browser_bottom(area: Rect) -> f32 {
    area.top() + (area.height() * 0.5).min(320.0).max(crate::workspace::floating_card_min_height()).min(area.height())
}

/// Paint the board's own canvas with the preferred grid style, independently of the schematic
/// and chrome theme. A hidden grid still leaves snapping to the caller's camera.
pub fn canvas(p: &Painter, rect: Rect, origin: Pos2, step: f32, opacity: u8, style: crate::grid::Style) {
    let t = Tokens::get();
    p.rect_filled(rect, 0, t.board_canvas);
    let Some(color) = crate::grid::color(t.board_grid, opacity) else {
        return;
    };
    crate::grid::paint_style(p, rect, origin, step, color, style);
}
