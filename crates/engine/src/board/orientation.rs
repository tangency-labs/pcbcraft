//! Assembly orientation of footprint packages.
//!
//! IPC-7351 section 7.3.1 asks that orientation and polarity of a device be defined in the
//! CAD data, and section 16 defines rotations from the library's zero component orientation.
//! A package shows its orientation on the board when its copper is not symmetric under a
//! quarter or half turn, or when its silkscreen or fabrication drawing marks the way round.
//! [`orientation_marked`] decides that from geometry alone; the definition's optional
//! `orientation` object states polarity and the zero orientation. A polarized package with
//! neither gets one warning.
use super::{
    Footprint,
    definition::{CopperBinding, FootprintDefinition, GraphicPurpose, Key, PadShape},
    view::BoardView,
};
use crate::{
    CommandError, Session,
    library_graphics::{Fill, Geometry},
    schematic::TextAlign,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Report polarized packages whose placement orientation cannot be checked. Edits nothing.
pub const CHECK_ASSEMBLY: &str = "pcb.check_assembly";
/// Registered kind of the finding this check emits.
pub const CHECK_KIND: &str = "assembly.orientation";
/// Rule the finding is raised under.
pub const RULE_ID: &str = "assembly.polarized_orientation";
/// Where the rule comes from.
pub const RULE_SOURCE: &str = "IPC-7351 section 7.3.1 (orientation and polarity defined in the CAD data, PDF page 43) \
     and section 16 (zero component orientations, PDF page 66)";

/// A point in doubled local coordinates, measured from the centroid of the pad centres.
/// Doubling keeps a centroid that falls halfway between nanometres exact.
type Doubled = (i128, i128);

/// Turn a doubled point by `q` quarter turns, as the shared drawing transform does: one turn
/// maps `(x, y)` to `(-y, x)` and adds 90 degrees to drawn angles.
fn turn((x, y): Doubled, q: u8) -> Doubled {
    match q % 4 {
        0 => (x, y),
        1 => (-y, x),
        2 => (-x, -y),
        _ => (y, -x),
    }
}

/// Map local points into doubled coordinates about the pad-centre centroid.
struct Frame {
    pivot: Doubled,
}

impl Frame {
    /// A rotation that maps a set of integer points onto itself fixes their centroid, and
    /// twice that centroid is then an integer point. Return none when it is not, because no
    /// such rotation can exist and the copper alone already shows the orientation.
    fn new(definition: &FootprintDefinition) -> Option<Self> {
        let n = i128::try_from(definition.pads.len()).ok().filter(|n| *n > 0)?;
        let (sx, sy) = definition.pads.iter().fold((0i128, 0i128), |(x, y), pad| (x + i128::from(pad.center.x_nm), y + i128::from(pad.center.y_nm)));
        ((2 * sx) % n == 0 && (2 * sy) % n == 0).then(|| Self { pivot: (2 * sx / n, 2 * sy / n) })
    }

    /// Return twice the point's offset from the pivot, so a half-nanometre centroid stays exact.
    fn point(&self, x: i64, y: i64) -> Doubled {
        (2 * i128::from(x) - self.pivot.0, 2 * i128::from(y) - self.pivot.1)
    }
}

/// Put a rectangle's size and rotation in one form: width at most height, and the rotation
/// reduced to the turns that leave such a rectangle unchanged.
fn rectangle_form(width: i64, height: i64, rotation: u32) -> (i64, i64, u32) {
    let (width, height, rotation) = if width > height { (height, width, rotation + 90) } else { (width, height, rotation) };
    (width, height, rotation % if width == height { 90 } else { 180 })
}

/// One land, in a form that is equal exactly when two lands cover the same copper.
type PadForm = (Doubled, u8, u8, Option<i64>, i64, (i64, i64, u32));

/// Return every land of the definition turned by `q` quarter turns about the frame's pivot,
/// sorted by form, with its pad number. Circles ignore rotation; other shapes add the turn.
fn pad_forms<'d>(definition: &'d FootprintDefinition, frame: &Frame, q: u8) -> Vec<(PadForm, &'d str)> {
    let mut forms: Vec<_> = definition
        .pads
        .iter()
        .map(|pad| {
            let shape = match pad.shape {
                PadShape::Rectangle => 0,
                PadShape::RoundedRectangle => 1,
                PadShape::Circle => 2,
                PadShape::Oval => 3,
            };
            let copper = match pad.copper {
                CopperBinding::ComponentSide => 0,
                CopperBinding::AllCopper => 1,
            };
            let rotation = if pad.shape == PadShape::Circle { 0 } else { u32::from(pad.rotation) + 90 * u32::from(q) };
            let form = (
                turn(frame.point(pad.center.x_nm, pad.center.y_nm), q),
                shape,
                copper,
                pad.drill_nm,
                pad.corner_radius_nm,
                rectangle_form(pad.width_nm, pad.height_nm, rotation),
            );
            (form, pad.number.as_str())
        })
        .collect();
    forms.sort();
    forms
}

