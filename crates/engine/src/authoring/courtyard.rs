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

/// Default courtyard clearance: 0.25 mm.
pub const DEFAULT_CLEARANCE_NM: i64 = 250_000;
/// Default courtyard outline stroke width: 0.05 mm.
pub const DEFAULT_STROKE_NM: i64 = 50_000;
/// Maximum allowed courtyard clearance when generating an outline (10.0 mm).
pub const MAX_CLEARANCE_NM: i64 = 10_000_000;
/// Thinnest courtyard outline stroke (0.001 mm = 1,000 nm).
pub const MIN_STROKE_NM: i64 = 1_000;
/// Widest courtyard outline stroke (2 mm), the library graphic limit.
pub const MAX_STROKE_NM: i64 = crate::library_graphics::MAX_STROKE_NM;
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

/// Build a closed courtyard polygon primitive from `points` and `stroke_nm` after checking
/// polygon simplicity and footprint schema bounds, without re-checking the minimum stroke
/// enforced for new outlines so geometry-only edits can preserve a valid saved thin stroke.
fn build_courtyard_polygon(points: Vec<Point>, stroke_nm: i64) -> Result<Primitive, String> {
    validate_polygon(&points)?;
    let graphic = Graphic { geometry: Geometry::Polyline { points, closed: true }, stroke_nm, fill: Fill::None };
    graphic.validate(footprint::MAX_OFFSET_NM)?;
    Ok(Primitive::Artwork { artwork: footprint::Artwork { purpose: GraphicPurpose::Courtyard, graphic } })
}

/// Build and validate a closed courtyard polygon primitive from `points` and `stroke_nm`.
///
/// Rejects polygons with fewer than 3 or more than 64 vertices, duplicate consecutive
/// vertices, zero area, self-intersecting edges, out-of-bounds coordinates, or invalid
/// stroke widths.
pub fn make_courtyard_polygon(points: Vec<Point>, stroke_nm: i64) -> Result<Primitive, String> {
    validate_stroke(stroke_nm)?;
    build_courtyard_polygon(points, stroke_nm)
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
    validate_stroke(stroke_nm)?;
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
    build_courtyard_polygon(updated, stroke_nm)
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
    build_courtyard_polygon(updated, stroke_nm)
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
    build_courtyard_polygon(updated, stroke_nm)
}

/// Compute the midpoint of edge `edge_index` (wrapping from the last vertex to the first),
/// rounded to the nanometre grid. An odd coordinate sum rounds toward negative infinity on
/// both axes, so the rounding is the same on either side of the origin and the point is at
/// most half a nanometre from the true midpoint.
pub fn edge_midpoint(points: &[Point], edge_index: usize) -> Option<Point> {
    let a = points.get(edge_index)?;
    let b = points.get((edge_index + 1) % points.len())?;
    let half = |p: i64, q: i64| i64::try_from((i128::from(p) + i128::from(q)).div_euclid(2)).ok();
    Some(Point { x_nm: half(a.x_nm, b.x_nm)?, y_nm: half(a.y_nm, b.y_nm)? })
}

/// Validate a courtyard outline stroke width against [`MIN_STROKE_NM`]..=[`MAX_STROKE_NM`], with
/// the same message the dialog, the inspector and the live command report.
pub fn validate_stroke(stroke_nm: i64) -> Result<(), String> {
    if (MIN_STROKE_NM..=MAX_STROKE_NM).contains(&stroke_nm) { Ok(()) } else { Err("Stroke (mm) must be between 0.001 and 2 mm.".into()) }
}

/// Return whether every point in `points` lies within the footprint definition coordinate bounds.
fn points_within_bounds(points: &[Point]) -> bool {
    let limit = footprint::MAX_OFFSET_NM;
    points.iter().all(|p| (-limit..=limit).contains(&p.x_nm) && (-limit..=limit).contains(&p.y_nm))
}

