//! Compare physical pad geometry at the existing placement, without applying a revision.
use super::{
    Board, Footprint,
    clearance::index::Tree,
    definition::{CopperBinding, FootprintDefinition, RectPad},
    geometry::Contour,
    setup::{BoardSetup, Side},
    view::{CopperLayers, rectangle_points},
};
use crate::{CommandError, schematic::Point};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

// This is a reply budget, not a board capacity. Each compact endpoint row uses less than
// 256 bytes; the two definitions together contribute at most 256 lands (each with up to
// 64 layer IDs). That leaves headroom for metadata and the MCP JSON text wrapper.
const MAX_LOOSE_ENDS: usize = 10_240;

/// Deterministic pad rows with no inferred mapping for added or removed identifiers.
#[derive(Serialize)]
pub(crate) struct Delta<'a> {
    pad_numbers_changed: bool,
    pads: Vec<PadDelta<'a>>,
}

#[derive(Serialize)]
struct PadDelta<'a> {
    number: &'a str,
    before: Option<Land<'a>>,
    after: Option<Land<'a>>,
    changes: Vec<Change>,
    center_shift_nm: Option<f64>,
    loose_ends: Vec<LooseEnd>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Change {
    Added,
    Removed,
    Moved,
    Resized,
    Rotated,
    Shape,
    CopperLayers,
    Drill,
    Mask,
    Paste,
}

#[derive(Serialize)]
struct Land<'a> {
    geometry: &'a RectPad,
    board_center: Point,
    copper_layers: CopperLayers,
    mask_sides: &'static [Side],
    paste_sides: &'static [Side],
    #[serde(skip)]
    contour: Contour,
}

impl<'a> Land<'a> {
    /// Project the copper outline by the board's existing mirroring and rotation rules.
    /// Mask and paste describe nominal pad openings, not user artwork or process expansion.
    fn new(pad: &'a RectPad, placement: &Footprint, setup: Option<&BoardSetup>) -> Self {
        let center = placement.point(pad.center);
        let corners = rectangle_points(placement, pad.center, pad.width_nm, pad.height_nm, pad.rotation);
        let mounting: &[Side] = if placement.bottom { &[Side::Back] } else { &[Side::Front] };
        let (mask_sides, paste_sides): (&[Side], &[Side]) = match pad.copper {
            CopperBinding::ComponentSide => (mounting, mounting),
            CopperBinding::AllCopper => (&[Side::Front, Side::Back], &[]),
        };
        Self {
            geometry: pad,
            board_center: center,
            copper_layers: CopperLayers::resolve(setup, pad.copper, placement.bottom),
            mask_sides,
            paste_sides,
            contour: Contour::pad(center, corners, pad.shape, pad.corner_radius_nm),
        }
    }

