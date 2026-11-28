//! Copper pours in the Gerber copper layers. Each island of the engine's current analytic zone
//! fill, the same material DRC checks, becomes one dark-polarity region (`G36`/`G37`,
//! Ucamco Gerber 2026.05 §4.10). Nothing is refilled, offset or flattened here.
//!
//! Contours follow the fill's exact line and circular-arc boundary. Every vertex and every arc
//! centre is rounded once to the file's 1 nm resolution (format 4.6 mm). Arcs use `G75`
//! multi-quadrant circular interpolation (§4.7). They are split at the quarter-turn angles, so
//! no written arc exceeds 90° and every extreme point of a loop is a vertex. A piece whose
//! rounded end points are less than 1 + √2 µm (about 2.41 µm) apart is written as a line:
//! §4.14.2 lets processing move each coordinate by up to 0.5 µm, so each end point by up to
//! √2/2 µm, and an arc whose end points come closer than 1 µm can turn into a nearly full circle.
//!
//! Holes, the clearances around other copper, are joined to their outer contour with
//! horizontal cut-ins: pairs of fully-coincident linear segments (§§4.10.3, 4.10.4.7). A contour
//! where one would be tangent to an arc, which §4.10 forbids, takes all its cut-ins along another
//! axis direction. No clear polarity is used, so the image does not depend on the order of
//! objects in the file.
//!
//! Conversion error from the analytic boundary, per written segment: a line is within √2/2 nm
//! (its rounded end points). An arc is within max(|r₁ − r|, |r₂ − r|) + |c′ − c|, where r and c
//! are the analytic radius and centre, c′ the written centre and r₁, r₂ the written start and end
//! radii: the ring that §4.7.2 allows any rendering of the arc to occupy. The written centre is
//! the one of the nine nanometre points around c that minimises this bound, which therefore
//! never exceeds 3√2/2 nm (about 2.1 nm). A short piece written as a line adds its sagitta;
//! a contour whose bound exceeds [`MAX_DEVIATION_NM`] is refused rather than written.
//! [`Stats::deviation_nm`] reports the largest bound of one export.

use super::{ApertureFunction, DELETE_APERTURE_FUNCTION, Gerber, object_attributes::RoutedNets};
use crate::board::{fill::ProblemKind, origin::Frame, view::BoardView};
use pcbcraft_geometry::planar::{Boundary, Edge};
use std::{collections::BTreeMap, f64::consts::FRAC_PI_2, fmt::Write as _};
use uuid::Uuid;

/// Largest distance a written segment may lie from the fill's analytic boundary: one fifth of
/// the ±0.5 µm a coordinate may move in processing (§4.14.2). Only a short arc written as a line
/// can come near it; a contour beyond it is refused.
const MAX_DEVIATION_NM: f64 = 100.;

/// Shortest chord written as a circular arc: 1 µm plus the √2 µm by which the §4.14.2
/// perturbation (±0.5 µm per coordinate of each end point) can shorten it (Ucamco Gerber 2026.05).
const MIN_ARC_CHORD_NM: f64 = 1_000. * (1. + std::f64::consts::SQRT_2);

/// A written coordinate pair in output nanometres.
pub(super) type Vertex = (i64, i64);

/// How a written segment reaches the start of the next segment of its contour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Shape {
    Line,
    /// A circular arc about the written `center`; `circle` keeps the analytic centre and radius.
    Arc {
        center: Vertex,
        clockwise: bool,
        circle: ([f64; 2], f64),
    },
}

/// One written segment from `start` to the start of the next segment. `exact` is the analytic
/// start point it was rounded from (equal to `start` for cut-ins).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Segment {
    pub(super) start: Vertex,
    pub(super) exact: [f64; 2],
    pub(super) shape: Shape,
}

/// Measurements of one conversion; the module documentation derives the deviation bound.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Stats {
    /// The largest bound, in nanometres, of any written segment from the analytic boundary.
    pub(super) deviation_nm: f64,
    /// Written contour segments, cut-ins included.
    pub(super) segments: usize,
    /// Written circular arcs.
    pub(super) arcs: usize,
    /// Cut-in pairs joining holes to their contour.
    pub(super) cut_ins: usize,
}

/// One analytic boundary piece in output coordinates before rounding: its start and, for an
/// arc, its centre, radius and direction.
#[derive(Clone, Copy)]
struct Piece {
    start: [f64; 2],
    arc: Option<([f64; 2], f64, bool)>,
}

/// One filled island of one zone: its net and one closed contour for each of its regions.
pub(super) struct Pour {
    net: Uuid,
    pub(super) contours: Vec<Vec<Segment>>,
}

/// The pour regions of one export by copper layer, each layer in the fill's zone order.
#[derive(Default)]
pub(super) struct Pours {
    layers: BTreeMap<Uuid, Vec<Pour>>,
    pub(super) stats: Stats,
}

impl Pours {
    /// Convert the board's current fill. A board without zones has no pours. Refuses, without
    /// writing anything, when pours were removed, when the fill for the current board is still
    /// pending or failed, or when a zone produced no fill because its net is unavailable: a
    /// stale or missing pour is never exported as if it were current. A zone whose fill cannot be
    /// written safely is refused with the description `name` gives from its net and layer.
    pub(super) fn new(view: BoardView<'_>, frame: Frame, name: impl Fn(Uuid, Uuid) -> String) -> Result<Self, String> {
        if view.zones().is_empty() {
            return Ok(Self::default());
        }
        if !view.zone_fill_enabled() {
            return Err("Copper pours are removed. Use Pour copper to fill the zones before generating fabrication outputs.".into());
        }
        let fill = match view.filled_zones() {
            Some(Ok(fill)) if fill.board == view.target => fill,
            Some(Err(message)) => {
                return Err(format!(
                    "Copper zones could not be poured: {message} Use Pour copper to try again. No fabrication files were generated."
                ));
            }
            _ => {
                return Err(
                    "Copper zones are not poured for the current board yet. Use Pour copper and let it finish before generating fabrication outputs."
                        .into(),
                );
            }
        };
        if let Some(problem) = fill.problems.iter().find(|problem| matches!(problem.kind, ProblemKind::NetUnavailable | ProblemKind::Unavailable)) {
            return Err(format!("A copper zone has no fill: {} No fabrication files were generated.", problem.message));
        }
        let mut pours = Self::default();
        for zone in &fill.zones {
            let refuse =
                |reason: String| format!("Cannot write the {} to Gerber: {reason} No fabrication files were generated.", name(zone.net, zone.layer));
            for island in &zone.islands {
                let boundaries = island.region.boundaries().map_err(|error| refuse(error.to_string()))?;
                let contours =
                    boundaries.iter().map(|boundary| contour(boundary, frame, &mut pours.stats)).collect::<Result<_, _>>().map_err(refuse)?;
                pours.layers.entry(zone.layer).or_default().push(Pour { net: zone.net, contours });
            }
        }
        Ok(pours)
    }

