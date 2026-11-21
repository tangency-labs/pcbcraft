//! Footprint courtyard outline generation, validation and vertex editing.
//!
//! A courtyard outline is stored as a closed `Geometry::Polyline` artwork primitive with
//! `GraphicPurpose::Courtyard` and `Fill::None`, matching the native footprint schema and
//! exact polygon-polygon DRC checks in `board::drc::courtyard`.
use super::{Item, Primitive};
use crate::{
    board::definition::{self as footprint, GraphicPurpose},
    library_graphics::{Fill, Geometry, Graphic},
    schematic::Point,
};
use serde::{Deserialize, Serialize};

/// Default courtyard clearance: 0.25 mm (IPC-7351 Density Level B nominal courtyard excess,
/// Table 3-13).
pub const DEFAULT_CLEARANCE_NM: i64 = 250_000;
/// Default courtyard outline stroke width: 0.05 mm.
pub const DEFAULT_STROKE_NM: i64 = 50_000;
/// Maximum allowed courtyard clearance when generating an outline (10.0 mm).
pub const MAX_CLEARANCE_NM: i64 = 10_000_000;
/// Minimum number of vertices in a closed courtyard outline.
pub const MIN_VERTICES: usize = 3;
/// Maximum number of vertices in a closed courtyard outline (matches `Graphic::validate`).
pub const MAX_VERTICES: usize = 64;

/// Serde default for `Action::GenerateCourtyard::stroke_nm`.
pub(crate) const fn default_stroke_nm() -> i64 {
    DEFAULT_STROKE_NM
}

/// Serde default for `Action::GenerateCourtyard::replace_existing`.
pub(crate) const fn default_replace_existing() -> bool {
    true
}

/// Which footprint primitives contribute to an auto-generated courtyard bounding outline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CourtyardSource {
    /// Include all copper pads, non-plated holes and non-text fabrication body graphics
    /// (excludes silkscreen and text legends).
    #[default]
    PadsAndBody,
    /// Include only copper pads.
    PadsOnly,
}

/// Derived physical metrics for a closed courtyard polygon in local footprint nanometers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CourtyardMetrics {
    pub min: Point,
    pub max: Point,
    pub width_nm: i64,
    pub height_nm: i64,
    /// Twice the signed area magnitude is halved; stored in square nanometers (`nm^2`).
    pub area_nm2: i128,
    pub vertex_count: usize,
    pub is_simple: bool,
}

/// Return whether a library primitive belongs to the footprint courtyard layer.
pub fn is_courtyard_primitive(primitive: &Primitive) -> bool {
    match primitive {
        Primitive::Outline { graphic } => graphic.purpose == GraphicPurpose::Courtyard,
        Primitive::Artwork { artwork } => artwork.purpose == GraphicPurpose::Courtyard,
        _ => false,
    }
}

/// Extract the vertices and stroke width of an editable closed courtyard polygon primitive.
/// Returns `None` for non-courtyard primitives, open polylines, rectangles, or other shapes.
pub fn courtyard_polygon(primitive: &Primitive) -> Option<(&[Point], i64)> {
    match primitive {
        Primitive::Artwork {
            artwork:
                footprint::Artwork {
                    purpose: GraphicPurpose::Courtyard,
                    graphic: Graphic { geometry: Geometry::Polyline { points, closed: true }, stroke_nm, .. },
                },
        } => Some((points.as_slice(), *stroke_nm)),
        _ => None,
    }
}

/// Build and validate a closed courtyard polygon primitive from `points` and `stroke_nm`.
///
/// Rejects polygons with fewer than 3 or more than 64 vertices, duplicate consecutive
/// vertices, zero area, self-intersecting edges, out-of-bounds coordinates, or invalid
/// stroke widths.
pub fn make_courtyard_polygon(points: Vec<Point>, stroke_nm: i64) -> Result<Primitive, String> {
    validate_polygon(&points)?;
    let graphic = Graphic { geometry: Geometry::Polyline { points, closed: true }, stroke_nm, fill: Fill::None };
    graphic.validate(footprint::MAX_OFFSET_NM)?;
    Ok(Primitive::Artwork { artwork: footprint::Artwork { purpose: GraphicPurpose::Courtyard, graphic } })
}

