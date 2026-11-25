//! Board inspection primitives; physical geometry and selection ownership stay with callers.
use crate::Tokens;
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, vec2};

const CONTEXT_OPACITY: f32 = 0.42;

/// Preserve layer hues when lightening selections or dimming copper during a net highlight.
pub fn copper_emphasis(color: Color32, emphasized: Option<bool>) -> Color32 {
    match emphasized {
        Some(true) => color.lerp_to_gamma(Color32::WHITE, 0.35),
        Some(false) => color.gamma_multiply(CONTEXT_OPACITY),
        None => color,
    }
}

/// Keep non-copper context visible at reduced opacity while copper is emphasized.
pub fn context_painter(painter: &Painter, dimmed: bool) -> Painter {
    let mut painter = painter.clone();
    if dimmed {
        painter.multiply_opacity(CONTEXT_OPACITY);
    }
    painter
}

/// Paint an accepted logical connection behind copper without suggesting routed geometry.
pub fn airwire(p: &Painter, from: Pos2, to: Pos2) {
    p.line_segment([from, to], Stroke::new(1., Tokens::get().board_ink.gamma_multiply(0.55)));
}

/// Emphasize an existing unrouted connection belonging to the selected reference's component.
pub fn selected_airwire(p: &Painter, from: Pos2, to: Pos2) {
    p.line_segment([from, to], Stroke::new(1.5, Tokens::get().accent));
}

/// Outline visible reference ink for hover or selection without expanding the footprint body bounds.
pub fn reference_outline(p: &Painter, bounds: Rect, selected: bool, color: Color32) {
    if bounds.is_finite() && bounds.is_positive() {
        let color = copper_emphasis(color, Some(true)).gamma_multiply(if selected { 1.0 } else { 0.6 });
        p.rect_stroke(bounds.expand(2.0), 0.0, Stroke::new(1.0, color), egui::StrokeKind::Outside);
    }
}

/// Connect selected or moving reference ink to its physical owner with a view-only dashed leader.
/// Suppress the leader when ink is hidden or overlaps the owner; canvas clipping remains the painter's responsibility.
pub fn reference_owner_link(p: &Painter, bounds: Rect, owner: Pos2, color: Color32) {
    if !bounds.is_finite() || !bounds.is_positive() || bounds.contains(owner) {
        return;
    }
    let stroke = Stroke::new(1., copper_emphasis(color, Some(true)).gamma_multiply(0.65));
    p.extend(egui::Shape::dashed_line(&[bounds.clamp(owner), owner], stroke, 4., 3.));
    p.circle_stroke(owner, 3., stroke);
}

/// The ratsnest is a view overlay, separate from physical drawing layers.
pub fn ratsnest_visibility(ui: &mut egui::Ui, visible: &mut bool) -> egui::Response {
    crate::controls::checkbox(ui, visible, "Ratsnest")
}

/// Paint layer linework at its physical screen width with round ends. Missing width retains
/// legacy hairlines; selection, previews and handles use the shared accent.
pub fn drawing(p: &Painter, points: Vec<Pos2>, closed: bool, width: Option<f32>, selected: bool, preview: bool, color: Color32) {
    let t = Tokens::get();
    let color = if selected || preview { t.accent } else { color };
    if let Some(width) = width {
        for (a, b) in points.iter().zip(points.iter().cycle().skip(1)).take(points.len().saturating_sub(usize::from(!closed))) {
            if selected {
                track(p, [*a, *b], width.max(1.) + 3., t.accent, false, false);
            }
            track(p, [*a, *b], width, color, false, preview);
        }
    } else {
        p.add(egui::Shape::Path(egui::epaint::PathShape {
            points: points.clone(),
            closed,
            fill: Color32::TRANSPARENT,
            stroke: Stroke::new(1.5, color).into(),
        }));
    }
    if selected {
        drawing_vertices(p, points);
    }
}

/// Draw the shared handles for an editable canvas path.
pub fn drawing_vertices(p: &Painter, points: impl IntoIterator<Item = Pos2>) {
    let accent = Tokens::get().accent;
    for point in points {
        p.rect_filled(Rect::from_center_size(point, vec2(7., 7.)), 0, accent);
    }
}