    /// Write this layer's pours, each with the zone net's object attribute.
    pub(super) fn write(&self, gerber: &mut Gerber, layer: Uuid, nets: &RoutedNets) {
        for pour in self.layers.get(&layer).into_iter().flatten() {
            gerber.object(nets.zone(pour.net), |gerber| region(&mut gerber.body, &pour.contours));
        }
    }
}

/// Round half up, so equal fractions round alike everywhere in the output frame.
fn round(value: f64) -> i64 {
    (value + 0.5).floor() as i64
}

fn vertex([x, y]: [f64; 2]) -> Vertex {
    (round(x), round(y))
}

fn float((x, y): Vertex) -> [f64; 2] {
    [x as f64, y as f64]
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Write one region's boundary as a single closed contour: the outer loop counter-clockwise,
/// every hole clockwise and joined to it by a cut-in. Returns why when a loop collapses at the
/// output resolution, a hole cannot be joined or the written boundary would lie more than
/// [`MAX_DEVIATION_NM`] from the fill.
pub(super) fn contour(boundary: &Boundary, frame: Frame, stats: &mut Stats) -> Result<Vec<Segment>, String> {
    let tolerated = |own: &Stats| {
        if own.deviation_nm <= MAX_DEVIATION_NM {
            return Ok(());
        }
        Err(format!(
            "A short arc written as a line would lie {:.0} nm from the fill, more than the {MAX_DEVIATION_NM:.0} nm tolerance.",
            own.deviation_nm
        ))
    };
    let mut own = Stats::default();
    let outer = oriented(written(&pieces(&boundary.outer, frame), &mut own)?, true)?;
    let holes = boundary.holes.iter().map(|hole| oriented(written(&pieces(hole, frame), &mut own)?, false)).collect::<Result<Vec<_>, _>>()?;
    // The loops' own lines are checked first; cut-ins can only split arcs further.
    tolerated(&own)?;
    let contour = bridge(outer, holes, &mut own)?;
    tolerated(&own)?;
    stats.deviation_nm = stats.deviation_nm.max(own.deviation_nm);
    stats.segments += contour.len();
    stats.arcs += contour.iter().filter(|segment| segment.shape != Shape::Line).count();
    stats.cut_ins += own.cut_ins;
    Ok(contour)
}

/// Express one analytic loop in output coordinates (Y up, so arcs change direction) and split
/// every arc at the quarter-turn angles strictly inside its sweep, in travel order.
fn pieces(edges: &[Edge], frame: Frame) -> Vec<Piece> {
    let mut pieces = Vec::with_capacity(edges.len());
    for edge in edges {
        match *edge {
            Edge::Line { start_nm } => pieces.push(Piece { start: frame.to_output_nm(start_nm), arc: None }),
            Edge::Arc { start_nm, center_nm, radius_nm, sweep_rad } => {
                let (start, center, sweep) = (frame.to_output_nm(start_nm), frame.to_output_nm(center_nm), -sweep_rad);
                let clockwise = sweep < 0.;
                let arc = Some((center, radius_nm, clockwise));
                pieces.push(Piece { start, arc });
                let from = (start[1] - center[1]).atan2(start[0] - center[0]);
                let (low, high) = if clockwise { (from + sweep, from) } else { (from, from + sweep) };
                let quarters = ((low / FRAC_PI_2).floor() as i64 + 1..).take_while(|&k| (k as f64) * FRAC_PI_2 < high);
                let mut splits: Vec<_> = quarters
                    .map(|k| {
                        // Quarter-turn points are exact: centre plus or minus the radius on one axis.
                        let (dx, dy) = [(1., 0.), (0., 1.), (-1., 0.), (0., -1.)][k.rem_euclid(4) as usize];
                        Piece { start: [center[0] + dx * radius_nm, center[1] + dy * radius_nm], arc }
                    })
                    .collect();
                if clockwise {
                    splits.reverse();
                }
                pieces.extend(splits);
            }
        }
    }
    pieces
}

/// Round one loop. Pieces whose end points round together are dropped, and each remaining arc
/// gets its written centre. Fewer than two remaining segments is a collapsed loop.
fn written(pieces: &[Piece], stats: &mut Stats) -> Result<Vec<Segment>, String> {
    let starts: Vec<_> = pieces.iter().map(|piece| vertex(piece.start)).collect();
    let n = pieces.len();
    let kept: Vec<_> = (0..n).filter(|&i| starts[i] != starts[(i + 1) % n]).collect();
    if kept.len() < 2 {
        return Err("A copper zone island is too small to write at the 1 nm Gerber resolution.".into());
    }
    Ok(kept
        .iter()
        .enumerate()
        .map(|(j, &i)| {
            let next = kept[(j + 1) % kept.len()];
            let shape = shape(starts[i], starts[next], pieces[i].arc, stats);
            if shape == Shape::Line {
                // A written line only moves by the rounding of its two end points.
                let rounding = distance(float(starts[i]), pieces[i].start).max(distance(float(starts[next]), pieces[next].start));
                stats.deviation_nm = stats.deviation_nm.max(rounding);
            }
            Segment { start: starts[i], exact: pieces[i].start, shape }
        })
        .collect())
}

/// Choose how a written segment from `start` to `end` follows its analytic piece. An arc keeps
/// the circular interpolation unless its chord is under [`MIN_ARC_CHORD_NM`]; its written centre minimises the
/// deviation bound over the nine nanometre points around the analytic centre (ties keep the
/// first in a fixed order, so the choice is deterministic). Records the bound in `stats`.
fn shape(start: Vertex, end: Vertex, arc: Option<([f64; 2], f64, bool)>, stats: &mut Stats) -> Shape {
    let Some((center, radius, clockwise)) = arc else { return Shape::Line };
    let (a, b) = (float(start), float(end));
    let chord = distance(a, b);
    if chord < MIN_ARC_CHORD_NM {
        let sagitta = radius - (radius * radius - chord * chord / 4.).max(0.).sqrt();
        stats.deviation_nm = stats.deviation_nm.max(sagitta + std::f64::consts::FRAC_1_SQRT_2);
        return Shape::Line;
    }
    let base = vertex(center);
    let mut best = (base, f64::INFINITY);
    for dx in -1..=1 {
        for dy in -1..=1 {
            let candidate = (base.0 + dx, base.1 + dy);
            let c = float(candidate);
            let bound = (distance(a, c) - radius).abs().max((distance(b, c) - radius).abs()) + distance(c, center);
            if bound < best.1 {
                best = (candidate, bound);
            }
        }
    }
    stats.deviation_nm = stats.deviation_nm.max(best.1);
    Shape::Arc { center: best.0, clockwise, circle: (center, radius) }
}

/// Signed area of a written loop: the polygon through its vertices plus, for each arc, the
/// circular segment between its chord and the arc (positive counter-clockwise).
fn area(segments: &[Segment]) -> f64 {
    let n = segments.len();
    (0..n)
        .map(|i| {
            let (a, b) = (float(segments[i].start), float(segments[(i + 1) % n].start));
            let chord = (a[0] * b[1] - a[1] * b[0]) / 2.;
            let Shape::Arc { center, clockwise, .. } = segments[i].shape else { return chord };
            let c = float(center);
            let (u, v) = ([a[0] - c[0], a[1] - c[1]], [b[0] - c[0], b[1] - c[1]]);
            let mut theta = (u[0] * v[1] - u[1] * v[0]).atan2(u[0] * v[0] + u[1] * v[1]).abs();
            if clockwise {
                theta = -theta;
            }
            let r = (distance(a, c) + distance(b, c)) / 2.;
            chord + r * r / 2. * (theta - theta.sin())
        })
        .sum()
}

/// Orient a written loop counter-clockwise for material or clockwise for a hole, so a cut-in
/// keeps the material on one side (§4.10.3). A loop without area is refused.
fn oriented(segments: Vec<Segment>, material: bool) -> Result<Vec<Segment>, String> {
    let area = area(&segments);
    if area == 0. || !area.is_finite() {
        return Err("A copper zone contour has no area at the 1 nm Gerber resolution.".into());
    }
    if (area > 0.) == material {
        return Ok(segments);
    }
    let n = segments.len();
    Ok((0..n)
        .rev()
        .map(|i| {
            let next = segments[(i + 1) % n];
            let shape = match segments[i].shape {
                Shape::Arc { center, clockwise, circle } => Shape::Arc { center, clockwise: !clockwise, circle },
                Shape::Line => Shape::Line,
            };
            Segment { start: next.start, exact: next.exact, shape }
        })
        .collect())
}

/// Where a cut-in from a hole meets another loop.
enum Hit {
    /// At the start of the segment with this index.
    Vertex(usize),
    /// Inside the segment with this index, at this analytic X on the cut-in's line.
    Inside(usize, f64),
}

/// A hole joined to a segment of another loop: the cut-in leaves the segment at `h`, `along`
/// its travel from its start, and runs to the hole's rightmost vertex.
struct Join {
    segment: usize,
    along: f64,
    h: Vertex,
    exact: [f64; 2],
    hole: usize,
}

/// A cut-in closer to tangent than this, measured as the offset of its end from the arc
/// centre's line across the cut-in, could become tangent when the §4.14.2 perturbation moves the
/// end and the centre by up to 0.5 µm each across it.
const MIN_TANGENT_OFFSET_NM: i64 = 1_000;

/// The four ray directions cut-ins may take, as maps of the output frame in which the ray runs
/// in +X: unchanged, mirrored in X (rays in −X), mirrored about the diagonal (rays in +Y) and
/// turned a quarter clockwise (rays in −Y). A mirror reverses the direction of every arc.
#[derive(Clone, Copy)]
enum Turn {
    Same,
    MirrorX,
    Diagonal,
    Quarter,
}

impl Turn {
    const ALL: [Self; 4] = [Self::Same, Self::MirrorX, Self::Diagonal, Self::Quarter];

    /// Map a point into this frame, or back out of it with `back`.
    fn map<T: Copy + std::ops::Neg<Output = T>>(self, [x, y]: [T; 2], back: bool) -> [T; 2] {
        match (self, back) {
            (Self::Same, _) => [x, y],
            (Self::MirrorX, _) => [-x, y],
            (Self::Diagonal, _) => [y, x],
            (Self::Quarter, false) => [-y, x],
            (Self::Quarter, true) => [y, -x],
        }
    }

    fn segment(self, segment: &Segment, back: bool) -> Segment {
        let vertex = |(x, y): Vertex| {
            let [x, y] = self.map([x, y], back);
            (x, y)
        };
        let shape = match segment.shape {
            Shape::Line => Shape::Line,
            Shape::Arc { center, clockwise, circle: (c, r) } => Shape::Arc {
                center: vertex(center),
                clockwise: clockwise != matches!(self, Self::MirrorX | Self::Diagonal),
                circle: (self.map(c, back), r),
            },
        };
        Segment { start: vertex(segment.start), exact: self.map(segment.exact, back), shape }
    }
}

/// Join every hole to the outer contour with cut-ins all in one direction, trying the next
/// direction when one would leave a cut-in tangent to an arc, too close to its own hole or within
/// 1 µm of another edge, and refusing with the first direction's reason when none works.
fn bridge(outer: Vec<Segment>, holes: Vec<Vec<Segment>>, stats: &mut Stats) -> Result<Vec<Segment>, String> {
    let mut loops = vec![outer];
    loops.extend(holes);
    if loops.len() == 1 {
        return bridge_in(loops, stats);
    }
    let mut first = None;
    for turn in Turn::ALL {
        let turned = loops.iter().map(|segments| segments.iter().map(|segment| turn.segment(segment, false)).collect()).collect();
        match bridge_in(turned, stats) {
            Ok(contour) => return Ok(contour.iter().map(|segment| turn.segment(segment, true)).collect()),
            Err(reason) => {
                first.get_or_insert(reason);
            }
        }
    }
    Err(first.unwrap_or_default())
}

/// Whether a horizontal cut-in ending at `at` would be tangent to `segment` there, within the
/// perturbation: an arc's tangent at `at` is horizontal when `at` is above or below its centre.
fn grazes(segment: &Segment, at: Vertex) -> bool {
    matches!(segment.shape, Shape::Arc { center, .. } if (at.0 - center.0).abs() < MIN_TANGENT_OFFSET_NM)
}

/// Join every hole of `loops` (the outer loop first) to the outer contour. A cut-in runs from the
/// hole's rightmost vertex in +X to the first segment it meets. That segment is on the outer loop
/// or on a hole whose rightmost vertex lies further right, since no other loop reaches the ray;
/// so taking holes from the largest X down, cut-ins never cross holes or each other. Rays are
/// answered from horizontal bands of segments and the contour is assembled in one pass, so the
/// work grows with the number of segments, not with holes times segments, for loops spread over
/// the region. Refuses a cut-in tangent to an arc at either end (§4.10) and a contour with
/// pieces that do not meet but come within 1 µm of each other (§4.14.2).
fn bridge_in(loops: Vec<Vec<Segment>>, stats: &mut Stats) -> Result<Vec<Segment>, String> {
    let entries: Vec<usize> = loops
        .iter()
        .enumerate()
        .map(|(l, segments)| match l {
            0 => 0,
            _ => segments.iter().enumerate().max_by_key(|(_, segment)| (segment.start.0, std::cmp::Reverse(segment.start.1))).map_or(0, |(k, _)| k),
        })
        .collect();
    let bands = Bands::new(&loops);
    let mut order: Vec<usize> = (1..loops.len()).collect();
    order.sort_by_key(|&l| {
        let v = loops[l][entries[l]].start;
        (std::cmp::Reverse(v.0), v.1)
    });
    let mut joins: Vec<Vec<Join>> = loops.iter().map(|_| Vec::new()).collect();
    for hole in order {
        let v = loops[hole][entries[hole]].start;
        let (l, hit) = nearest(&loops, bands.crossing(v.1), v)?;
        let segments = &loops[l];
        let n = segments.len();
        let (segment, h, exact) = match hit {
            Hit::Vertex(k) => (k, segments[k].start, segments[k].exact),
            Hit::Inside(k, x) => match (round(x), v.1) {
                h if h == segments[k].start => (k, h, segments[k].exact),
                h if h == segments[(k + 1) % n].start => ((k + 1) % n, h, segments[(k + 1) % n].exact),
                h => (k, h, [x, v.1 as f64]),
            },
        };
        if h.0 <= v.0 {
            return Err("A copper zone hole is too close to its boundary to join at the 1 nm Gerber resolution.".into());
        }
        let (m, entry) = (loops[hole].len(), entries[hole]);
        let at_h = if h == segments[segment].start { vec![segment, (segment + n - 1) % n] } else { vec![segment] };
        if at_h.into_iter().any(|k| grazes(&segments[k], h)) || [entry, (entry + m - 1) % m].iter().any(|&k| grazes(&loops[hole][k], v)) {
            return Err("A copper zone hole cannot be joined to its boundary without a cut-in tangent to a circular edge.".into());
        }
        let along = distance(float(segments[segment].start), float(h));
        joins[l].push(Join { segment, along, h, exact, hole });
    }
    for (l, list) in joins.iter_mut().enumerate() {
        let (n, entry) = (loops[l].len(), entries[l]);
        // A quarter-turn arc's chord grows along its travel, so `along` orders points on any segment.
        list.sort_by(|a, b| ((a.segment + n - entry) % n).cmp(&((b.segment + n - entry) % n)).then(a.along.total_cmp(&b.along)));
    }
    let cut_ins: Vec<_> = joins.iter().enumerate().flat_map(|(l, list)| list.iter().map(move |join| (l, join))).collect();
    separated(
        &loops,
        cut_ins.iter().map(|&(l, join)| {
            (join.h, loops[join.hole][entries[join.hole]].start, [(l, join.segment, join.along), (join.hole, entries[join.hole], 0.)])
        }),
    )?;
    stats.cut_ins += loops.len() - 1;
    Ok(assemble(&loops, &entries, &joins, stats))
}

/// Closest distance that §4.14.2 allows between points and segments that are not coincident.
const MIN_SEPARATION_NM: f64 = 1_000.;

/// One straight or circular piece of a contour for the separation check: its end points, its
/// circle (centre, radius, clockwise) when it is an arc, how far a valid reading of the arc may
/// stray from that circle (§4.7.2), and what it belongs to.
struct Part {
    a: Vertex,
    b: Vertex,
    arc: Option<([f64; 2], f64, bool)>,
    fuzz: f64,
    owner: Owner,
}

/// A loop segment (loop, index), or a cut-in whose two ends lie on loop segments, each given as
/// (loop, index, distance along the segment from its start).
#[derive(Clone, Copy)]
enum Owner {
    Loop(usize, usize),
    Cut([(usize, usize, f64); 2]),
}

/// Check that pieces which do not meet stay 1 µm apart (§4.14.2), so no allowed perturbation
/// makes the contour cross itself. Pieces sharing an end point meet there, a cut-in meets the
/// segments its ends lie on, and pieces joined along one loop by less than 1 µm are one
/// sub-micrometre corner of the fill rather than a gap; everything else is measured. Candidate
/// pairs come from a sweep over X-sorted bounding boxes, which bound every quadrant-sized arc.
fn separated(loops: &[Vec<Segment>], cut_ins: impl Iterator<Item = (Vertex, Vertex, [(usize, usize, f64); 2])>) -> Result<(), String> {
    let mut parts = Vec::new();
    let mut along = Vec::with_capacity(loops.len());
    for (l, segments) in loops.iter().enumerate() {
        let n = segments.len();
        let mut prefix = Vec::with_capacity(n + 1);
        prefix.push(0.);
        for k in 0..n {
            let (a, b) = (segments[k].start, segments[(k + 1) % n].start);
            let (arc, fuzz) = match segments[k].shape {
                Shape::Arc { center, clockwise, .. } => {
                    let (c, ra, rb) = (float(center), distance(float(a), float(center)), distance(float(b), float(center)));
                    (Some((c, (ra + rb) / 2., clockwise)), (ra - rb).abs())
                }
                Shape::Line => (None, 0.),
            };
            parts.push(Part { a, b, arc, fuzz, owner: Owner::Loop(l, k) });
            prefix.push(prefix[k] + distance(float(a), float(b)));
        }
        along.push(prefix);
    }
    parts.extend(cut_ins.map(|(h, v, ends)| Part { a: v, b: h, arc: None, fuzz: 0., owner: Owner::Cut(ends) }));
    let boxes: Vec<_> =
        parts.iter().map(|part| [part.a.0.min(part.b.0), part.a.1.min(part.b.1), part.a.0.max(part.b.0), part.a.1.max(part.b.1)]).collect();
    let reach = MIN_SEPARATION_NM.ceil() as i64 + 1;
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by_key(|&i| boxes[i][0]);
    for (at, &i) in order.iter().enumerate() {
        for &j in order[at + 1..].iter().take_while(|&&j| boxes[j][0] <= boxes[i][2] + reach) {
            if boxes[j][1] > boxes[i][3] + reach || boxes[i][1] > boxes[j][3] + reach || meet(&parts[i], &parts[j], &along) {
                continue;
            }
            let gap = gap(&parts[i], &parts[j]);
            if gap < MIN_SEPARATION_NM + parts[i].fuzz + parts[j].fuzz {
                return Err(format!(
                    "Two of its edges that do not meet are {gap:.0} nm apart, less than the 1 µm Gerber §4.14.2 requires, so processing could make them touch."
                ));
            }
        }
    }
    Ok(())
}

/// Whether two pieces meet by construction (see [`separated`]), so their distance is not checked.
fn meet(p: &Part, q: &Part, along: &[Vec<f64>]) -> bool {
    if [p.a, p.b].iter().any(|end| *end == q.a || *end == q.b) {
        return true;
    }
    // Distance along loop `l` from the point `offset` into segment `k` to segment `j`, either way.
    let apart = |l: usize, k: usize, offset: f64, j: usize| {
        let prefix = &along[l];
        let total = prefix[prefix.len() - 1];
        let at = prefix[k] + offset;
        (prefix[j] - at).rem_euclid(total).min((at - prefix[j + 1]).rem_euclid(total))
    };
    match (p.owner, q.owner) {
        (Owner::Loop(l, i), Owner::Loop(m, j)) if l == m => {
            let (i, j) = (i.min(j), i.max(j));
            apart(l, i, along[l][i + 1] - along[l][i], j) < MIN_SEPARATION_NM
        }
        (Owner::Loop(l, j), Owner::Cut(ends)) | (Owner::Cut(ends), Owner::Loop(l, j)) => {
            ends.iter().any(|&(m, k, offset)| m == l && (k == j || apart(l, k, offset, j) < MIN_SEPARATION_NM))
        }
        _ => false,
    }
}

/// Whether `p` lies within the sweep of the arc from `a` to `b` about `c` (at most a half turn).
fn within(p: [f64; 2], c: [f64; 2], a: Vertex, b: Vertex, clockwise: bool) -> bool {
    let (u, v, w) = ([a.0 as f64 - c[0], a.1 as f64 - c[1]], [b.0 as f64 - c[0], b.1 as f64 - c[1]], [p[0] - c[0], p[1] - c[1]]);
    let (from, to) = (u[0] * w[1] - u[1] * w[0], w[0] * v[1] - w[1] * v[0]);
    if clockwise { from <= 0. && to <= 0. } else { from >= 0. && to >= 0. }
}

/// Distance from a point to a piece.
fn reach_to(p: [f64; 2], part: &Part) -> f64 {
    let (a, b) = (float(part.a), float(part.b));
    match part.arc {
        Some((c, r, clockwise)) if within(p, c, part.a, part.b, clockwise) => (distance(p, c) - r).abs(),
        Some(_) => distance(p, a).min(distance(p, b)),
        None => {
            let d = [b[0] - a[0], b[1] - a[1]];
            let t = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1])).clamp(0., 1.);
            distance(p, [a[0] + t * d[0], a[1] + t * d[1]])
        }
    }
}