/// Generate a closed rectangular courtyard polygon enclosing the selected primitives in `item`
/// expanded outward by `clearance_nm`.
///
/// Returns an error if `clearance_nm` is negative or exceeds [`MAX_CLEARANCE_NM`], if no
/// qualifying source primitives exist, or if the resulting outline exceeds definition bounds.
pub fn generate(item: &Item, clearance_nm: i64, source: CourtyardSource, stroke_nm: i64) -> Result<Primitive, String> {
    if !(0..=MAX_CLEARANCE_NM).contains(&clearance_nm) {
        return Err("Courtyard clearance must be between 0 and 10 mm.".into());
    }
    let mut bounds: Option<(i64, i64, i64, i64)> = None;
    for primitive in item.objects.values() {
        let include = match (source, primitive) {
            (CourtyardSource::PadsOnly, Primitive::Pad { .. }) => true,
            (CourtyardSource::PadsOnly, _) => false,
            (CourtyardSource::PadsAndBody, Primitive::Pad { .. } | Primitive::NonPlatedHole { .. }) => true,
            (CourtyardSource::PadsAndBody, Primitive::Outline { graphic }) => graphic.purpose == GraphicPurpose::Fabrication,
            (CourtyardSource::PadsAndBody, Primitive::Artwork { artwork }) => artwork.purpose == GraphicPurpose::Fabrication,
            (CourtyardSource::PadsAndBody, _) => false,
        };
        if !include {
            continue;
        }
        for point in primitive_extents(primitive) {
            bounds = Some(match bounds {
                Some((min_x, min_y, max_x, max_y)) => (min_x.min(point.x_nm), min_y.min(point.y_nm), max_x.max(point.x_nm), max_y.max(point.y_nm)),
                None => (point.x_nm, point.y_nm, point.x_nm, point.y_nm),
            });
        }
    }
    let (min_x, min_y, max_x, max_y) = bounds.ok_or_else(|| match source {
        CourtyardSource::PadsOnly => "Add at least one pad before generating a courtyard.".to_owned(),
        CourtyardSource::PadsAndBody => "Add at least one pad or body graphic before generating a courtyard.".to_owned(),
    })?;
    let min_x = min_x.checked_sub(clearance_nm).ok_or("Courtyard extends outside the definition bounds.")?;
    let min_y = min_y.checked_sub(clearance_nm).ok_or("Courtyard extends outside the definition bounds.")?;
    let max_x = max_x.checked_add(clearance_nm).ok_or("Courtyard extends outside the definition bounds.")?;
    let max_y = max_y.checked_add(clearance_nm).ok_or("Courtyard extends outside the definition bounds.")?;
    if min_x >= max_x || min_y >= max_y {
        return Err("Generated courtyard has zero area; increase clearance or add geometry.".into());
    }
    let points = vec![
        Point { x_nm: min_x, y_nm: min_y },
        Point { x_nm: max_x, y_nm: min_y },
        Point { x_nm: max_x, y_nm: max_y },
        Point { x_nm: min_x, y_nm: max_y },
    ];
    make_courtyard_polygon(points, stroke_nm)
}

/// Return a new courtyard polygon primitive with vertex `index` moved to `to`.
///
/// Rejects out-of-range indices, duplicate adjacent vertices, self-intersections, zero area,
/// or coordinates outside footprint definition bounds.
pub fn with_moved_vertex(primitive: &Primitive, index: usize, to: Point) -> Result<Primitive, String> {
    let (points, stroke_nm) = courtyard_polygon(primitive).ok_or("Selected primitive is not a closed courtyard outline.")?;
    let mut updated = points.to_vec();
    let slot = updated.get_mut(index).ok_or("Courtyard vertex index is out of range.")?;
    *slot = to;
    make_courtyard_polygon(updated, stroke_nm)
}

