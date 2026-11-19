//! Pure reference resolution and measurement over one immutable native board.

use super::{Corner, Definition, Kind, Point, Reference, Side, Witness};
use crate::board::{
    Board, Footprint,
    definition::{CopperBinding, GraphicPurpose, GraphicRect, PadShape, RectPad},
    view::{CopperLayers, rectangle_points},
};
use crate::{Project, schematic::PinNumber};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Axis {
    X,
    Y,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Measure {
    Projection { axis: Axis, signed_nm: i64, witnesses: [Witness; 2] },
    Aligned { distance_nm: f64, witnesses: [Witness; 2] },
}

impl Definition {
    /// Read a placement's exact physical definition key, retaining the native legacy marker case.
    fn of(fp: &Footprint) -> Self {
        Self { library: fp.library.clone(), id: fp.definition.clone(), revision: fp.definition_revision }
    }

    /// Match a pinned placement without loading a different revision by name or index.
    fn matches(&self, fp: &Footprint) -> bool {
        self.library == fp.library && self.id == fp.definition && self.revision == fp.definition_revision
    }
}

/// Look up the native counter ID; neither a component ID nor a pad name can replace it.
pub(super) fn footprint(board: &Board, id: u64) -> Option<&Footprint> {
    let footprints = board.footprints();
    footprints.binary_search_by_key(&id, |fp| fp.id).ok().map(|i| &footprints[i])
}

/// Resolve the exact pad record from the currently pinned immutable native definition.
fn pad<'a>(board: &'a Board, fp: &Footprint, number: &str) -> Option<&'a RectPad> {
    board.definition_for(fp)?.pads.iter().find(|p| p.number == number)
}

/// A courtyard edge needs a sole graphic rectangle, no courtyard artwork, and exact quarter turns.
fn courtyard<'a>(board: &'a Board, fp: &Footprint) -> Option<(usize, &'a GraphicRect)> {
    let def = board.definition_for(fp)?;
    if !fp.rotation.is_multiple_of(90) || def.artwork.iter().any(|a| a.purpose == GraphicPurpose::Courtyard) {
        return None;
    }
    let mut rects = def.graphics.iter().enumerate().filter(|(_, rect)| rect.purpose == GraphicPurpose::Courtyard);
    let (index, rect) = rects.next()?;
    (rects.next().is_none() && rect.rotation.is_multiple_of(90)).then_some((index, rect))
}

/// Select the pad-local side and trim its endpoints to the finite straight flat.
fn segment(corners: [Point; 4], side: Side, radius: i64) -> Option<Witness> {
    let (a, b) = match side {
        Side::Left => (corners[0], corners[3]),
        Side::Right => (corners[1], corners[2]),
        Side::Top => (corners[0], corners[1]),
        Side::Bottom => (corners[3], corners[2]),
    };
    let dx = b.x_nm - a.x_nm;
    let dy = b.y_nm - a.y_nm;
    if dx.abs() + dy.abs() <= 2 * radius || (dx != 0 && dy != 0) {
        return None;
    }
    let (x, y) = (dx.signum() * radius, dy.signum() * radius);
    Some(Witness::Segment { a: Point { x_nm: a.x_nm + x, y_nm: a.y_nm + y }, b: Point { x_nm: b.x_nm - x, y_nm: b.y_nm - y } })
}