/// Whether two pieces cross or touch: exact orientation tests for two lines, circle
/// intersections checked against each sweep otherwise.
fn crossing(p: &Part, q: &Part) -> bool {
    let orient =
        |a: Vertex, b: Vertex, c: Vertex| (i128::from(b.0 - a.0) * i128::from(c.1 - a.1) - i128::from(b.1 - a.1) * i128::from(c.0 - a.0)).signum();
    let on_line = |part: &Part, x: [f64; 2]| {
        let (a, b) = (float(part.a), float(part.b));
        let t = ((x[0] - a[0]) * (b[0] - a[0]) + (x[1] - a[1]) * (b[1] - a[1])) / ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2));
        (0. ..=1.).contains(&t)
    };
    let on = |part: &Part, x: [f64; 2]| match part.arc {
        Some((c, _, clockwise)) => within(x, c, part.a, part.b, clockwise),
        None => on_line(part, x),
    };
    match (p.arc, q.arc) {
        (None, None) => {
            let (d1, d2, d3, d4) = (orient(q.a, q.b, p.a), orient(q.a, q.b, p.b), orient(p.a, p.b, q.a), orient(p.a, p.b, q.b));
            d1 * d2 < 0 && d3 * d4 < 0
        }
        (Some((c, r, _)), None) | (None, Some((c, r, _))) => {
            let line = if p.arc.is_none() { p } else { q };
            let (a, b) = (float(line.a), float(line.b));
            let (d, f) = ([b[0] - a[0], b[1] - a[1]], [a[0] - c[0], a[1] - c[1]]);
            let (qa, qb, qc) = (d[0] * d[0] + d[1] * d[1], 2. * (f[0] * d[0] + f[1] * d[1]), f[0] * f[0] + f[1] * f[1] - r * r);
            let disc = qb * qb - 4. * qa * qc;
            disc >= 0.
                && [-1., 1.].iter().any(|sign| {
                    let t = (-qb + sign * disc.sqrt()) / (2. * qa);
                    (0. ..=1.).contains(&t) && on(p, [a[0] + t * d[0], a[1] + t * d[1]]) && on(q, [a[0] + t * d[0], a[1] + t * d[1]])
                })
        }
        (Some((c1, r1, _)), Some((c2, r2, _))) => {
            let d = distance(c1, c2);
            if d == 0. || d > r1 + r2 || d < (r1 - r2).abs() {
                return false;
            }
            let x = (d * d + r1 * r1 - r2 * r2) / (2. * d);
            let y = (r1 * r1 - x * x).max(0.).sqrt();
            let (ux, uy) = ((c2[0] - c1[0]) / d, (c2[1] - c1[1]) / d);
            [-1., 1.].iter().any(|sign| {
                let point = [c1[0] + x * ux - sign * y * uy, c1[1] + x * uy + sign * y * ux];
                on(p, point) && on(q, point)
            })
        }
    }
}