/// Insert a new vertex immediately after `edge_index` (between `edge_index` and the next vertex).
///
/// Rejects insertions that would exceed [`MAX_VERTICES`], duplicate an adjacent vertex, or
/// create a self-intersecting outline.
pub fn with_inserted_vertex(primitive: &Primitive, edge_index: usize, point: Point) -> Result<Primitive, String> {
    let (points, stroke_nm) = courtyard_polygon(primitive).ok_or("Selected primitive is not a closed courtyard outline.")?;
    if points.len() >= MAX_VERTICES {
        return Err(format!("A courtyard outline can have at most {MAX_VERTICES} vertices."));
    }
    if edge_index >= points.len() {
        return Err("Courtyard edge index is out of range.".into());
    }
    let mut updated = points.to_vec();
    updated.insert(edge_index + 1, point);
    make_courtyard_polygon(updated, stroke_nm)
}

/// Remove vertex `index` from a courtyard polygon, requiring at least [`MIN_VERTICES`] to remain.
pub fn with_removed_vertex(primitive: &Primitive, index: usize) -> Result<Primitive, String> {
    let (points, stroke_nm) = courtyard_polygon(primitive).ok_or("Selected primitive is not a closed courtyard outline.")?;
    if points.len() <= MIN_VERTICES {
        return Err("A courtyard outline needs at least three vertices.".into());
    }
    if index >= points.len() {
        return Err("Courtyard vertex index is out of range.".into());
    }
    let mut updated = points.to_vec();
    updated.remove(index);
    make_courtyard_polygon(updated, stroke_nm)
}

/// Compute the integer midpoint of edge `edge_index` (wrapping from the last vertex to the first).
pub fn edge_midpoint(points: &[Point], edge_index: usize) -> Option<Point> {
    let a = points.get(edge_index)?;
    let b = points.get((edge_index + 1) % points.len())?;
    Some(Point { x_nm: ((i128::from(a.x_nm) + i128::from(b.x_nm)) / 2) as i64, y_nm: ((i128::from(a.y_nm) + i128::from(b.y_nm)) / 2) as i64 })
}

/// Validate that `points` forms a bounded, non-degenerate, non-self-intersecting closed polygon.
pub fn validate_polygon(points: &[Point]) -> Result<(), String> {
    if points.len() < MIN_VERTICES {
        return Err("A courtyard outline needs at least three vertices.".into());
    }
    if points.len() > MAX_VERTICES {
        return Err(format!("A courtyard outline can have at most {MAX_VERTICES} vertices."));
    }
    let n = points.len();
    for i in 0..n {
        let Some(&a) = points.get(i) else {
            return Err("Invalid courtyard vertex list.".into());
        };
        let Some(&b) = points.get((i + 1) % n) else {
            return Err("Invalid courtyard vertex list.".into());
        };
        if a == b {
            return Err("Adjacent courtyard vertices must be distinct.".into());
        }
    }
    if doubled_signed_area(points) == 0 {
        return Err("A courtyard outline must enclose a non-zero area.".into());
    }
    if !is_simple_polygon(points) {
        return Err("A courtyard outline cannot intersect itself.".into());
    }
    Ok(())
}

/// Return whether `points` forms a simple polygon whose non-adjacent edges never meet
/// and whose adjacent edges do not fold back onto each other.
pub fn is_simple_polygon(points: &[Point]) -> bool {
    let n = points.len();
    if n < MIN_VERTICES || doubled_signed_area(points) == 0 {
        return false;
    }
    for i in 0..n {
        let Some(&u) = points.get((i + n - 1) % n) else { return false };
        let Some(&v) = points.get(i) else { return false };
        let Some(&w) = points.get((i + 1) % n) else { return false };
        if v == w || (cross(u, v, w) == 0 && dot(u, v, w) > 0) {
            return false;
        }
    }
    for i in 0..n {
        let Some(&a) = points.get(i) else { return false };
        let Some(&b) = points.get((i + 1) % n) else { return false };
        for j in (i + 2)..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            let Some(&c) = points.get(j) else { return false };
            let Some(&d) = points.get((j + 1) % n) else { return false };
            if segments_meet(a, b, c, d) {
                return false;
            }
        }
    }
    true
}