/// Validate that `points` forms a bounded, non-degenerate, non-self-intersecting closed polygon.
pub fn validate_polygon(points: &[Point]) -> Result<(), String> {
    if points.len() < MIN_VERTICES {
        return Err("A courtyard outline needs at least three vertices.".into());
    }
    if points.len() > MAX_VERTICES {
        return Err(format!("A courtyard outline can have at most {MAX_VERTICES} vertices."));
    }
    if !points_within_bounds(points) {
        return Err("Drawing extends outside the definition bounds.".into());
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
    if n < MIN_VERTICES || !points_within_bounds(points) || doubled_signed_area(points) == 0 {
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
    if !points_within_bounds(points) {
        return None;
    }
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

/// Exact endpoints and any cardinal-angle extrema (`k * 90°`) within an arc's sweep.
fn arc_extents(center: Point, radius_nm: i64, start_degrees: i16, sweep_degrees: i16) -> Vec<Point> {
    let start = i32::from(start_degrees);
    let end = start + i32::from(sweep_degrees);
    let (lo, hi) = if start <= end { (start, end) } else { (end, start) };
    let point_at = |degrees: i32| {
        let a = f64::from(degrees).to_radians();
        Point {
            x_nm: center.x_nm.saturating_add((radius_nm as f64 * a.cos()).round() as i64),
            y_nm: center.y_nm.saturating_add((radius_nm as f64 * a.sin()).round() as i64),
        }
    };
    let mut points = vec![point_at(start), point_at(end)];
    let floor_cardinal = lo.div_euclid(90) * 90;
    let first_cardinal = if floor_cardinal < lo { floor_cardinal + 90 } else { floor_cardinal };
    for step in 0..4 {
        let angle = first_cardinal + step * 90;
        if angle > hi {
            break;
        }
        let (dx, dy) = match angle.rem_euclid(360) {
            0 => (radius_nm, 0),
            90 => (0, radius_nm),
            180 => (-radius_nm, 0),
            _ => (0, -radius_nm),
        };
        points.push(Point { x_nm: center.x_nm.saturating_add(dx), y_nm: center.y_nm.saturating_add(dy) });
    }
    points
}

/// Collect boundary points of a footprint primitive for courtyard generation.
/// Fabrication artwork bounds include half their stroke width so clearance is measured from the
/// visible outer body edge rather than the stroke centerline.
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
        Primitive::Artwork { artwork } => {
            let centerline = match &artwork.graphic.geometry {
                Geometry::Text { .. } => return Vec::new(),
                Geometry::Arc { center, radius_nm, start_degrees, sweep_degrees } => arc_extents(*center, *radius_nm, *start_degrees, *sweep_degrees),
                _ => artwork.graphic.paths().into_iter().flatten().collect(),
            };
            let Some(&first) = centerline.first() else {
                return Vec::new();
            };
            let (mut min_x, mut min_y, mut max_x, mut max_y) = (first.x_nm, first.y_nm, first.x_nm, first.y_nm);
            for &point in centerline.iter().skip(1) {
                min_x = min_x.min(point.x_nm);
                min_y = min_y.min(point.y_nm);
                max_x = max_x.max(point.x_nm);
                max_y = max_y.max(point.y_nm);
            }
            let half_stroke = artwork.graphic.stroke_nm.max(0).saturating_add(1) / 2;
            vec![
                Point { x_nm: min_x.saturating_sub(half_stroke), y_nm: min_y.saturating_sub(half_stroke) },
                Point { x_nm: max_x.saturating_add(half_stroke), y_nm: max_y.saturating_add(half_stroke) },
            ]
        }
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
                            align: crate::schematic::TextAlign::Middle,
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

    /// Fabrication arc courtyard bounds include exact cardinal extrema plus half the stroke
    /// width within the sweep, not only tessellated sample points (both positive and negative
    /// sweeps/starts).
    #[test]
    fn generate_from_fabrication_arc_includes_exact_cardinal_extrema() {
        let radius = 50_000_000;
        let stroke = 120_000;
        let half_stroke = stroke / 2;
        for (start_degrees, sweep_degrees) in [(1, 180), (181, -180)] {
            let mut item = Item::new(Kind::Footprint).unwrap();
            item.objects.insert(
                92,
                Primitive::Artwork {
                    artwork: footprint::Artwork {
                        purpose: GraphicPurpose::Fabrication,
                        graphic: Graphic {
                            geometry: Geometry::Arc { center: pt(0, 0), radius_nm: radius, start_degrees, sweep_degrees },
                            stroke_nm: stroke,
                            fill: Fill::None,
                        },
                    },
                },
            );
            for clearance in [0, DEFAULT_CLEARANCE_NM] {
                let primitive = generate(&item, clearance, CourtyardSource::PadsAndBody, DEFAULT_STROKE_NM).unwrap();
                let (points, _) = courtyard_polygon(&primitive).unwrap();
                let min_x = points.iter().map(|p| p.x_nm).min().unwrap();
                let max_y = points.iter().map(|p| p.y_nm).max().unwrap();
                assert_eq!(min_x, -radius - half_stroke - clearance);
                assert_eq!(max_y, radius + half_stroke + clearance);
            }
        }

        // Negative start angle spanning -90°, 0° and 90° hits all three exact cardinal extrema.
        let mut neg = Item::new(Kind::Footprint).unwrap();
        neg.objects.insert(
            93,
            Primitive::Artwork {
                artwork: footprint::Artwork {
                    purpose: GraphicPurpose::Fabrication,
                    graphic: Graphic {
                        geometry: Geometry::Arc { center: pt(0, 0), radius_nm: radius, start_degrees: -100, sweep_degrees: 200 },
                        stroke_nm: stroke,
                        fill: Fill::None,
                    },
                },
            },
        );
        let primitive = generate(&neg, 0, CourtyardSource::PadsAndBody, DEFAULT_STROKE_NM).unwrap();
        let (points, _) = courtyard_polygon(&primitive).unwrap();
        assert_eq!(points.iter().map(|p| p.x_nm).max().unwrap(), radius + half_stroke);
        assert_eq!(points.iter().map(|p| p.y_nm).min().unwrap(), -radius - half_stroke);
        assert_eq!(points.iter().map(|p| p.y_nm).max().unwrap(), radius + half_stroke);
    }

    /// Fabrication artwork bounds include half their stroke width so a thick-stroked body
    /// rectangle (2 mm centerline, 0.60 mm stroke => visible edge at ±1.30 mm) with 0.25 mm
    /// clearance puts the courtyard centerline at ±1.55 mm (0.25 mm outside the visible edge).
    #[test]
    fn generate_from_thick_fabrication_stroke_clears_visible_body_edge() {
        let mut item = Item::new(Kind::Footprint).unwrap();
        item.objects.clear();
        item.objects.insert(
            1,
            Primitive::Artwork {
                artwork: footprint::Artwork {
                    purpose: GraphicPurpose::Fabrication,
                    graphic: Graphic {
                        geometry: Geometry::Rectangle { min: pt(-1_000_000, -1_000_000), max: pt(1_000_000, 1_000_000) },
                        stroke_nm: 600_000,
                        fill: Fill::None,
                    },
                },
            },
        );
        let primitive = generate(&item, 250_000, CourtyardSource::PadsAndBody, DEFAULT_STROKE_NM).unwrap();
        let (points, _) = courtyard_polygon(&primitive).unwrap();
        assert_eq!(points, &[pt(-1_550_000, -1_550_000), pt(1_550_000, -1_550_000), pt(1_550_000, 1_550_000), pt(-1_550_000, 1_550_000)]);
        let m = metrics(points).unwrap();
        assert_eq!(m.width_nm, 3_100_000);
        assert_eq!(m.height_nm, 3_100_000);
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

    /// One stroke check with one message guards generation and every polygon edit, at the
    /// reported 0.001 mm (1,000 nm) to 2 mm limits.
    #[test]
    fn courtyard_stroke_range_is_shared_and_reported_with_the_ui_message() {
        let message = "Stroke (mm) must be between 0.001 and 2 mm.";
        assert_eq!((MIN_STROKE_NM, MAX_STROKE_NM), (1_000, 2_000_000));
        assert!(validate_stroke(MIN_STROKE_NM).is_ok() && validate_stroke(MAX_STROKE_NM).is_ok());
        assert_eq!(validate_stroke(0).unwrap_err(), message);
        assert_eq!(validate_stroke(MIN_STROKE_NM - 1).unwrap_err(), message);
        assert_eq!(validate_stroke(MAX_STROKE_NM + 1).unwrap_err(), message);
        let item = Item::new(Kind::Footprint).unwrap();
        assert_eq!(generate(&item, DEFAULT_CLEARANCE_NM, CourtyardSource::PadsAndBody, MIN_STROKE_NM - 1).unwrap_err(), message);
        assert_eq!(generate(&item, DEFAULT_CLEARANCE_NM, CourtyardSource::PadsAndBody, 5_000_000).unwrap_err(), message);
        let square = vec![pt(0, 0), pt(1_000_000, 0), pt(1_000_000, 1_000_000), pt(0, 1_000_000)];
        assert_eq!(make_courtyard_polygon(square.clone(), 0).unwrap_err(), message);
        assert_eq!(make_courtyard_polygon(square.clone(), MIN_STROKE_NM - 1).unwrap_err(), message);
        assert!(make_courtyard_polygon(square.clone(), MIN_STROKE_NM).is_ok());
        assert!(make_courtyard_polygon(square, MAX_STROKE_NM).is_ok());
    }

    /// Odd coordinate sums round toward negative infinity on both sides of the origin, so a
    /// diagonal-edge midpoint is the nanometre grid point at most half a nanometre from the true
    /// midpoint and the inserted vertex keeps the outline simple.
    #[test]
    fn edge_midpoint_rounds_odd_sums_to_the_nanometre_grid_consistently() {
        // Even sums are exact; odd sums floor on both signs (truncation would give -1 and 0).
        assert_eq!(edge_midpoint(&[pt(-4, 6), pt(2, -2)], 0), Some(pt(-1, 2)));
        assert_eq!(edge_midpoint(&[pt(-3, 1), pt(0, 0)], 0), Some(pt(-2, 0)));
        assert_eq!(edge_midpoint(&[pt(1, 2), pt(2, 5)], 0), Some(pt(1, 3)));
        // The wrapping edge from the last vertex back to the first, and an extreme sum.
        assert_eq!(edge_midpoint(&[pt(0, 0), pt(10, 0), pt(-7, -9)], 2), Some(pt(-4, -5)));
        assert_eq!(edge_midpoint(&[pt(i64::MAX, i64::MIN), pt(i64::MAX, i64::MIN)], 0), Some(pt(i64::MAX, i64::MIN)));
        assert_eq!(edge_midpoint(&[], 0), None);
        assert_eq!(edge_midpoint(&[pt(0, 0), pt(1, 1)], 2), None);

        // A diagonal edge with odd sums on both axes, in negative coordinates.
        let primitive =
            make_courtyard_polygon(vec![pt(-1_000_001, -1_000_000), pt(0, -1_000_000), pt(0, 0), pt(-1_000_000, -3)], DEFAULT_STROKE_NM).unwrap();
        let (points, _) = courtyard_polygon(&primitive).unwrap();
        let mid = edge_midpoint(points, 3).unwrap();
        assert_eq!(mid, pt(-1_000_001, -500_002));
        let inserted = with_inserted_vertex(&primitive, 3, mid).unwrap();
        let (after, _) = courtyard_polygon(&inserted).unwrap();
        assert_eq!(after.len(), 5);
        assert!(is_simple_polygon(after));
    }

    /// Pads-only generation ignores a wider fabrication outline.
    #[test]
    fn generate_pads_only_ignores_larger_fabrication_body() {
        let item = Item::new(Kind::Footprint).unwrap();
        let primitive = generate(&item, 100_000, CourtyardSource::PadsOnly, DEFAULT_STROKE_NM).unwrap();
        let (points, _) = courtyard_polygon(&primitive).unwrap();
        assert_eq!(points, &[pt(-850_000, -400_000), pt(850_000, -400_000), pt(850_000, 400_000), pt(-850_000, 400_000),]);
    }

    /// Out-of-bounds coordinates (including extreme `i64::MIN..=i64::MAX` corners that would
    /// overflow `i128` polygon area arithmetic) are rejected before area or intersection math.
    #[test]
    fn validate_polygon_and_make_reject_out_of_bounds_coordinates_without_overflow() {
        let message = "Drawing extends outside the definition bounds.";
        let extreme = vec![pt(i64::MIN, i64::MIN), pt(i64::MAX, i64::MIN), pt(i64::MAX, i64::MAX), pt(i64::MIN, i64::MAX)];
        assert_eq!(validate_polygon(&extreme).unwrap_err(), message);
        assert_eq!(make_courtyard_polygon(extreme, DEFAULT_STROKE_NM).unwrap_err(), message);

        let limit = footprint::MAX_OFFSET_NM;
        let just_outside = vec![pt(0, 0), pt(limit + 1, 0), pt(limit + 1, 1_000_000), pt(0, 1_000_000)];
        assert_eq!(validate_polygon(&just_outside).unwrap_err(), message);
        assert_eq!(make_courtyard_polygon(just_outside, DEFAULT_STROKE_NM).unwrap_err(), message);

        let negative_outside = vec![pt(-limit - 1, 0), pt(0, 0), pt(0, 1_000_000)];
        assert_eq!(validate_polygon(&negative_outside).unwrap_err(), message);

        let at_limit = vec![pt(-limit, -limit), pt(limit, -limit), pt(limit, limit), pt(-limit, limit)];
        assert!(validate_polygon(&at_limit).is_ok());
        assert!(make_courtyard_polygon(at_limit, DEFAULT_STROKE_NM).is_ok());
    }

    /// Public polygon queries reject out-of-bounds coordinates (including extreme
    /// `i64::MIN..=i64::MAX` corners) before arithmetic, while keeping valid bounded results.
    #[test]
    fn metrics_and_is_simple_polygon_guard_coordinate_bounds_before_arithmetic() {
        let extreme = [pt(i64::MIN, i64::MIN), pt(i64::MAX, i64::MIN), pt(i64::MAX, i64::MAX), pt(i64::MIN, i64::MAX)];
        assert!(!is_simple_polygon(&extreme));
        assert_eq!(metrics(&extreme), None);

        let limit = footprint::MAX_OFFSET_NM;
        let just_outside = [pt(0, 0), pt(limit + 1, 0), pt(limit + 1, 1_000_000), pt(0, 1_000_000)];
        assert!(!is_simple_polygon(&just_outside));
        assert_eq!(metrics(&just_outside), None);

        let negative_outside = [pt(-limit - 1, 0), pt(0, 0), pt(0, 1_000_000)];
        assert!(!is_simple_polygon(&negative_outside));
        assert_eq!(metrics(&negative_outside), None);

        let at_limit = [pt(-limit, -limit), pt(limit, -limit), pt(limit, limit), pt(-limit, limit)];
        assert!(is_simple_polygon(&at_limit));
        let bounded = metrics(&at_limit).unwrap();
        assert_eq!(bounded.min, pt(-limit, -limit));
        assert_eq!(bounded.max, pt(limit, limit));
        assert_eq!(bounded.width_nm, 2 * limit);
        assert_eq!(bounded.height_nm, 2 * limit);
        assert_eq!(bounded.area_nm2, i128::from(2 * limit) * i128::from(2 * limit));
        assert_eq!(bounded.vertex_count, 4);
        assert!(bounded.is_simple);
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

    /// Raw insert and replace actions cannot commit a self-intersecting courtyard outline, and a
    /// refused edit leaves the draft and its revision unchanged.
    #[test]
    fn raw_insert_and_replace_refuse_a_self_intersecting_courtyard() {
        let bowtie = Primitive::Artwork {
            artwork: footprint::Artwork {
                purpose: GraphicPurpose::Courtyard,
                graphic: Graphic {
                    geometry: Geometry::Polyline {
                        // An uneven bowtie: its first and third edges cross, but its signed area
                        // is not zero, so the self-intersection check is what refuses it.
                        points: vec![pt(0, 0), pt(2_000_000, 2_000_000), pt(2_000_000, 0), pt(0, 1_000_000)],
                        closed: true,
                    },
                    stroke_nm: DEFAULT_STROKE_NM,
                    fill: Fill::None,
                },
            },
        };
        let mut session = Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
        let before = session.snapshot();
        let error = session.execute(0, Action::Insert { primitive: bowtie.clone() }).unwrap_err();
        assert_eq!(error, "A courtyard outline cannot intersect itself.");
        assert_eq!(session.snapshot(), before);
        assert_eq!(session.revision(), 0);

        let square = make_courtyard_polygon(vec![pt(0, 0), pt(2_000_000, 0), pt(2_000_000, 2_000_000), pt(0, 2_000_000)], DEFAULT_STROKE_NM).unwrap();
        let rev = session.execute(0, Action::Insert { primitive: square }).unwrap();
        let accepted = session.snapshot();
        let id = *accepted.objects.iter().find(|(_, p)| is_courtyard_primitive(p)).unwrap().0;
        let error = session.execute(rev, Action::Replace { object: id, primitive: bowtie }).unwrap_err();
        assert_eq!(error, "A courtyard outline cannot intersect itself.");
        assert_eq!(session.snapshot(), accepted);
        assert_eq!(session.revision(), rev);
    }

    /// Raw insert and replace actions enforce the 0.001 mm (1,000 nm) courtyard minimum stroke,
    /// leaving the draft and its revision unchanged when refused.
    #[test]
    fn raw_insert_and_replace_enforce_courtyard_minimum_stroke() {
        let message = "Stroke (mm) must be between 0.001 and 2 mm.";
        let square = |stroke_nm| Primitive::Artwork {
            artwork: footprint::Artwork {
                purpose: GraphicPurpose::Courtyard,
                graphic: Graphic {
                    geometry: Geometry::Polyline {
                        points: vec![pt(0, 0), pt(2_000_000, 0), pt(2_000_000, 2_000_000), pt(0, 2_000_000)],
                        closed: true,
                    },
                    stroke_nm,
                    fill: Fill::None,
                },
            },
        };
        let mut session = Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
        let before = session.snapshot();
        let error = session.execute(0, Action::Insert { primitive: square(MIN_STROKE_NM - 1) }).unwrap_err();
        assert_eq!(error, message);
        assert_eq!(session.snapshot(), before);
        assert_eq!(session.revision(), 0);

        let rev = session.execute(0, Action::Insert { primitive: square(MIN_STROKE_NM) }).unwrap();
        assert_eq!(rev, 1);
        let accepted = session.snapshot();
        let id = *accepted.objects.iter().find(|(_, p)| is_courtyard_primitive(p)).unwrap().0;
        let error = session.execute(rev, Action::Replace { object: id, primitive: square(MIN_STROKE_NM - 1) }).unwrap_err();
        assert_eq!(error, message);
        assert_eq!(session.snapshot(), accepted);
        assert_eq!(session.revision(), rev);
    }

    /// Raw insert and replace actions also enforce the 0.001 mm (1,000 nm) courtyard minimum
    /// stroke for non-polyline courtyard artwork such as rectangles.
    #[test]
    fn raw_insert_and_replace_enforce_courtyard_minimum_stroke_for_rectangles() {
        let message = "Stroke (mm) must be between 0.001 and 2 mm.";
        let rect = |stroke_nm| Primitive::Artwork {
            artwork: footprint::Artwork {
                purpose: GraphicPurpose::Courtyard,
                graphic: Graphic { geometry: Geometry::Rectangle { min: pt(0, 0), max: pt(2_000_000, 2_000_000) }, stroke_nm, fill: Fill::None },
            },
        };
        let mut session = Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
        let before = session.snapshot();
        let error = session.execute(0, Action::Insert { primitive: rect(MIN_STROKE_NM - 1) }).unwrap_err();
        assert_eq!(error, message);
        assert_eq!(session.snapshot(), before);
        assert_eq!(session.revision(), 0);

        let rev = session.execute(0, Action::Insert { primitive: rect(MIN_STROKE_NM) }).unwrap();
        assert_eq!(rev, 1);
        let accepted = session.snapshot();
        let id = *accepted.objects.iter().find(|(_, p)| is_courtyard_primitive(p)).unwrap().0;
        let error = session.execute(rev, Action::Replace { object: id, primitive: rect(MIN_STROKE_NM - 1) }).unwrap_err();
        assert_eq!(error, message);
        assert_eq!(session.snapshot(), accepted);
        assert_eq!(session.revision(), rev);
    }

    /// A valid saved footprint with a 500 nm courtyard stroke stays editable for plain moves and
    /// geometry-only vertex edits, preserving 500 nm, while an explicit stroke change below
    /// `MIN_STROKE_NM` is still refused.
    #[test]
    fn thin_saved_courtyard_survives_move_and_vertex_edit_while_explicit_stroke_change_is_refused() {
        let mut setup = Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
        let initial =
            make_courtyard_polygon(vec![pt(0, 0), pt(2_000_000, 0), pt(2_000_000, 2_000_000), pt(0, 2_000_000)], DEFAULT_STROKE_NM).unwrap();
        setup.execute(0, Action::Insert { primitive: initial }).unwrap();
        let mut saved = (*setup.snapshot()).clone();
        let id = *saved.objects.iter().find(|(_, p)| is_courtyard_primitive(p)).unwrap().0;
        let thin_stroke = 500;
        if let Some(Primitive::Artwork { artwork }) = saved.objects.get_mut(&id) {
            artwork.graphic.stroke_nm = thin_stroke;
        }
        let bytes = saved.encode().unwrap();
        let loaded = Item::decode(&bytes).unwrap();
        assert_eq!(courtyard_polygon(&loaded.objects[&id]).unwrap().1, thin_stroke);

        let mut session = Session::new(loaded, false).unwrap();
        let rev = session
            .execute(
                0,
                Action::Transform {
                    objects: vec![id],
                    transform: crate::library_graphics::Transform {
                        pivot: pt(0, 0),
                        offset: pt(500_000, -250_000),
                        quarter_turns: 0,
                        mirror_x: false,
                        mirror_y: false,
                    },
                },
            )
            .unwrap();
        assert_eq!(rev, 1);
        let after_move = session.snapshot();
        let (moved_pts, moved_stroke) = courtyard_polygon(&after_move.objects[&id]).unwrap();
        assert_eq!(moved_stroke, thin_stroke);
        assert_eq!(moved_pts[0], pt(500_000, -250_000));

        let vertex_edited = with_moved_vertex(&after_move.objects[&id], 1, pt(3_000_000, -250_000)).unwrap();
        let rev = session.execute(rev, Action::Replace { object: id, primitive: vertex_edited }).unwrap();
        assert_eq!(rev, 2);
        let after_vertex = session.snapshot();
        let (vertex_pts, vertex_stroke) = courtyard_polygon(&after_vertex.objects[&id]).unwrap();
        assert_eq!(vertex_stroke, thin_stroke);
        assert_eq!(vertex_pts[1], pt(3_000_000, -250_000));

        // Explicit stroke change below 1,000 nm (even from 500 nm to 600 nm) is still refused.
        let mut changed_stroke = after_vertex.objects[&id].clone();
        if let Primitive::Artwork { artwork } = &mut changed_stroke {
            artwork.graphic.stroke_nm = 600;
        }
        let error = session.execute(rev, Action::Replace { object: id, primitive: changed_stroke }).unwrap_err();
        assert_eq!(error, "Stroke (mm) must be between 0.001 and 2 mm.");
        assert_eq!(session.snapshot(), after_vertex);
        assert_eq!(session.revision(), rev);
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
        let generated = session.snapshot();
        let (pts, _) = courtyard_polygon(&generated.objects[&generated_id]).unwrap();
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
        let restored = session.snapshot();
        let (pts, _) = courtyard_polygon(&restored.objects[&restored_id]).unwrap();
        assert_eq!(metrics(pts).unwrap().width_nm, 2_100_000);
    }

    /// The live `generate_courtyard` operation needs only a clearance: source, stroke and
    /// replacement take their documented defaults, and unknown fields are refused.
    #[test]
    fn generate_courtyard_action_parses_from_live_json_with_defaults() {
        let action: Action = serde_json::from_value(serde_json::json!({"operation": "generate_courtyard", "clearance_nm": 250_000})).unwrap();
        let Action::GenerateCourtyard { clearance_nm, source, stroke_nm, replace_existing } = action else {
            panic!("expected a courtyard generation action, found {action:?}");
        };
        assert_eq!(clearance_nm, DEFAULT_CLEARANCE_NM);
        assert_eq!(source, CourtyardSource::PadsAndBody);
        assert_eq!(stroke_nm, DEFAULT_STROKE_NM);
        assert!(replace_existing);
        let explicit: Action =
            serde_json::from_value(serde_json::json!({"operation": "generate_courtyard", "clearance_nm": 100_000, "source": "pads_only", "stroke_nm": 120_000, "replace_existing": false})).unwrap();
        assert!(matches!(
            explicit,
            Action::GenerateCourtyard { clearance_nm: 100_000, source: CourtyardSource::PadsOnly, stroke_nm: 120_000, replace_existing: false }
        ));
        assert!(
            serde_json::from_value::<Action>(serde_json::json!({"operation": "generate_courtyard", "clearance_nm": 250_000, "margin_nm": 1}))
                .is_err()
        );
        assert!(serde_json::from_value::<Action>(serde_json::json!({"operation": "generate_courtyard"})).is_err());
    }
}