/// Shortest distance between two pieces: zero when they cross, otherwise the least of the end
/// point distances and, for arcs, of the arc points facing the other piece (along the line's
/// normal, or along the line between the two centres).
fn gap(p: &Part, q: &Part) -> f64 {
    if crossing(p, q) {
        return 0.;
    }
    let mut best = [reach_to(float(p.a), q), reach_to(float(p.b), q), reach_to(float(q.a), p), reach_to(float(q.b), p)]
        .into_iter()
        .fold(f64::INFINITY, f64::min);
    for (arc, other) in [(p, q), (q, p)] {
        let Some((c, r, clockwise)) = arc.arc else { continue };
        let toward = match other.arc {
            Some((o, ..)) => [o[0] - c[0], o[1] - c[1]],
            None => [-(float(other.b)[1] - float(other.a)[1]), float(other.b)[0] - float(other.a)[0]],
        };
        let length = toward[0].hypot(toward[1]);
        if length == 0. {
            continue;
        }
        for sign in [-1., 1.] {
            let point = [c[0] + sign * r * toward[0] / length, c[1] + sign * r * toward[1] / length];
            if within(point, c, arc.a, arc.b, clockwise) {
                best = best.min(reach_to(point, other));
            }
        }
    }
    best
}

/// Walk the loops from the outer one, each from its entry vertex, entering every joined hole
/// where its cut-in leaves the segment and returning along the coincident cut-in. An explicit
/// stack keeps deeply chained holes off the call stack.
fn assemble(loops: &[Vec<Segment>], entries: &[usize], joins: &[Vec<Join>], stats: &mut Stats) -> Vec<Segment> {
    struct Walk {
        l: usize,
        step: usize,
        next: usize,
        at: Option<(Vertex, [f64; 2])>,
    }
    let cut = |start: Vertex| Segment { start, exact: float(start), shape: Shape::Line };
    let arc_of = |segment: &Segment| match segment.shape {
        Shape::Arc { clockwise, circle: (center, radius), .. } => Some((center, radius, clockwise)),
        Shape::Line => None,
    };
    let mut out = Vec::with_capacity(loops.iter().map(Vec::len).sum::<usize>() + 2 * loops.len());
    let mut stack = vec![Walk { l: 0, step: 0, next: 0, at: None }];
    while let Some(walk) = stack.last_mut() {
        let segments = &loops[walk.l];
        let n = segments.len();
        if walk.step == n {
            let done = walk.l;
            stack.pop();
            if done != 0 {
                out.push(cut(loops[done][entries[done]].start));
            }
            continue;
        }
        let k = (entries[walk.l] + walk.step) % n;
        let (p, exact) = walk.at.unwrap_or((segments[k].start, segments[k].exact));
        if let Some(join) = joins[walk.l].get(walk.next).filter(|join| join.segment == k) {
            if join.h != p {
                out.push(Segment { start: p, exact, shape: shape(p, join.h, arc_of(&segments[k]), stats) });
            }
            walk.at = Some((join.h, join.exact));
            walk.next += 1;
            out.push(cut(join.h));
            stack.push(Walk { l: join.hole, step: 0, next: 0, at: None });
            continue;
        }
        let shape = match walk.at {
            None => segments[k].shape,
            Some(_) => shape(p, segments[(k + 1) % n].start, arc_of(&segments[k]), stats),
        };
        out.push(Segment { start: p, exact, shape });
        walk.at = None;
        walk.step += 1;
    }
    out
}