/// One piece of drawing, in a form that is equal exactly when two pieces are drawn alike: the
/// same primitive, place and size. Its layer, stroke and fill are kept beside it as [`Ink`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Mark {
    Segment(Doubled, Doubled),
    Rectangle(Doubled, (i64, i64, u32)),
    Circle(Doubled, i64),
    Arc(Doubled, i64, i32, i32),
    Text(Doubled, String, i64, u32, u8),
}

/// A segment with its ends in a fixed order, so the same line drawn either way compares equal.
fn segment(a: Doubled, b: Doubled) -> Mark {
    if a <= b { Mark::Segment(a, b) } else { Mark::Segment(b, a) }
}

/// Append the segments of a polyline, plus the closing segment when it is closed and has at
/// least three points.
fn path(points: &[Doubled], closed: bool, marks: &mut Vec<Mark>) {
    marks.extend(points.windows(2).map(|pair| segment(pair[0], pair[1])));
    if closed && points.len() > 2 {
        marks.push(segment(points[points.len() - 1], points[0]));
    }
}

/// Layer, stroke width and fill of a piece of drawing. The layer is 0 for silkscreen and 1 for
/// fabrication, so a mark on one is never matched by the other. Silkscreen paths and technical
/// rectangles have no stroke of their own, since the layer's setting draws them all alike, so
/// they use zero.
type Ink = (u8, i64, u8);

/// The layer number of a drawing that can mark a part, or none for the courtyard, which bounds
/// the part's space and says nothing about which way round it goes.
fn marking_layer(purpose: GraphicPurpose) -> Option<u8> {
    match purpose {
        GraphicPurpose::Silkscreen => Some(0),
        GraphicPurpose::Fabrication => Some(1),
        GraphicPurpose::Courtyard => None,
    }
}