    /// A through via spans all configured copper, while a track end requires its own layer.
    fn shares_layer(&self, layer: Option<Uuid>) -> bool {
        match &self.copper_layers {
            CopperLayers::Unconfigured { .. } => false,
            CopperLayers::Resolved { layers, .. } => layer.map_or(!layers.is_empty(), |layer| layers.contains(&layer)),
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum EndKind {
    TrackStart,
    TrackEnd,
    Via,
}

#[derive(Serialize)]
struct LooseEnd {
    kind: EndKind,
    id: Uuid,
    layer: Option<Uuid>,
    at: Point,
    gap_nm: Option<f64>,
}

struct End {
    kind: EndKind,
    id: Uuid,
    layer: Option<Uuid>,
    at: Point,
    radius: f64,
}

impl End {
    /// Test only the endpoint cap, not the segment shaft; drill voids use the existing copper contour convention.
    fn contour(&self) -> Contour {
        Contour::capsule(self.at, self.at, self.radius)
    }

    /// Keep a lost old contact only. A missing same-number pad or missing layer has no gap.
    fn lost_contact(&self, before: &Land<'_>, after: Option<&Land<'_>>) -> Option<LooseEnd> {
        if !before.shares_layer(self.layer) || !before.contour.overlaps_segment(self.at, self.at, self.radius) {
            return None;
        }
        let gap_nm = after.filter(|land| land.shares_layer(self.layer)).map(|land| land.contour.gap(&self.contour()));
        if gap_nm.is_some_and(|gap| gap <= 0.) {
            return None;
        }
        Some(LooseEnd { kind: self.kind, id: self.id, layer: self.layer, at: self.at, gap_nm })
    }
}

struct Ends {
    copper: Vec<End>,
    index: Tree,
}

impl Ends {
    /// Index only straight track caps and through vias. The shared bounds lookup prunes
    /// unrelated copper without a net or DRC pass; standalone arc ends are outside this preview.
    fn new(board: &Board) -> Self {
        let copper: Vec<_> = board
            .tracks
            .iter()
            .flat_map(|track| {
                [(EndKind::TrackStart, track.start), (EndKind::TrackEnd, track.end)].map(|(kind, at)| End {
                    kind,
                    id: track.id,
                    layer: Some(track.layer),
                    at,
                    radius: track.width_nm as f64 / 2.,
                })
            })
            .chain(board.vias.iter().map(|via| End { kind: EndKind::Via, id: via.id, layer: None, at: via.at, radius: via.diameter_nm as f64 / 2. }))
            .collect();
        let index = Tree::new(copper.iter().map(|end| end.contour().bounds()).collect());
        Self { copper, index }
    }
}

/// Compare exact pad numbers; renumbering appears as an addition and a removal, not an inferred
/// pairing. Visit changed old lands only and do not move endpoints or report net connectivity.
/// Refuse over-budget replies without returning a partial result, even for a single pad.
pub(crate) fn compare<'a>(
    board: &Board,
    placement: &Footprint,
    pinned: &'a FootprintDefinition,
    candidate: &'a FootprintDefinition,
    pad_filter: Option<&str>,
) -> Result<Delta<'a>, CommandError> {
    let old: BTreeMap<_, _> = pinned.pads.iter().map(|pad| (pad.number.as_str(), pad)).collect();
    let new: BTreeMap<_, _> = candidate.pads.iter().map(|pad| (pad.number.as_str(), pad)).collect();
    let pad_numbers_changed = old.keys().ne(new.keys());
    let numbers: BTreeSet<_> = old.keys().chain(new.keys()).copied().collect();
    if pad_filter.is_some_and(|pad| !numbers.contains(pad)) {
        return Err(CommandError::NotFound("Pad number not present in either footprint revision".into()));
    }
    let setup = board.setup.as_ref();
    let mut ends = None;
    let mut count = 0;
    let mut pads = Vec::new();
    for number in numbers.into_iter().filter(|&number| pad_filter.is_none_or(|pad| pad == number)) {
        let old_pad = old.get(number).copied();
        let new_pad = new.get(number).copied();
        if old_pad == new_pad {
            continue;
        }
        let before = old_pad.map(|pad| Land::new(pad, placement, setup));
        let after = new_pad.map(|pad| Land::new(pad, placement, setup));
        let (changes, center_shift_nm, check_contact) = describe(before.as_ref(), after.as_ref());
        let mut loose_ends = Vec::new();
        if let Some(before) = before.as_ref().filter(|_| check_contact && setup.is_some()) {
            let ends = ends.get_or_insert_with(|| Ends::new(board));
            for index in ends.index.candidates(before.contour.bounds(), 0.) {
                if let Some(loose) = ends.copper[index].lost_contact(before, after.as_ref()) {
                    if count == MAX_LOOSE_ENDS {
                        let recovery = if pad_filter.is_some() {
                            "This pad alone exceeds the limit; inspect its copper in the board editor."
                        } else {
                            "To narrow the preview, request one pad by number using the pad parameter."
                        };
                        return Err(CommandError::CapExceeded(format!(
                            "Preview exceeds the {MAX_LOOSE_ENDS} loose-end row limit; no partial report was returned. {recovery}"
                        )));
                    }
                    count += 1;
                    loose_ends.push(loose);
                }
            }
        }
        pads.push(PadDelta { number, before, after, changes, center_shift_nm, loose_ends });
    }
    Ok(Delta { pad_numbers_changed, pads })
}

/// Compare the resolved convex cores without depending on their starting corner or winding.
/// Equal radii then give the same outline, including point and segment cores.
fn same_outline(a: &Contour, b: &Contour) -> bool {
    let corners = |outline: &Contour| {
        let mut points = outline.core.map(|point| (point.x_nm, point.y_nm));
        points.sort_unstable();
        points
    };
    a.radius.to_bits() == b.radius.to_bits() && corners(a) == corners(b)
}

/// Distinguish copper-affecting edits from drill-only edits, and report changes in the nominal
/// apertures. Native pads do not have independent mask or paste adjustments.
fn describe(before: Option<&Land<'_>>, after: Option<&Land<'_>>) -> (Vec<Change>, Option<f64>, bool) {
    let (Some(before), Some(after)) = (before, after) else {
        return if before.is_some() { (vec![Change::Removed], None, true) } else { (vec![Change::Added], None, false) };
    };
    let a = before.geometry;
    let b = after.geometry;
    let moved = a.center != b.center;
    let resized = (a.width_nm, a.height_nm) != (b.width_nm, b.height_nm);
    let rotated = a.rotation != b.rotation;
    let shape = (a.shape, a.corner_radius_nm) != (b.shape, b.corner_radius_nm);
    // Odd dimensions can move an integer edge by a nanometer even at an apparently
    // symmetric angle. The projected contour is also the one used for contact tests.
    let profile = !same_outline(&before.contour, &after.contour);
    let layers = a.copper != b.copper;
    let mask = profile || before.mask_sides != after.mask_sides;
    let paste = (profile && (!before.paste_sides.is_empty() || !after.paste_sides.is_empty())) || before.paste_sides != after.paste_sides;
    let changes = [
        (moved, Change::Moved),
        (resized, Change::Resized),
        (rotated, Change::Rotated),
        (shape, Change::Shape),
        (layers, Change::CopperLayers),
        (a.drill_nm != b.drill_nm, Change::Drill),
        (mask, Change::Mask),
        (paste, Change::Paste),
    ]
    .into_iter()
    .filter_map(|(changed, kind)| changed.then_some(kind))
    .collect();
    let shift = ((after.board_center.x_nm - before.board_center.x_nm) as f64).hypot((after.board_center.y_nm - before.board_center.y_nm) as f64);
    (changes, Some(shift), profile || layers)
}