/// Compute axis-aligned bounds, dimensions, area and simplicity for a closed courtyard polygon.
pub fn metrics(points: &[Point]) -> Option<CourtyardMetrics> {
    let &first = points.first()?;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (first.x_nm, first.y_nm, first.x_nm, first.y_nm);
    for &p in points.iter().skip(1) {
        min_x = min_x.min(p.x_nm);
        min_y = min_y.min(p.y_nm);
        max_x = max_x.max(p.x_nm);
        max_y = max_y.max(p.y_nm);
    }
    let width_nm = max_x.saturating_sub(min_x);
    let height_nm = max_y.saturating_sub(min_y);
    let area_nm2 = doubled_signed_area(points).abs() / 2;
    Some(CourtyardMetrics {
        min: Point { x_nm: min_x, y_nm: min_y },
        max: Point { x_nm: max_x, y_nm: max_y },
        width_nm,
        height_nm,
        area_nm2,
        vertex_count: points.len(),
        is_simple: is_simple_polygon(points),
    })
}

/// Compute twice the signed area of a closed polygon in square nanometers.
fn doubled_signed_area(points: &[Point]) -> i128 {
    let n = points.len();
    let mut sum = 0i128;
    for i in 0..n {
        if let (Some(&a), Some(&b)) = (points.get(i), points.get((i + 1) % n)) {
            sum += i128::from(a.x_nm) * i128::from(b.y_nm) - i128::from(a.y_nm) * i128::from(b.x_nm);
        }
    }
    sum
}

/// 2D cross product `(b - a) x (c - a)` in exact `i128` arithmetic.
fn cross(a: Point, b: Point, c: Point) -> i128 {
    (i128::from(b.x_nm) - i128::from(a.x_nm)) * (i128::from(c.y_nm) - i128::from(a.y_nm))
        - (i128::from(b.y_nm) - i128::from(a.y_nm)) * (i128::from(c.x_nm) - i128::from(a.x_nm))
}

/// 2D dot product `(a - b) . (c - b)` in exact `i128` arithmetic; positive on a zero-width spike.
fn dot(a: Point, b: Point, c: Point) -> i128 {
    (i128::from(a.x_nm) - i128::from(b.x_nm)) * (i128::from(c.x_nm) - i128::from(b.x_nm))
        + (i128::from(a.y_nm) - i128::from(b.y_nm)) * (i128::from(c.y_nm) - i128::from(b.y_nm))
}

/// Return whether point `p` lies inside the axis-aligned box of segment `a..b`.
fn within(a: Point, b: Point, p: Point) -> bool {
    p.x_nm >= a.x_nm.min(b.x_nm) && p.x_nm <= a.x_nm.max(b.x_nm) && p.y_nm >= a.y_nm.min(b.y_nm) && p.y_nm <= a.y_nm.max(b.y_nm)
}

/// Return whether closed segments `a..b` and `c..d` share any point.
fn segments_meet(a: Point, b: Point, c: Point, d: Point) -> bool {
    let o1 = cross(a, b, c).signum();
    let o2 = cross(a, b, d).signum();
    let o3 = cross(c, d, a).signum();
    let o4 = cross(c, d, b).signum();
    if o1 != o2 && o3 != o4 {
        return true;
    }
    (o1 == 0 && within(a, b, c)) || (o2 == 0 && within(a, b, d)) || (o3 == 0 && within(c, d, a)) || (o4 == 0 && within(c, d, b))
}