/// Collect the silkscreen and fabrication drawing of a definition turned by `q` quarter turns,
/// each piece with its ink. The assembly drawing comes from the fabrication layer, so a mark
/// there counts as much as one in the silkscreen. Courtyards mark nothing.
fn marks(definition: &FootprintDefinition, frame: &Frame, q: u8) -> Vec<(Ink, Mark)> {
    let at = |x: i64, y: i64| turn(frame.point(x, y), q);
    let mut marks = Vec::new();
    for silk in &definition.silkscreen {
        let points: Vec<_> = silk.points.iter().map(|p| at(p.x_nm, p.y_nm)).collect();
        path(&points, silk.closed, &mut marks);
    }
    let mut inked: Vec<_> = marks.into_iter().map(|mark| ((0, 0, 0), mark)).collect();
    for rect in &definition.graphics {
        let Some(layer) = marking_layer(rect.purpose) else { continue };
        let mut marks = Vec::new();
        let center = frame.point(rect.center.x_nm, rect.center.y_nm);
        if rect.rotation % 90 == 0 {
            let (w, h) = if rect.rotation % 180 == 90 { (rect.height_nm, rect.width_nm) } else { (rect.width_nm, rect.height_nm) };
            let (w, h) = (i128::from(w), i128::from(h));
            let corners: Vec<_> = [(-w, -h), (w, -h), (w, h), (-w, h)].into_iter().map(|(dx, dy)| turn((center.0 + dx, center.1 + dy), q)).collect();
            path(&corners, true, &mut marks);
        } else {
            let form = rectangle_form(rect.width_nm, rect.height_nm, u32::from(rect.rotation) + 90 * u32::from(q));
            marks.push(Mark::Rectangle(turn(center, q), form));
        }
        inked.extend(marks.into_iter().map(|mark| ((layer, 0, 0), mark)));
    }
    let angle = |degrees: i32| (degrees + 90 * i32::from(q)).rem_euclid(360);
    for artwork in &definition.artwork {
        let Some(layer) = marking_layer(artwork.purpose) else { continue };
        let mut marks = Vec::new();
        let fill = match artwork.graphic.fill {
            Fill::None => 0,
            Fill::Solid => 1,
            Fill::Background => 2,
        };
        match &artwork.graphic.geometry {
            Geometry::Line { from, to } => marks.push(segment(at(from.x_nm, from.y_nm), at(to.x_nm, to.y_nm))),
            Geometry::Rectangle { min, max } => {
                let corners = [at(min.x_nm, min.y_nm), at(max.x_nm, min.y_nm), at(max.x_nm, max.y_nm), at(min.x_nm, max.y_nm)];
                path(&corners, true, &mut marks);
            }
            Geometry::Polyline { points, closed } => {
                let points: Vec<_> = points.iter().map(|p| at(p.x_nm, p.y_nm)).collect();
                path(&points, *closed, &mut marks);
            }
            Geometry::Circle { center, radius_nm } => marks.push(Mark::Circle(at(center.x_nm, center.y_nm), *radius_nm)),
            Geometry::Arc { center, radius_nm, start_degrees, sweep_degrees } => {
                let (start, sweep) = (i32::from(*start_degrees), i32::from(*sweep_degrees));
                let (start, sweep) = if sweep < 0 { (start + sweep, -sweep) } else { (start, sweep) };
                marks.push(if sweep >= 360 {
                    Mark::Circle(at(center.x_nm, center.y_nm), *radius_nm)
                } else {
                    Mark::Arc(at(center.x_nm, center.y_nm), *radius_nm, angle(start), sweep)
                });
            }
            Geometry::Text { at: anchor, text, height_nm, rotation, align } => {
                let align = match align {
                    TextAlign::Start => 0,
                    TextAlign::Middle => 1,
                    TextAlign::End => 2,
                };
                let rotation = angle(i32::from(*rotation)).unsigned_abs();
                marks.push(Mark::Text(at(anchor.x_nm, anchor.y_nm), text.clone(), *height_nm, rotation, align));
            }
        }
        inked.extend(marks.into_iter().map(|mark| ((layer, artwork.graphic.stroke_nm, fill), mark)));
    }
    inked.sort();
    inked
}

/// Whether a placed part of this package shows which way round it goes.
///
/// The copper alone shows it unless a quarter, half or three-quarter turn about the centroid
/// of the pad centres lands every pad on an identical pad and moves at least one pad onto a
/// differently numbered one. For each such turn the silkscreen or the fabrication drawing must
/// change under the same turn, as a cathode bar or a pin 1 dot does. Courtyards do not count.
/// The test is exact: lengths stay integer nanometres.
///
/// A definition without pads shows nothing and is not marked.
pub fn orientation_marked(definition: &FootprintDefinition) -> bool {
    if definition.pads.is_empty() {
        return false;
    }
    let Some(frame) = Frame::new(definition) else { return true };
    let base = pad_forms(definition, &frame, 0);
    let printed = marks(definition, &frame, 0);
    (1..4).all(|q| {
        let turned = pad_forms(definition, &frame, q);
        if turned.iter().map(|(form, _)| form).ne(base.iter().map(|(form, _)| form)) || turned == base {
            return true;
        }
        marks(definition, &frame, q) != printed
    })
}