/// Resolve strictly on the live snapshot, without any saved witness or fallback search.
pub(super) fn resolve(project: &Project, reference: &Reference) -> Option<Witness> {
    let Some((id, key)) = reference.footprint() else {
        let at = project.board_view(project.board_target()?)?.placement_origin().ok()?.at;
        return Some(Witness::Point { at });
    };
    let board = &project.board;
    let fp = footprint(board, id)?;
    if !key.matches(fp) {
        return None;
    }
    let corners = |p: &RectPad| rectangle_points(fp, p.center, p.width_nm, p.height_nm, p.rotation);
    match reference {
        Reference::Origin { .. } => Some(Witness::Point { at: Point { x_nm: fp.x_nm, y_nm: fp.y_nm } }),
        Reference::PadCenter { pad, .. } => Some(Witness::Point { at: fp.pad_position(PinNumber::new(pad)?, board)? }),
        Reference::PadCorner { pad: number, corner, .. } => {
            let p = pad(board, fp, number)?;
            let index = match corner {
                Corner::Ll => 3,
                Corner::Lr => 2,
                Corner::Ur => 1,
                Corner::Ul => 0,
            };
            (p.shape == PadShape::Rectangle).then(|| Witness::Point { at: corners(p)[index] })
        }
        Reference::PadEdge { pad: number, side, .. } => {
            let p = pad(board, fp, number)?;
            if !fp.rotation.is_multiple_of(90)
                || !p.rotation.is_multiple_of(90)
                || !matches!(p.shape, PadShape::Rectangle | PadShape::RoundedRectangle)
            {
                return None;
            }
            segment(corners(p), *side, p.corner_radius_nm)
        }
        Reference::CourtyardEdge { rect, side, .. } => {
            let (index, graphic) = courtyard(board, fp)?;
            if *rect != index {
                return None;
            }
            segment(rectangle_points(fp, graphic.center, graphic.width_nm, graphic.height_nm, graphic.rotation), *side, 0)
        }
        Reference::BoardOrigin {} => None,
    }
}

/// List exactly the closed offered feature tags; calling the resolver keeps eligibility in one place.
pub(super) fn features(project: &Project, fp: &Footprint) -> Vec<(Reference, Witness)> {
    let id = fp.id;
    let def = Definition::of(fp);
    let mut refs = vec![Reference::Origin { fp: id, def: def.clone() }];
    let sides = [Side::Left, Side::Right, Side::Top, Side::Bottom];
    for number in project.board.pad_numbers(fp) {
        let pad = number.as_str().to_owned();
        refs.push(Reference::PadCenter { fp: id, pad: pad.clone(), def: def.clone() });
        refs.extend([Corner::Ll, Corner::Lr, Corner::Ur, Corner::Ul].map(|corner| Reference::PadCorner {
            fp: id,
            pad: pad.clone(),
            def: def.clone(),
            corner,
        }));
        refs.extend(sides.map(|side| Reference::PadEdge { fp: id, pad: pad.clone(), def: def.clone(), side }));
    }
    if let Some((rect, _)) = courtyard(&project.board, fp) {
        refs.extend(sides.map(|side| Reference::CourtyardEdge { fp: id, def: def.clone(), rect, side }));
    }
    refs.into_iter().filter_map(|reference| Some((reference.clone(), resolve(project, &reference)?))).collect()
}

fn edge_axis(w: Witness) -> Option<Axis> {
    match w {
        Witness::Segment { a, b } if a.x_nm == b.x_nm && a.y_nm != b.y_nm => Some(Axis::X),
        Witness::Segment { a, b } if a.y_nm == b.y_nm && a.x_nm != b.x_nm => Some(Axis::Y),
        _ => None,
    }
}

impl Witness {
    /// Points project directly; for a parallel edge either endpoint gives its constant coordinate.
    fn on_axis(self, axis: Axis) -> i64 {
        let (Self::Point { at } | Self::Segment { a: at, .. }) = self;
        match axis {
            Axis::X => at.x_nm,
            Axis::Y => at.y_nm,
        }
    }
}