/// Most bands one segment is copied into before it moves to the list every ray checks.
const BAND_COPIES: usize = 16;

/// Segments sorted into horizontal bands by the Y range they span, so a horizontal ray only
/// looks at the segments of its own band. Written segments are straight or within one quadrant,
/// so their end points bound their Y range. A segment spanning more than [`BAND_COPIES`] bands
/// is kept once in `long` instead, which every ray also looks at, so the index stays linear in
/// its segments while ordinary arcs, a few bands tall, stay out of every ray's way.
struct Bands {
    low: i64,
    height: i64,
    bands: Vec<Vec<(usize, usize)>>,
    long: Vec<(usize, usize)>,
}

impl Bands {
    fn new(loops: &[Vec<Segment>]) -> Self {
        let ys = loops.iter().flatten().map(|segment| segment.start.1);
        let (low, high) = (ys.clone().min().unwrap_or(0), ys.max().unwrap_or(0));
        let total: usize = loops.iter().map(Vec::len).sum();
        let count = (total / 8).clamp(1, 1 << 16) as i64;
        let height = ((high - low) / count + 1).max(1);
        let (mut bands, mut long) = (vec![Vec::new(); ((high - low) / height + 1) as usize], Vec::new());
        for (l, segments) in loops.iter().enumerate() {
            let n = segments.len();
            for k in 0..n {
                let (a, b) = (segments[k].start.1, segments[(k + 1) % n].start.1);
                let (first, last) = (((a.min(b) - low) / height) as usize, ((a.max(b) - low) / height) as usize);
                if last - first >= BAND_COPIES {
                    long.push((l, k));
                    continue;
                }
                for band in &mut bands[first..=last] {
                    band.push((l, k));
                }
            }
        }
        Self { low, height, bands, long }
    }