/// Paint one copper track segment at its physical width with round ends, never thinner than a
/// hairline. Selection and previews lighten the supplied layer color without adding an outline.
pub fn track(p: &Painter, ends: [Pos2; 2], width: f32, color: Color32, selected: bool, preview: bool) {
    let width = width.max(1.);
    let fill = copper_emphasis(color, (selected || preview).then_some(true));
    p.line_segment(ends, Stroke::new(width, fill));
    for end in ends {
        p.circle_filled(end, width / 2., fill);
    }
}

/// Shade the clearance around a round-ended segment or via, leaving nearby copper visible.
/// The caller supplies the drawn copper width and clearance in logical screen points.
pub fn routing_clearance(p: &Painter, [a, b]: [Pos2; 2], width: f32, clearance: f32) {
    let color = Tokens::get().board_ink.gamma_multiply(0.19);
    let radius = width.max(1.0) / 2.0 + clearance;
    let direction = (b - a).normalized();
    if direction == egui::Vec2::ZERO {
        p.circle_filled(a, radius, color);
        return;
    }
    let points = [(a, -direction), (b, direction)]
        .into_iter()
        .flat_map(|(center, outward)| {
            (0..=12).map(move |step| {
                let angle = outward.angle() + std::f32::consts::PI * (step as f32 / 12.0 - 0.5);
                center + egui::Vec2::angled(angle) * radius
            })
        })
        .collect();
    p.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

/// Paint the projected finished opening without a copper annulus or pad label.
pub fn non_plated_hole(p: &Painter, center: Pos2, radius: f32, selected: bool) {
    let t = Tokens::get();
    let ink = if selected { t.icon_active_2 } else { t.board_ink };
    p.circle(center, radius, t.board_canvas, egui::Stroke::new(1., ink));
}

/// Paint a through via: a round pad of `radius` with its drill hole, never smaller than a dot.
/// Selection lightens through-hole copper without an accent ring. Net focus takes precedence
/// over selection, matching pads; the placement preview retains its separate ghost color.
pub fn via(p: &Painter, center: Pos2, radius: f32, drill_radius: f32, selected: bool, preview: bool, emphasized: Option<bool>) {
    let t = Tokens::get();
    let fill = if preview { t.accent.gamma_multiply(0.6) } else { copper_emphasis(t.board_through, emphasized.or_else(|| selected.then_some(true))) };
    via_land(p, center, radius, drill_radius, fill);
}

/// Mark a blocking via's copper while preserving its physical drill opening.
pub fn collision_via(p: &Painter, center: Pos2, radius: f32, drill_radius: f32) {
    via_land(p, center, radius, drill_radius, Tokens::get().route_collision);
}

/// Draw a filled annulus around the actual via drill.
fn via_land(p: &Painter, center: Pos2, radius: f32, drill_radius: f32, fill: Color32) {
    let radius = radius.max(2.);
    p.circle_filled(center, radius, fill);
    p.circle_filled(center, drill_radius.clamp(0.5, radius - 0.5), Tokens::get().board_canvas);
}

/// Draw a measuring ruler: a dashed accent line with end ticks and a label near its middle.
pub fn measurement(p: &Painter, a: Pos2, b: Pos2, label: &str) {
    let t = Tokens::get();
    let stroke = Stroke::new(1.5, t.accent);
    p.extend(egui::Shape::dashed_line(&[a, b], stroke, 6., 4.));
    let direction = (b - a).normalized();
    let normal = if direction.is_finite() && direction.length_sq() > 0. { vec2(-direction.y, direction.x) } else { vec2(0., 1.) };
    for end in [a, b] {
        p.line_segment([end - normal * 6., end + normal * 6.], stroke);
    }
    let galley = p.layout_no_wrap(label.to_owned(), FontId::proportional(t.font_small), t.text);
    let at = a + (b - a) / 2. + normal * 14.;
    let rect = Rect::from_center_size(at, galley.size() + vec2(t.space_8, t.space_4));
    p.rect_filled(rect, t.radius_sm, t.panel);
    p.galley(rect.center() - galley.size() / 2., galley, t.text);
}

/// Mark a design rules finding: a small warning triangle in the error color, filled and ringed
/// when it is the focused finding.
pub fn drc_marker(p: &Painter, at: Pos2, focused: bool) {
    let t = Tokens::get();
    let size = if focused { 9. } else { 6. };
    let points = vec![at + vec2(0., -size), at + vec2(size * 0.9, size * 0.6), at + vec2(-size * 0.9, size * 0.6)];
    if focused {
        p.circle_stroke(at, size * 1.6, Stroke::new(1.5, t.error));
        p.add(egui::Shape::convex_polygon(points, t.error, Stroke::new(1., t.board_canvas)));
    } else {
        p.add(egui::Shape::closed_line(points, Stroke::new(1.5, t.error)));
    }
}

/// Distinguish the snapped drawing target from authored vertices without changing its position.
pub fn drawing_target(p: &Painter, point: Pos2) {
    p.circle_stroke(point, 5., Stroke::new(1., Tokens::get().accent));
}

/// Indicate the reference for a snapped outline axis using the canvas selection accent.
pub fn drawing_alignment(p: &Painter, points: [Pos2; 2]) {
    let extension = (points[1] - points[0]).normalized() * 16.;
    let stroke = Stroke::new(1., Tokens::get().accent.gamma_multiply(0.7));
    p.extend(egui::Shape::dashed_line(&[points[0] - extension, points[1] + extension], stroke, 4., 3.));
    drawing_target(p, points[0]);
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
    /// Projected round cap centres and radius for an independent plated slot.
    pub plated_slot: Option<([Pos2; 2], f32)>,
    pub through_hole: bool,
    pub net: Option<String>,
}

/// Tessellate a slot from the projected cap centres and physical radius. A degenerate
/// spine is a circle, preserving the equal-width-and-length case.
fn slot_polygon([a, b]: [Pos2; 2], radius: f32) -> Vec<Pos2> {
    let axis = b - a;
    let angle = axis.y.atan2(axis.x);
    [a, b]
        .into_iter()
        .enumerate()
        .flat_map(|(end, center)| {
            (0..=16).map(move |i| {
                let theta = angle + (0.5 + end as f32 + i as f32 / 16.) * std::f32::consts::PI;
                center + vec2(theta.cos(), theta.sin()) * radius
            })
        })
        .collect()
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
/// Solid lands use their copper for contrast. Plated openings use a fine halo to keep the
/// annotation readable across the drill opening and the surrounding copper.
fn pad_label(p: &Painter, pad: &PadDrawing, corners: [Pos2; 4], copper: Color32) {
    let mut axis = corners[1] - corners[0];
    let mut width = axis.length();
    let mut height = (corners[3] - corners[0]).length();
    if height > width * 1.3 {
        std::mem::swap(&mut width, &mut height);
        axis = corners[3] - corners[0];
    }
    // Fit in an inscribed rectangle, including curved corners rather than just the land's
    // bounding box. This keeps the two-line annotation out of adjacent clearance gaps.
    match pad.contour {
        PadContour::Circle | PadContour::Oval => {
            width *= std::f32::consts::FRAC_1_SQRT_2;
            height *= std::f32::consts::FRAC_1_SQRT_2;
        }
        PadContour::RoundedRectangle { radius } => {
            let inset = 2. * radius.min(width.min(height) / 2.) * (1. - std::f32::consts::FRAC_1_SQRT_2);
            width -= inset;
            height -= inset;
        }
        PadContour::Rectangle => {}
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
    let opening = pad.drill_radius.is_some() || pad.plated_slot.is_some();
    let canvas = Tokens::get().board_canvas;
    let label_ink = copper_label_ink(if opening { canvas } else { copper });
    let galley = crate::canvas_text::layout(p, text, FontId::monospace(size), label_ink, egui::Align::Center);
    let pos = pad.center - galley.rect.center().to_vec2();
    let text = egui::epaint::TextShape::new(pos, galley, label_ink).with_angle_and_anchor(angle, Align2::CENTER_CENTER);
    p.add(if opening { outlined_copper_label(text, canvas) } else { egui::Shape::Text(text) });
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
    /// Reference paths do not expand the footprint's selection bounds.
    pub reference: bool,
}

impl GraphicDrawing {
    /// Screen-space ink bounds including physical stroke width.
    pub fn bounds(&self) -> Rect {
        self.points.iter().fold(Rect::NOTHING, |rect, point| rect.union(Rect::from_pos(*point))).expand(self.stroke_width.unwrap_or(0.0) / 2.0)
    }
}

/// Bounds include pads and fixed technical graphics. Legacy markers retain their
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
    for graphic in graphics.iter().filter(|graphic| !graphic.reference) {
        bounds = bounds.union(graphic.bounds());
    }
    bounds.expand(4.0)
}

/// Anchor for the editor-only reference on footprints without generated silkscreen lettering.
fn reference_anchor(bounds: Rect, shift: egui::Vec2) -> Pos2 {
    bounds.center_top() - vec2(0., Tokens::get().space_2) + shift
}

/// Hit bounds for the editor-only reference, using the same font and anchor as its painter.
pub fn footprint_reference_bounds(p: &Painter, bounds: Rect, text: &str, shift: egui::Vec2) -> Rect {
    let t = Tokens::get();
    let size = p.layout_no_wrap(text.to_owned(), FontId::proportional(t.font_body), t.board_ink).size();
    Align2::CENTER_BOTTOM.anchor_size(reference_anchor(bounds, shift), size)
}

/// Paint physical land contours and plated drills at their projected dimensions, with separate
/// technical graphics and lighter layer-colored selection ink. Only legacy definitions use center markers. Small
/// lands omit labels before they become illegible; physical dimensions never depend on zoom.
/// With copper emphasis active, use the caller's pad colors and dim technical graphics and text.
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
    copper_focused: bool,
    reference_shift: egui::Vec2,
) {
    let bounds = footprint_bounds(pads, graphics, center);
    footprint_in_bounds(p, pads, graphics, reference, center, copper, selected, highlighted, copper_focused, reference_shift, bounds, None, None);
}

/// A plated slot opening with its pad label and styling as resolved by its own footprint pass,
/// held back so it can be painted after copper from footprints painted later.
#[derive(Clone, Debug)]
pub struct SlotOpening {
    pad: PadDrawing,
    corners: [Pos2; 4],
    color: Color32,
    dimmed: bool,
}

/// Paint a deferred slot opening and its pad label exactly as its own footprint pass would have.
pub fn slot_opening(p: &Painter, opening: &SlotOpening) {
    land_opening(p, &context_painter(p, opening.dimmed), &opening.pad, opening.corners, opening.color);
}

/// Paint the selection frame and editor reference from the caller's shared visible hit bounds.
/// With `slots`, plated slot openings and their labels are collected for [`slot_opening`]
/// instead of being painted here; without it, every opening is painted in this pass.
#[allow(clippy::too_many_arguments)]
pub fn footprint_in_bounds(
    p: &Painter,
    pads: &[PadDrawing],
    graphics: &[GraphicDrawing],
    reference: &str,
    center: Pos2,
    copper: Color32,
    selected: bool,
    highlighted: bool,
    copper_focused: bool,
    reference_shift: egui::Vec2,
    bounds: Rect,
    reference_overview: Option<egui::Shape>,
    mut slots: Option<&mut Vec<SlotOpening>>,
) {
    let t = Tokens::get();
    let ink = copper_emphasis(pads.first().and_then(|pad| pad.color).unwrap_or(copper), (selected || highlighted).then_some(true));
    let context = context_painter(p, copper_focused);
    for graphic in graphics.iter().filter(|graphic| !graphic.reference || reference_overview.is_none()) {
        let width = match graphic.style {
            GraphicStyle::Fabrication => 0.8,
            GraphicStyle::Silkscreen => 1.2,
            GraphicStyle::Courtyard => 0.7,
        };
        let base = graphic.color.unwrap_or_else(|| graphic_color(graphic.style));
        let color = copper_emphasis(base, (selected || highlighted).then_some(true));
        let stroke = Stroke::new(graphic.stroke_width.unwrap_or(width), color);
        context.add(if graphic.closed {
            egui::Shape::closed_line(graphic.points.clone(), stroke)
        } else {
            egui::Shape::line(graphic.points.clone(), stroke)
        });
        if let Some(width) = graphic.stroke_width {
            for point in &graphic.points {
                context.circle_filled(*point, width / 2., color);
            }
        }
    }
    if let Some(label) = reference_overview {
        context.add(label);
    }
    if selected || highlighted {
        context.rect_stroke(bounds, t.radius_sm, Stroke::new(1.5, ink), egui::StrokeKind::Inside);
    }
    let extent = pads.iter().fold(Rect::from_pos(center), |r, p| r.union(Rect::from_pos(p.center)));
    let (offset, align) = if extent.height() > extent.width() { (vec2(9., 0.), Align2::LEFT_CENTER) } else { (vec2(0., 9.), Align2::CENTER_TOP) };
    for pad in pads {
        let color = copper_emphasis(pad.color.unwrap_or(copper), (!copper_focused && (selected || highlighted)).then_some(true));
        if let Some(points) = pad.copper {
            p.add(egui::Shape::convex_polygon(pad_contour(pad), color, Stroke::NONE));
            match slots.as_deref_mut() {
                Some(slots) if pad.plated_slot.is_some() => {
                    slots.push(SlotOpening { pad: pad.clone(), corners: points, color, dimmed: copper_focused });
                }
                _ => land_opening(p, &context, pad, points, color),
            }
        } else {
            p.circle_stroke(pad.center, 5., Stroke::new(1.5, color));
            p.line_segment([pad.center - vec2(2., 0.), pad.center + vec2(2., 0.)], Stroke::new(1., color));
            p.line_segment([pad.center - vec2(0., 2.), pad.center + vec2(0., 2.)], Stroke::new(1., color));
            context.text(pad.center + offset, align, &pad.number, FontId::proportional(t.font_small), t.board_ink);
        }
    }
    context.text(
        reference_anchor(bounds, reference_shift),
        Align2::CENTER_BOTTOM,
        reference,
        FontId::proportional(t.font_body),
        copper_emphasis(t.board_ink, (selected || highlighted).then_some(true)),
    );
}

/// Open a land's plated hole or slot to the canvas, then label the land above the opening.
fn land_opening(p: &Painter, context: &Painter, pad: &PadDrawing, points: [Pos2; 4], color: Color32) {
    let t = Tokens::get();
    let hole_edge = Stroke::new(0.7, t.board_ink.gamma_multiply(0.5));
    if let Some((ends, radius)) = pad.plated_slot.filter(|(ends, _)| ends[0] != ends[1]) {
        let opening = slot_polygon(ends, radius);
        p.add(egui::Shape::convex_polygon(opening.clone(), t.board_canvas, Stroke::NONE));
        context.add(egui::Shape::closed_line(opening, hole_edge));
    } else if let Some(radius) = pad.plated_slot.map(|(_, radius)| radius).or(pad.drill_radius) {
        p.circle_filled(pad.center, radius, t.board_canvas);
        context.circle_stroke(pad.center, radius, hole_edge);
    }
    let mut label_painter = p.clone();
    label_painter.multiply_opacity(f32::from(color.a()) / 255.0);
    pad_label(&label_painter, pad, points, color);
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
pub fn color_dialog(
    ctx: &egui::Context,
    title: &str,
    name: &str,
    reset: &str,
    rgb: &mut [u8; 3],
    hex: &mut String,
    error: Option<&str>,
) -> LayerColorAction {
    let mut action = LayerColorAction::None;
    let closing = crate::shell::scrolling_dialog(ctx, "board_color", title, |ui| {
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
            if crate::controls::button(ui, "Apply color").clicked() {
                action = LayerColorAction::Apply;
            }
            if crate::controls::button(ui, reset).clicked() {
                action = LayerColorAction::Reset;
            }
            if crate::controls::button(ui, "Cancel").clicked() {
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
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), visible, format!("Show {name}")));
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

/// Keep domain annotation ink legible against either a dark or a bright copper swatch.
fn copper_label_ink(copper: Color32) -> Color32 {
    let [r, g, b, _] = copper.to_normalized_gamma_f32();
    if 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.65 { Tokens::get().board_canvas } else { Color32::WHITE }
}

/// Paint repeated upright names wholly within a visible track's copper. Clipping the centerline
/// first bounds text preparation for long off-screen segments; sub-legible widths show no text.
pub fn track_net_label(p: &Painter, ends: [Pos2; 2], width: f32, name: &str, copper: Color32) {
    track_net_labels(p, ends, width, name, copper, |shape| {
        p.add(shape);
    });
}

/// Lay out sparse upright labels in the visible span; the caller controls overlap and painting.
fn track_net_labels(p: &Painter, ends: [Pos2; 2], width: f32, name: &str, copper: Color32, mut emit: impl FnMut(egui::epaint::TextShape)) {
    let size = (width - 4.).min(14.);
    if size < 7. {
        return;
    }
    let delta = ends[1] - ends[0];
    let length = delta.length();
    if length <= f32::EPSILON {
        return;
    }
    let axis = delta / length;
    let clip = p.clip_rect().shrink(width / 2.);
    let (mut lo, mut hi) = (0.0_f32, length);
    for (origin, step, min, max) in [(ends[0].x, axis.x, clip.min.x, clip.max.x), (ends[0].y, axis.y, clip.min.y, clip.max.y)] {
        if step.abs() <= f32::EPSILON {
            if origin < min || origin > max {
                return;
            }
        } else {
            let a = (min - origin) / step;
            let b = (max - origin) / step;
            lo = lo.max(a.min(b));
            hi = hi.min(a.max(b));
        }
    }
    if lo >= hi {
        return;
    }
    let ink = copper_label_ink(copper);
    let galley = crate::canvas_text::layout(p, name.to_owned(), FontId::monospace(size), ink, egui::Align::Center);
    if galley.size().y > width - 2. || galley.size().x + 16. > hi - lo {
        return;
    }
    let mut angle = axis.y.atan2(axis.x);
    if angle > std::f32::consts::FRAC_PI_2 {
        angle -= std::f32::consts::PI;
    }
    if angle < -std::f32::consts::FRAC_PI_2 {
        angle += std::f32::consts::PI;
    }
    let count = ((hi - lo) / (galley.size().x + 120.).max(400.)).floor().max(1.) as usize;
    for i in 0..count {
        let center = ends[0] + axis * (lo + (hi - lo) * (i as f32 + 0.5) / count as f32);
        let pos = center - galley.rect.center().to_vec2();
        emit(egui::epaint::TextShape::new(pos, galley.clone(), ink).with_angle_and_anchor(angle, Align2::CENTER_CENTER));
    }
}

mod track_labels;
pub use track_labels::TrackLabels;

/// Center a readable name across the via's full diameter, independently of its drill size.
/// A fine glyph halo maintains contrast across both ring and hole. Hide names
/// that cannot fit inside the circular outline at 11 pt or would be clipped by the viewport.
pub fn via_net_label(p: &Painter, center: Pos2, diameter: f32, name: &str) {
    if let Some(shape) = via_net_label_shape(p, center, diameter, name) {
        p.add(shape);
    }
}

/// Group foreground and halo into one removable annotation, rejecting unreadable or clipped text.
fn via_net_label_shape(p: &Painter, center: Pos2, diameter: f32, name: &str) -> Option<egui::Shape> {
    if name.is_empty() || !diameter.is_finite() || diameter <= 10. || !p.clip_rect().contains(center) {
        return None;
    }
    let halo = Tokens::get().board_canvas;
    let ink = copper_label_ink(halo);
    let galley = crate::canvas_text::layout(p, name.to_owned(), FontId::monospace(14.), ink, egui::Align::Center);
    let extent = galley.mesh_bounds.size().length();
    if !extent.is_finite() || extent <= f32::EPSILON {
        return None;
    }
    // Reserving ten points on the diagonal keeps the text and its fine outline inside
    // the circle, including its corners, with at least two points clear of the outer edge.
    let scale = ((diameter - 10.) / extent).min(1.);
    if 14. * scale < 11. {
        return None;
    }
    let mut text = egui::epaint::TextShape::new(Pos2::ZERO, galley, ink);
    text.transform(egui::emath::TSTransform::from_scaling(scale));
    text.pos = center - text.galley.mesh_bounds.center().to_vec2();
    if !p.clip_rect().contains_rect(text.visual_bounding_rect().expand(COPPER_LABEL_HALO)) {
        return None;
    }
    Some(outlined_copper_label(text, halo))
}

const COPPER_LABEL_HALO: f32 = 0.75;

/// Outline an annotation for contrast where it crosses both a plated opening and its copper.
fn outlined_copper_label(text: egui::epaint::TextShape, halo: Color32) -> egui::Shape {
    let offset = COPPER_LABEL_HALO;
    let mut shapes = Vec::with_capacity(5);
    for shift in [vec2(-offset, 0.), vec2(offset, 0.), vec2(0., -offset), vec2(0., offset)] {
        let mut outline = text.clone().with_override_text_color(halo);
        outline.pos += shift;
        shapes.push(egui::Shape::Text(outline));
    }
    shapes.push(egui::Shape::Text(text));
    egui::Shape::Vec(shapes)
}

#[cfg(test)]
mod net_label_tests;

#[cfg(test)]
mod slot_tests {
    use super::*;

    /// The painter draws physical slot ends and sides, not a round marker at the pad centre.
    #[test]
    fn tilted_slot_polygon_follows_the_projected_spine() {
        let ends = [Pos2::new(0., 0.), Pos2::new(30., 40.)];
        let polygon = slot_polygon(ends, 5.);
        let axis = (ends[1] - ends[0]).normalized();
        let normal = vec2(-axis.y, axis.x);
        let along = |v: egui::Vec2| (v.dot(axis), v.dot(normal));
        let (left, across, right, other_side) = polygon.iter().fold((0_f32, 0_f32, 0_f32, 0_f32), |(l, t, r, b), vertex| {
            let (x, y) = along(*vertex - ends[0]);
            (l.min(x), t.min(y), r.max(x), b.max(y))
        });
        for (actual, expected) in [(left, -5.), (across, -5.), (right, 55.), (other_side, 5.)] {
            assert!((actual - expected).abs() < 0.001);
        }
    }
}

/// Short diagonal strokes at the real polygon boundary distinguish zone intent from fill.
/// Screen-space spacing stays stable while panning; viewport clipping never creates fake edges.
pub fn zone_hatch(p: &Painter, points: &[Pos2], color: Color32) {
    zone_hatch_segments(points, p.clip_rect(), |segment| {
        p.line_segment(segment, Stroke::new(0.7, color.gamma_multiply(0.5)));
    });
}

/// Scan globally phased 45-degree lines through the polygon, retaining only bounded end strokes.
/// Pairing half-open edge intersections handles concavity and either winding. Work is bounded
/// by the visible diagonal range and 512 scan lines, independently of offscreen polygon area.
fn zone_hatch_segments(points: &[Pos2], clip: Rect, mut emit: impl FnMut([Pos2; 2])) {
    const SPACING: f32 = 18.;
    const LENGTH: f32 = 18.;
    if points.len() < 3 {
        return;
    }
    let bounds = Rect::from_points(points).intersect(clip.expand(LENGTH));
    if !bounds.is_positive() {
        return;
    }
    let first = ((bounds.left() + bounds.top()) / SPACING).floor() * SPACING;
    let count = (((bounds.right() + bounds.bottom() - first) / SPACING).ceil() as usize).min(512);
    let mut intersections = Vec::with_capacity(points.len());
    for index in 0..count {
        let diagonal = first + index as f32 * SPACING;
        intersections.clear();
        for (a, b) in points.iter().zip(points.iter().cycle().skip(1)).take(points.len()) {
            let (start, end) = (a.x + a.y, b.x + b.y);
            if (start <= diagonal && end > diagonal) || (end <= diagonal && start > diagonal) {
                intersections.push(a.x + (diagonal - start) * (b.x - a.x) / (end - start));
            }
        }
        intersections.sort_by(f32::total_cmp);
        for pair in intersections.as_chunks::<2>().0 {
            let a = egui::pos2(pair[0], diagonal - pair[0]);
            let b = egui::pos2(pair[1], diagonal - pair[1]);
            let delta = b - a;
            let length = delta.length();
            if length <= 2. * LENGTH {
                if Rect::from_two_pos(a, b).intersects(clip) {
                    emit([a, b]);
                }
            } else {
                let stub = delta * (LENGTH / length);
                for segment in [[a, a + stub], [b - stub, b]] {
                    if Rect::from_two_pos(segment[0], segment[1]).intersects(clip) {
                        emit(segment);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod zone_hatch_tests;