/// Collect boundary points of a footprint primitive for courtyard generation.
fn primitive_extents(primitive: &Primitive) -> Vec<Point> {
    let rect_corners = |center: Point, width_nm: i64, height_nm: i64, rotation: u16| {
        let angle = f64::from(rotation).to_radians();
        let (sin, cos) = angle.sin_cos();
        [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)]
            .into_iter()
            .map(|(rx, ry)| {
                let (x, y) = (rx * width_nm as f64, ry * height_nm as f64);
                Point {
                    x_nm: center.x_nm.saturating_add((x * cos - y * sin).round() as i64),
                    y_nm: center.y_nm.saturating_add((x * sin + y * cos).round() as i64),
                }
            })
            .collect::<Vec<_>>()
    };
    match primitive {
        Primitive::Pad { pad } => rect_corners(pad.center, pad.width_nm, pad.height_nm, pad.rotation),
        Primitive::NonPlatedHole { hole } => rect_corners(hole.center, hole.diameter_nm, hole.diameter_nm, 0),
        Primitive::Outline { graphic } => rect_corners(graphic.center, graphic.width_nm, graphic.height_nm, graphic.rotation),
        Primitive::Artwork { artwork } if matches!(artwork.graphic.geometry, Geometry::Text { .. }) => Vec::new(),
        Primitive::Artwork { artwork } => artwork.graphic.paths().into_iter().flatten().collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring::{Action, Kind, Session};

    fn pt(x_nm: i64, y_nm: i64) -> Point {
        Point { x_nm, y_nm }
    }

    /// Generating from pads and body encloses both pads and fabrication outline plus clearance.
    #[test]
    fn generate_from_pads_and_body_expands_by_clearance_and_validates() {
        let item = Item::new(Kind::Footprint).unwrap();
        // Default footprint has pads at x = ±500_000 (w=500_000, h=600_000 => x in [-750_000, 750_000], y in [-300_000, 300_000])
        // and fabrication rect at (0, 0) w=1_600_000, h=800_000 => x in [-800_000, 800_000], y in [-400_000, 400_000].
        let primitive = generate(&item, DEFAULT_CLEARANCE_NM, CourtyardSource::PadsAndBody, DEFAULT_STROKE_NM).unwrap();
        let (points, stroke) = courtyard_polygon(&primitive).unwrap();
        assert_eq!(stroke, DEFAULT_STROKE_NM);
        assert_eq!(points, &[pt(-1_050_000, -650_000), pt(1_050_000, -650_000), pt(1_050_000, 650_000), pt(-1_050_000, 650_000),]);
        let m = metrics(points).unwrap();
        assert_eq!(m.width_nm, 2_100_000);
        assert_eq!(m.height_nm, 1_300_000);
        assert_eq!(m.vertex_count, 4);
        assert!(m.is_simple);
    }

    /// Pads-and-body generation ignores silkscreen legends/artwork outside the body.
    #[test]
    fn generate_pads_and_body_ignores_silkscreen_artwork_and_text() {
        let mut item = Item::new(Kind::Footprint).unwrap();
        item.objects.insert(
            90,
            Primitive::Artwork {
                artwork: footprint::Artwork {
                    purpose: GraphicPurpose::Silkscreen,
                    graphic: Graphic {
                        geometry: Geometry::Line { from: pt(-5_000_000, 0), to: pt(5_000_000, 0) },
                        stroke_nm: 120_000,
                        fill: Fill::None,
                    },
                },
            },
        );
        let primitive = generate(&item, DEFAULT_CLEARANCE_NM, CourtyardSource::PadsAndBody, DEFAULT_STROKE_NM).unwrap();
        let (points, _) = courtyard_polygon(&primitive).unwrap();
        let m = metrics(points).unwrap();
        assert_eq!(m.width_nm, 2_100_000);
        assert_eq!(m.height_nm, 1_300_000);
    }

    /// Pads-and-body generation ignores wide fabrication text labels (e.g. `${REFERENCE}`).
    #[test]
    fn generate_pads_and_body_ignores_fabrication_text() {
        let mut item = Item::new(Kind::Footprint).unwrap();
        item.objects.insert(
            91,
            Primitive::Artwork {
                artwork: footprint::Artwork {
                    purpose: GraphicPurpose::Fabrication,
                    graphic: Graphic {
                        geometry: Geometry::Text {
                            at: pt(0, 0),
                            text: "${REFERENCE}_LONG_FAB_LABEL".into(),
                            height_nm: 2_000_000,
                            rotation: 0,
                            align: crate::library_graphics::TextAlign::Middle,
                        },
                        stroke_nm: 120_000,
                        fill: Fill::None,
                    },
                },
            },
        );
        let primitive = generate(&item, DEFAULT_CLEARANCE_NM, CourtyardSource::PadsAndBody, DEFAULT_STROKE_NM).unwrap();
        let (points, _) = courtyard_polygon(&primitive).unwrap();
        let m = metrics(points).unwrap();
        assert_eq!(m.width_nm, 2_100_000);
        assert_eq!(m.height_nm, 1_300_000);
    }

    /// Inserting an exact edge midpoint on a default 0.25 mm outline keeps its area and bounds unchanged.
    #[test]
    fn insert_edge_midpoint_on_default_clearance_preserves_area_and_bounds() {
        let item = Item::new(Kind::Footprint).unwrap();
        let primitive = generate(&item, DEFAULT_CLEARANCE_NM, CourtyardSource::PadsAndBody, DEFAULT_STROKE_NM).unwrap();
        let (before_pts, _) = courtyard_polygon(&primitive).unwrap();
        let before = metrics(before_pts).unwrap();

        let mid = edge_midpoint(before_pts, 1).unwrap();
        assert_eq!(mid, pt(1_050_000, 0));
        let inserted = with_inserted_vertex(&primitive, 1, mid).unwrap();
        let (after_pts, _) = courtyard_polygon(&inserted).unwrap();
        let after = metrics(after_pts).unwrap();

        assert_eq!(after.vertex_count, 5);
        assert_eq!(after.area_nm2, before.area_nm2);
        assert_eq!(after.width_nm, before.width_nm);
        assert_eq!(after.height_nm, before.height_nm);
    }

    /// Pads-only generation ignores a wider fabrication outline.
    #[test]
    fn generate_pads_only_ignores_larger_fabrication_body() {
        let item = Item::new(Kind::Footprint).unwrap();
        let primitive = generate(&item, 100_000, CourtyardSource::PadsOnly, DEFAULT_STROKE_NM).unwrap();
        let (points, _) = courtyard_polygon(&primitive).unwrap();
        assert_eq!(points, &[pt(-850_000, -400_000), pt(850_000, -400_000), pt(850_000, 400_000), pt(-850_000, 400_000),]);
    }

    /// Out-of-range clearance and empty footprints are rejected cleanly.
    #[test]
    fn generate_rejects_empty_source_and_out_of_range_clearance() {
        let mut item = Item::new(Kind::Footprint).unwrap();
        assert!(generate(&item, -1, CourtyardSource::PadsAndBody, DEFAULT_STROKE_NM).is_err());
        assert!(generate(&item, MAX_CLEARANCE_NM + 1, CourtyardSource::PadsAndBody, DEFAULT_STROKE_NM).is_err());
        item.objects.clear();
        assert!(generate(&item, DEFAULT_CLEARANCE_NM, CourtyardSource::PadsAndBody, DEFAULT_STROKE_NM).is_err());
        assert!(generate(&item, DEFAULT_CLEARANCE_NM, CourtyardSource::PadsOnly, DEFAULT_STROKE_NM).is_err());
    }

    /// Moving, inserting and removing vertices keep the outline simple and refuse bowties.
    #[test]
    fn vertex_move_insert_remove_preserve_simple_polygon_and_reject_self_intersection() {
        let square = make_courtyard_polygon(vec![pt(0, 0), pt(2_000_000, 0), pt(2_000_000, 2_000_000), pt(0, 2_000_000)], DEFAULT_STROKE_NM).unwrap();

        // Insert a vertex on edge 1 (right edge) and indent it to form a concave L-shape.
        let mid = edge_midpoint(courtyard_polygon(&square).unwrap().0, 1).unwrap();
        assert_eq!(mid, pt(2_000_000, 1_000_000));
        let with_five = with_inserted_vertex(&square, 1, mid).unwrap();
        let moved = with_moved_vertex(&with_five, 2, pt(1_000_000, 1_000_000)).unwrap();
        let (pts, _) = courtyard_polygon(&moved).unwrap();
        assert_eq!(pts.len(), 5);
        assert!(is_simple_polygon(pts));

        // Moving vertex 2 across the bottom edge creates a self-intersection and must fail.
        assert!(with_moved_vertex(&square, 2, pt(0, -1_000_000)).is_err());
        // Removing a vertex from the 5-gon succeeds; removing down to 2 vertices fails.
        let back_to_four = with_removed_vertex(&moved, 2).unwrap();
        let triangle = with_removed_vertex(&back_to_four, 3).unwrap();
        assert_eq!(courtyard_polygon(&triangle).unwrap().0.len(), 3);
        assert!(with_removed_vertex(&triangle, 0).is_err());
    }

    /// GenerateCourtyard replaces existing courtyard primitives in one undoable transaction
    /// and rejects symbol documents.
    #[test]
    fn generate_courtyard_action_replaces_existing_in_one_undo_step_and_round_trips() {
        let mut symbol_session = Session::new(Item::new(Kind::Symbol).unwrap(), false).unwrap();
        assert!(
            symbol_session
                .execute(
                    0,
                    Action::GenerateCourtyard {
                        clearance_nm: DEFAULT_CLEARANCE_NM,
                        source: CourtyardSource::PadsAndBody,
                        stroke_nm: DEFAULT_STROKE_NM,
                        replace_existing: true,
                    },
                )
                .is_err()
        );

        let mut session = Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
        let rev = session
            .execute(
                0,
                Action::GenerateCourtyard {
                    clearance_nm: DEFAULT_CLEARANCE_NM,
                    source: CourtyardSource::PadsAndBody,
                    stroke_nm: DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        let first_count = session.snapshot().objects.values().filter(|p| is_courtyard_primitive(p)).count();
        assert_eq!(first_count, 1);

        // Regenerate with larger clearance: still exactly 1 courtyard primitive, one undo restores the first.
        let rev = session
            .execute(
                rev,
                Action::GenerateCourtyard {
                    clearance_nm: 500_000,
                    source: CourtyardSource::PadsAndBody,
                    stroke_nm: DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        assert_eq!(session.snapshot().objects.values().filter(|p| is_courtyard_primitive(p)).count(), 1);
        let generated_id = *session.snapshot().objects.iter().find(|(_, p)| is_courtyard_primitive(p)).unwrap().0;
        let (pts, _) = courtyard_polygon(&session.snapshot().objects[&generated_id]).unwrap();
        assert_eq!(metrics(pts).unwrap().width_nm, 2_600_000);

        // Round-trip through Item encode/decode and FootprintDefinition.
        let encoded = session.snapshot().encode().unwrap();
        let decoded = Item::decode(&encoded).unwrap();
        let def = decoded.footprint().unwrap();
        assert_eq!(def.artwork.len(), 1);
        assert_eq!(def.artwork[0].purpose, GraphicPurpose::Courtyard);

        // Undo restores the 0.25 mm courtyard in one step.
        session.execute(rev, Action::Undo).unwrap();
        let restored_id = *session.snapshot().objects.iter().find(|(_, p)| is_courtyard_primitive(p)).unwrap().0;
        let (pts, _) = courtyard_polygon(&session.snapshot().objects[&restored_id]).unwrap();
        assert_eq!(metrics(pts).unwrap().width_nm, 2_100_000);
    }
}
