//! Original PCB dimension illustrations; geometry is illustrative and never edits the model.
use egui::{Pos2, Rect, Stroke, Ui, vec2};
use pcbcraft_chrome::Tokens;
use pcbcraft_engine::net_classes::Property;

/// Human dimensional labels are shared by the authored and effective presentations.
pub(super) fn label(property: Property) -> &'static str {
    use Property::*;
    match property {
        ClearanceNm => "Copper clearance",
        MinTrackWidthNm => "Minimum track width",
        EdgeClearanceNm => "Board edge clearance",
        MinAnnularRingNm => "Minimum annular ring",
        MinViaDiameterNm => "Minimum via diameter",
        MaxViaDiameterNm => "Maximum via diameter",
        MinViaDrillNm => "Minimum via drill",
        MaxViaDrillNm => "Maximum via drill",
        PreferredTrackWidthNm => "Preferred track width",
        PreferredViaDiameterNm => "Preferred via diameter",
        PreferredViaDrillNm => "Preferred via drill",
    }
}

/// Describe the measured boundaries independently of color, scale and the current numeric value.
fn description(property: Property) -> &'static str {
    use Property::*;
    match property {
        ClearanceNm => "Space between copper edges on different nets.",
        MinTrackWidthNm | PreferredTrackWidthNm => "Width across the copper track.",
        EdgeClearanceNm => "Space from copper to the board edge.",
        MinAnnularRingNm => "Radial copper from the drill edge to the outside of the via. The ring is on both sides of the hole.",
        MinViaDiameterNm | MaxViaDiameterNm | PreferredViaDiameterNm => "Outside diameter of the via copper, including the hole.",
        MinViaDrillNm | MaxViaDrillNm | PreferredViaDrillNm => "Diameter of the drilled hole inside the via.",
    }
}

/// Draw a fixed-scale example of the named dimension beside its field. Screen readers receive
/// the same boundary description as the hover; the drawing is not a live geometry preview.
pub(super) fn show(ui: &mut Ui, property: Property) {
    let (rect, response) = ui.allocate_exact_size(vec2(104., 48.), egui::Sense::hover());
    let caption = format!("{} diagram: {}", label(property), description(property));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &caption));
    response.on_hover_text(format!("{} Illustrative, not to scale.", description(property)));
    if !ui.is_rect_visible(rect) {
        return;
    }
    let t = Tokens::get();
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let at = |x, y| rect.min + vec2(x, y);
    let copper = t.stack_copper;
    let dimension = Stroke::new(1.5, t.accent);
    use Property::*;
    match property {
        ClearanceNm => {
            painter.rect_filled(Rect::from_min_max(at(8., 6.), at(40., 42.)), 4., copper);
            painter.rect_filled(Rect::from_min_max(at(68., 6.), at(100., 42.)), 4., copper);
            measure(&painter, at(40., 24.), at(68., 24.), dimension);
        }
        EdgeClearanceNm => {
            painter.rect_filled(Rect::from_min_max(at(8., 10.), at(46., 38.)), 4., copper);
            painter.line_segment([at(77., 2.), at(77., 46.)], Stroke::new(2., t.text_dim));
            for y in [2., 14., 26., 38.] {
                painter.line_segment([at(80., y + 8.), at(88., y)], Stroke::new(1., t.text_dim));
            }
            measure(&painter, at(46., 24.), at(77., 24.), dimension);
        }
        MinTrackWidthNm | PreferredTrackWidthNm => {
            painter.rect_filled(Rect::from_min_max(at(8., 14.), at(74., 34.)), 0., copper);
            for y in [14., 34.] {
                painter.line_segment([at(77., y), at(98., y)], Stroke::new(1., t.text_dim));
            }
            measure(&painter, at(89., 14.), at(89., 34.), dimension);
        }
        MinAnnularRingNm | MinViaDiameterNm | MaxViaDiameterNm | PreferredViaDiameterNm | MinViaDrillNm | MaxViaDrillNm | PreferredViaDrillNm => {
            let center = at(44., 24.);
            painter.circle_filled(center, 21., copper);
            painter.circle_filled(center, 10., t.dialog_body);
            painter.circle_stroke(center, 10., Stroke::new(1., t.text_dim));
            let (left, right) = match property {
                MinAnnularRingNm => (54., 65.),
                MinViaDrillNm | MaxViaDrillNm | PreferredViaDrillNm => (34., 54.),
                _ => (23., 65.),
            };
            measure(&painter, at(left, 24.), at(right, 24.), dimension);
        }
    }
}

/// Mark the exact measured endpoints with extension ticks and inward-facing arrowheads.
fn measure(painter: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    let direction = (b - a).normalized();
    let normal = vec2(-direction.y, direction.x);
    painter.line_segment([a, b], stroke);
    for (point, inward) in [(a, direction), (b, -direction)] {
        painter.line_segment([point - normal * 5., point + normal * 5.], stroke);
        for side in [-1., 1.] {
            painter.line_segment([point, point + inward * 3.5 + normal * 2.5 * side], stroke);
        }
    }
}