    /// The segments whose Y range may contain `y`.
    fn crossing(&self, y: i64) -> impl Iterator<Item = &(usize, usize)> {
        let band = usize::try_from((y - self.low) / self.height).ok().and_then(|band| self.bands.get(band));
        band.map_or(&[][..], Vec::as_slice).iter().chain(&self.long)
    }
}

/// Find the first of `candidates` a horizontal ray from `v` in +X meets, preferring an existing
/// vertex at the same X. Written segments are straight or within one quadrant of their circle,
/// so each meets the ray's line at most once (a horizontal line on it is met at its nearer end).
fn nearest<'a>(loops: &[Vec<Segment>], candidates: impl Iterator<Item = &'a (usize, usize)>, v: Vertex) -> Result<(usize, Hit), String> {
    let mut best: Option<(f64, usize, Hit)> = None;
    let mut offer = |x: f64, l: usize, hit: Hit| {
        if x > v.0 as f64
            && best.as_ref().is_none_or(|(bx, _, current)| x < *bx || (x == *bx && matches!((current, &hit), (Hit::Inside(..), Hit::Vertex(_)))))
        {
            best = Some((x, l, hit));
        }
    };
    for &(l, k) in candidates {
        let segments = &loops[l];
        let n = segments.len();
        let (a, b) = (segments[k].start, segments[(k + 1) % n].start);
        if a.1 == v.1 {
            offer(a.0 as f64, l, Hit::Vertex(k));
        }
        if a.1 == v.1 || b.1 == v.1 || (a.1 < v.1) == (b.1 < v.1) {
            continue;
        }
        let (low, high) = (a.0.min(b.0) as f64, a.0.max(b.0) as f64);
        let x = match segments[k].shape {
            Shape::Line => {
                let (p, q) = (segments[k].exact, segments[(k + 1) % n].exact);
                if p[1] == q[1] { a.0 as f64 } else { p[0] + (v.1 as f64 - p[1]) * (q[0] - p[0]) / (q[1] - p[1]) }
            }
            Shape::Arc { circle: (center, radius), .. } => {
                let reach = (radius * radius - (v.1 as f64 - center[1]).powi(2)).max(0.).sqrt();
                if (a.0 + b.0) as f64 >= 2. * center[0] { center[0] + reach } else { center[0] - reach }
            }
        };
        offer(x.clamp(low, high), l, Hit::Inside(k, x.clamp(low, high)));
    }
    best.map(|(_, l, hit)| (l, hit)).ok_or_else(|| "A copper zone hole could not be joined to its boundary.".into())
}