/// The first 128 bits of a SHA-256 digest of a canonical JSON value, as lowercase hex.
fn digest(value: &Value) -> String {
    Sha256::digest(value.to_string().as_bytes())[..16].iter().map(|b| format!("{b:02x}")).collect()
}

impl BoardView<'_> {
    /// Warnings for polarized packages placed on this board that neither show their
    /// orientation (see [`orientation_marked`]) nor state a zero orientation. There is one
    /// finding per package revision, naming every placement of it in board order; findings
    /// come in package order. A package without an `orientation` statement, or one that is
    /// not polarized, is not reported.
    pub fn orientation_findings(self) -> Vec<Value> {
        let project = self.project();
        let mut packages: BTreeMap<Key, (&FootprintDefinition, Vec<&Footprint>)> = BTreeMap::new();
        for placement in project.board.footprints() {
            if let (Some(key), Some(definition)) = (placement.key(), project.board.definition_for(placement)) {
                packages.entry(key).or_insert_with(|| (definition, Vec::new())).1.push(placement);
            }
        }
        let mut findings = Vec::new();
        for ((library, id, revision), (definition, placements)) in packages {
            let Some(orientation) = definition.orientation.as_ref() else { continue };
            if !orientation.polarized || orientation.zero.is_some() || orientation_marked(definition) {
                continue;
            }
            // Only a reference is needed; do not resolve pad nets or render placement graphics.
            let placed: Vec<_> = placements
                .into_iter()
                .filter_map(|p| {
                    let reference =
                        project.components.get(&p.component).map(|component| component.reference()).or_else(|| {
                            project.board.baseline.as_ref()?.components.get(&p.component).map(|component| component.reference.as_str())
                        })?;
                    Some((p, format!("footprint-{}", p.id), reference))
                })
                .collect();
            if placed.is_empty() {
                continue;
            }
            let objects: Vec<_> =
                placed.iter().map(|(_, id, reference)| json!({"id":id,"kind":"footprint","role":"placement","reference":reference})).collect();
            let mut locations: Vec<_> = placed.iter().map(|(p, ..)| json!({"point":{"x_nm":p.x_nm,"y_nm":p.y_nm}})).collect();
            let (xs, ys): (Vec<_>, Vec<_>) = placed.iter().map(|(p, ..)| (p.x_nm, p.y_nm)).unzip();
            locations.push(
                json!({"bounds":{"min":{"x_nm":xs.iter().min(),"y_nm":ys.iter().min()},"max":{"x_nm":xs.iter().max(),"y_nm":ys.iter().max()}}}),
            );
            let names: Vec<_> = placed.iter().map(|(_, _, reference)| *reference).collect();
            let key = digest(&json!({"check_kind":CHECK_KIND,"rule_id":RULE_ID,"package":{"library":library,"id":id,"revision":revision}}));
            let fingerprint = digest(&json!({"definition":definition,"placements":objects}));
            findings.push(json!({
                "key": key,
                "fingerprint": fingerprint,
                "check_kind": CHECK_KIND,
                "rule_id": RULE_ID,
                "rule_source": RULE_SOURCE,
                "severity": "warning",
                "objects": objects,
                "package": {"library":library,"id":id,"revision":revision,"name":definition.name},
                "locations": locations,
                "message": format!(
                    "Polarized package {} ({library}/{id} r{revision}) has no pin 1 or polarity mark in its copper, silkscreen or fabrication drawing \
                     and states no zero orientation, so its placement rotation cannot be checked. Placed as {}.",
                    definition.name,
                    names.join(", ")
                ),
            }));
        }
        findings
    }
}

impl Session {
    /// Read-only assembly orientation report for one board, the only board when `board` is
    /// omitted. Fails when no project is open or the board is unknown or unavailable. Changes
    /// nothing.
    pub(crate) fn assembly_check(&self, params: &Value) -> Result<Value, CommandError> {
        let project = self.open_project()?;
        let target = project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let view = project.board_view(target).ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
        Ok(json!({"revision":self.revision(),"assembly_check":{"board":target,"findings":view.orientation_findings()}}))
    }
}

#[cfg(test)]
mod tests;