/// Measure on native board axes, always second pick minus first; winding plays no role.
/// Callers must go through this live-resolution entry; saved witnesses are not accepted.
pub(super) fn measure(project: &Project, kind: Kind, refs: &[Reference; 2]) -> Option<Measure> {
    if !kind.accepts(refs) {
        return None;
    }
    let witnesses = [resolve(project, &refs[0])?, resolve(project, &refs[1])?];
    let axis = match kind {
        Kind::Horizontal | Kind::AlignV => Axis::X,
        Kind::Vertical | Kind::AlignH => Axis::Y,
        Kind::PointEdge => edge_axis(witnesses[0]).or_else(|| edge_axis(witnesses[1]))?,
        Kind::Gap => {
            let axis = edge_axis(witnesses[0])?;
            if edge_axis(witnesses[1])? != axis {
                return None;
            }
            axis
        }
        Kind::Aligned => {
            let [Witness::Point { at: a }, Witness::Point { at: b }] = witnesses else { return None };
            return Some(Measure::Aligned { distance_nm: ((b.x_nm - a.x_nm) as f64).hypot((b.y_nm - a.y_nm) as f64), witnesses });
        }
    };
    Some(Measure::Projection { axis, signed_nm: witnesses[1].on_axis(axis) - witnesses[0].on_axis(axis), witnesses })
}

impl Measure {
    /// The native resolver's geometry, in pick order, for a live label or future lost display.
    pub(super) fn witnesses(self) -> [Witness; 2] {
        match self {
            Self::Projection { witnesses, .. } | Self::Aligned { witnesses, .. } => witnesses,
        }
    }
}

/// Compare physical pad semantics and terminal mapping independently of position and rotation.
/// An explicit re-pick (not correspondence) is required to adopt a changed land meaning.
fn same_pad(previous: &Project, candidate: &Project, a: &Footprint, b: &Footprint, number: &str) -> bool {
    let Some(pin) = PinNumber::new(number) else { return false };
    let old = pad(&previous.board, a, number);
    let new = pad(&candidate.board, b, number);
    let physical = match (old, new) {
        (Some(p), Some(q)) => {
            let layers = |project: &Project, copper: CopperBinding, bottom| CopperLayers::resolve(project.board.setup.as_ref(), copper, bottom);
            (p.shape, p.width_nm, p.height_nm, p.corner_radius_nm, p.copper, p.drill_nm)
                == (q.shape, q.width_nm, q.height_nm, q.corner_radius_nm, q.copper, q.drill_nm)
                && layers(previous, p.copper, a.bottom) == layers(candidate, q.copper, b.bottom)
        }
        (None, None) => a.library.is_none() && b.library.is_none(),
        _ => false,
    };
    physical
        && a.component_pin(pin) == b.component_pin(pin)
        && a.unconnected_pads.iter().any(|p| p == number) == b.unconnected_pads.iter().any(|p| p == number)
}

/// On a pin revision transition only, propose the same offered feature on the same placement.
/// An unchanged exact revision still permits native transforms, including bottom-side changes.
pub(super) fn correspondence(previous: &Project, candidate: &Project, old: &Reference) -> Option<Reference> {
    let Some((id, key)) = old.footprint() else { return resolve(candidate, old).map(|_| old.clone()) };
    let after = footprint(&candidate.board, id)?;
    let before = footprint(&previous.board, id)?;
    if !key.matches(before) || before.component != after.component {
        return None;
    }
    // Even with an unchanged definition, a package-map change can repurpose a land.
    if let Some(number) = old.pad() {
        let pin = PinNumber::new(number)?;
        if before.component_pin(pin) != after.component_pin(pin)
            || before.unconnected_pads.iter().any(|p| p == number) != after.unconnected_pads.iter().any(|p| p == number)
        {
            return None;
        }
    }
    if key.matches(after) {
        return resolve(candidate, old).map(|_| old.clone());
    }
    if old.pad().is_some_and(|number| !same_pad(previous, candidate, before, after, number)) {
        return None;
    }
    let mut next = old.clone();
    match &mut next {
        Reference::Origin { def, .. } | Reference::PadCenter { def, .. } | Reference::PadCorner { def, .. } | Reference::PadEdge { def, .. } => {
            *def = Definition::of(after);
        }
        Reference::CourtyardEdge { rect, def, .. } => {
            let (index, _) = courtyard(&candidate.board, after)?;
            if *rect != index || courtyard(&previous.board, before)?.0 != index {
                return None;
            }
            *def = Definition::of(after);
        }
        Reference::BoardOrigin {} => return None,
    }
    resolve(candidate, &next).map(|_| next)
}