/// Write one island as one region: a contour per region of the island, tagged as conductor
/// copper (§5.6.10). `G75` precedes the region when it has arcs, because only D01, D02 and G01–G03
/// may appear inside one (§4.10.2); linear mode is restored before it ends.
fn region(body: &mut String, contours: &[Vec<Segment>]) {
    if contours.iter().flatten().any(|segment| segment.shape != Shape::Line) {
        body.push_str("G75*\n");
    }
    ApertureFunction::Conductor.set(body);
    body.push_str("G36*\n");
    let mut mode = "G01*";
    for contour in contours {
        let Some(first) = contour.first() else { continue };
        let _ = writeln!(body, "X{}Y{}D02*", first.start.0, first.start.1);
        for (i, segment) in contour.iter().enumerate() {
            let (x, y) = contour[(i + 1) % contour.len()].start;
            let wanted = match segment.shape {
                Shape::Line => "G01*",
                Shape::Arc { clockwise: true, .. } => "G02*",
                Shape::Arc { clockwise: false, .. } => "G03*",
            };
            if wanted != mode {
                let _ = writeln!(body, "{wanted}");
                mode = wanted;
            }
            match segment.shape {
                Shape::Line => {
                    let _ = writeln!(body, "X{x}Y{y}D01*");
                }
                Shape::Arc { center, .. } => {
                    let _ = writeln!(body, "X{x}Y{y}I{}J{}D01*", center.0 - segment.start.0, center.1 - segment.start.1);
                }
            }
        }
    }
    if mode != "G01*" {
        body.push_str("G01*\n");
    }
    body.push_str("G37*\n");
    body.push_str(DELETE_APERTURE_FUNCTION);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcbcraft_geometry::planar::{Operation, Regions};

    const MM: i64 = 1_000_000;

    fn rect(x: i64, y: i64, w: i64, h: i64) -> Regions {
        Regions::polygon(&[[x, y], [x + w, y], [x + w, y + h], [x, y + h]].map(|p| p.map(|v| v * MM / 10))).unwrap()
    }

    /// Cut the holes from a 10 mm square and write its single region.
    fn written_board(holes: &[Regions]) -> (Vec<Segment>, Stats, f64) {
        let mut material = rect(0, 0, 100, 100);
        for hole in holes {
            material = material.boolean(hole, Operation::Difference).unwrap();
        }
        let boundaries = material.boundaries().unwrap();
        assert_eq!(boundaries.len(), 1);
        let mut stats = Stats::default();
        let contour = contour(&boundaries[0], Frame::BOARD_ZERO, &mut stats).unwrap();
        (contour, stats, material.area().unwrap().value_nm2)
    }

    /// Straight segments of a contour between consecutive vertices.
    fn lines(contour: &[Segment]) -> Vec<(Vertex, Vertex)> {
        (0..contour.len()).map(|i| (contour[i].start, contour[(i + 1) % contour.len()].start)).collect()
    }

    fn turn(a: Vertex, b: Vertex, c: Vertex) -> i128 {
        i128::from(b.0 - a.0) * i128::from(c.1 - a.1) - i128::from(b.1 - a.1) * i128::from(c.0 - a.0)
    }

    /// Holes are joined by horizontal cut-ins made of two fully-coincident segments, and the
    /// contour has no other crossing or overlap, also when a cut-in runs into another hole's
    /// vertex along that hole's edge and when two holes share their rightmost X.
    #[test]
    fn holes_join_with_horizontal_fully_coincident_cut_ins() {
        let holes = [rect(10, 10, 20, 20), rect(10, 40, 20, 10), rect(50, 40, 10, 10), rect(70, 70, 15, 5), rect(15, 70, 10, 10)];
        let (contour, stats, area) = written_board(&holes);
        assert_eq!(stats.cut_ins, holes.len());
        assert_eq!(stats.arcs, 0);
        let lines = lines(&contour);
        assert!(lines.iter().all(|(a, b)| a != b), "no zero-length segment");
        let twins: Vec<_> = (0..lines.len()).filter(|&i| lines.iter().any(|&(a, b)| (b, a) == lines[i])).collect();
        assert_eq!(twins.len(), 2 * holes.len(), "each cut-in is a pair of fully-coincident segments");
        assert!(twins.iter().all(|&i| lines[i].0.1 == lines[i].1.1), "cut-ins are horizontal");
        for i in 0..lines.len() {
            for j in i + 2..lines.len() {
                if i == 0 && j == lines.len() - 1 {
                    continue;
                }
                let ((a, b), (c, d)) = (lines[i], lines[j]);
                let proper = turn(a, b, c).signum() * turn(a, b, d).signum() < 0 && turn(c, d, a).signum() * turn(c, d, b).signum() < 0;
                assert!(!proper, "segments {i} and {j} cross");
                if turn(a, b, c) == 0 && turn(a, b, d) == 0 && (a, b) != (d, c) {
                    let axis = |p: Vertex| if a.0 == b.0 { p.1 } else { p.0 };
                    let overlap = axis(a).max(axis(b)).min(axis(c).max(axis(d))) - axis(a).min(axis(b)).max(axis(c).min(axis(d)));
                    assert!(overlap <= 0, "collinear segments {i} and {j} overlap");
                }
            }
        }
        assert!((super::area(&contour) - area).abs() < 1., "the cut-ins add no area: {} against {area}", super::area(&contour));
    }

    /// A circular clearance stays four exact quarter arcs within the documented bound; its
    /// rightmost point is a vertex, so the cut-in starts there.
    #[test]
    fn circular_holes_keep_exact_quarter_arcs() {
        let hole = Regions::circle([3_333_333, 6_666_667], 2_000_001).unwrap();
        let (contour, stats, area) = written_board(&[hole]);
        assert_eq!((stats.arcs, stats.cut_ins), (4, 1));
        assert!(stats.deviation_nm <= 3. * std::f64::consts::FRAC_1_SQRT_2, "{}", stats.deviation_nm);
        assert!(contour.iter().any(|segment| segment.start == (4_333_334, -6_666_667)), "the rightmost point, in output axes, is a vertex");
        assert!((super::area(&contour) - area).abs() < 2. * 7_000_000., "{} against {area}", super::area(&contour));
    }

    /// Arcs under the minimum chord are lines with their sagitta in the bound, and a loop that rounds
    /// away entirely is refused rather than written empty.
    #[test]
    fn short_arcs_become_lines_and_collapsed_loops_are_refused() {
        let mut stats = Stats::default();
        let arc = Some(([0., 0.], 100_000., false));
        assert_eq!(shape((100_000, 0), (99_999, 447), arc, &mut stats), Shape::Line);
        assert!(stats.deviation_nm < 1.8, "{}", stats.deviation_nm);
        assert!(matches!(shape((100_000, 0), (99_500, 9_987), arc, &mut stats), Shape::Arc { .. }));
        let tiny = [Piece { start: [0.2, 0.1], arc: None }, Piece { start: [0.4, 0.3], arc: None }, Piece { start: [0.1, 0.4], arc: None }];
        assert!(written(&tiny, &mut stats).is_err());
    }

    /// An arc is written only when its end points stay at least 1 µm apart after each moves by
    /// the ±0.5 µm per coordinate that §4.14.2 allows. A 1131 nm chord can close to 1 nm and
    /// flip a quarter arc into a nearly full circle, so it is a line; a 2.5 µm chord stays an arc.
    #[test]
    fn arcs_keep_their_end_points_apart_under_the_allowed_perturbation() {
        let mut stats = Stats::default();
        let radius = 71_100f64.hypot(70_300.);
        let arc = Some(([0., 0.], radius, false));
        assert_eq!(shape((71_100, 70_300), (70_300, 71_100), arc, &mut stats), Shape::Line);
        assert!(matches!(shape((72_000, 69_368), (70_232, 71_134), arc, &mut stats), Shape::Arc { .. }));
    }

    /// A short arc written as a line moves the boundary by its sagitta. Beyond the documented
    /// tolerance the contour is refused: a clearance circle of 500 nm radius would be a square whose
    /// sides are 146 nm inside the circle.
    #[test]
    fn a_line_beyond_the_tolerance_is_refused() {
        let material = rect(0, 0, 100, 100).boolean(&Regions::circle([5 * MM, 5 * MM], 1_000).unwrap(), Operation::Difference).unwrap();
        let boundaries = material.boundaries().unwrap();
        let error = contour(&boundaries[0], Frame::BOARD_ZERO, &mut Stats::default()).unwrap_err();
        assert!(error.contains("100 nm"), "{error}");
    }

    /// Facing edges of two holes 500 nm apart could touch under the ±0.5 µm perturbation of
    /// §4.14.2, so the contour is refused, for straight edges and for arcs; 1 µm or more apart it
    /// is written.
    #[test]
    fn segments_closer_than_one_micrometre_are_refused() {
        let square = |x: i64| Regions::polygon(&[[x, 2 * MM], [x + MM, 2 * MM], [x + MM, 4 * MM], [x, 4 * MM]]).unwrap();
        let disc = |x: i64| Regions::circle([x + MM / 2, 3 * MM], MM).unwrap();
        type Hole<'a> = &'a dyn Fn(i64) -> Regions;
        let cases: [(Hole, i64, bool); 4] = [(&square, 500, false), (&square, 1_000, true), (&disc, 500, false), (&disc, 1_500, true)];
        for (hole, gap, ok) in cases {
            let material = rect(0, 0, 100, 100)
                .boolean(&hole(2 * MM), Operation::Difference)
                .unwrap()
                .boolean(&hole(3 * MM + gap), Operation::Difference)
                .unwrap();
            let boundaries = material.boundaries().unwrap();
            let result = contour(&boundaries[0], Frame::BOARD_ZERO, &mut Stats::default());
            assert_eq!(result.is_ok(), ok, "gap {gap}: {:?}", result.err());
            if let Err(error) = result {
                assert!(error.contains("1 µm"), "{error}");
            }
        }
    }

    /// A cut-in never meets a circular segment tangentially (§4.10.3). The +X ray from the left
    /// hole's rightmost point touches the right hole at the bottom of its circle, and the −X ray
    /// from the right hole touches the left one at its top, so the holes are joined another way.
    #[test]
    fn cut_ins_never_touch_an_arc_tangentially() {
        let holes = [Regions::circle([2 * MM, 4 * MM], 2 * MM).unwrap(), Regions::circle([5 * MM, 3 * MM], 2 * MM).unwrap()];
        let (contour, stats, area) = written_board(&holes);
        assert_eq!(stats.cut_ins, 2);
        let lines = lines(&contour);
        for &(a, b) in lines.iter().filter(|&&(a, b)| lines.contains(&(b, a))) {
            let direction = [(b.0 - a.0) as f64, (b.1 - a.1) as f64];
            let length = direction[0].hypot(direction[1]);
            for (i, segment) in contour.iter().enumerate() {
                let Shape::Arc { center, .. } = segment.shape else { continue };
                for end in [segment.start, contour[(i + 1) % contour.len()].start] {
                    if end == a || end == b {
                        let radial = [(end.0 - center.0) as f64, (end.1 - center.1) as f64];
                        let along = (radial[0] * direction[0] + radial[1] * direction[1]).abs() / length;
                        assert!(along > 1_000., "the cut-in {a:?}-{b:?} is tangent to the arc about {center:?}");
                    }
                }
            }
        }
        assert!((super::area(&contour) - area).abs() < 4. * 7_000_000., "{} against {area}", super::area(&contour));
    }

    /// Long edges are kept once, not copied into every band they span: 200 tall holes give at
    /// most two band entries per segment.
    #[test]
    fn bands_hold_each_long_edge_once() {
        let square = |x: i64, y: i64, w: i64, h: i64| {
            [(x, y), (x + w, y), (x + w, y + h), (x, y + h)].map(|start| Segment { start, exact: float(start), shape: Shape::Line }).to_vec()
        };
        let mut loops = vec![square(0, 0, 1_000 * MM, 100 * MM)];
        loops.extend((0..200).map(|i| square((i * 5 + 1) * MM, MM, MM, 98 * MM)));
        let total: usize = loops.iter().map(Vec::len).sum();
        let bands = Bands::new(&loops);
        let entries = bands.bands.iter().map(Vec::len).sum::<usize>() + bands.long.len();
        assert!(entries <= 2 * total, "{entries} band entries for {total} segments");
        assert_eq!(
            bands.crossing(50 * MM).filter(|&&(l, k)| loops[l][k].start.0 == loops[l][(k + 1) % 4].start.0).count(),
            402,
            "every vertical edge"
        );
    }
}
