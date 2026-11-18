//! Courtyard overlap decisions shared by the synchronous check and the resumable job.
//!
//! A footprint's courtyard is the union of its courtyard graphics on its placement side.
//! Rectangles and closed polylines are decided exactly from their transformed corners, using
//! integer orientation tests. A finding requires the interiors of two same-side courtyards to
//! overlap, which includes containment; boundaries that only touch along an edge or at a vertex
//! are allowed. Bounding boxes only select candidate pairs.
//!
//! Circles and arcs (drawn as polygonized curves), open lines or polylines, text, and degenerate
//! or self-intersecting polygons are unsupported. Their combined bounds keep the previous
//! conservative comparison, and such a finding says that it is conservative. Footprints without
//! courtyard graphics are not checked.
use super::{Kind, Violation};
use crate::{
    board::{definition::GraphicPurpose, view::FootprintView},
    schematic::Point,
};
use std::cmp::Ordering;

/// `[min_x, min_y, max_x, max_y]` in board nanometers.
type Bounds = [i64; 4];

/// Message for an exact interior overlap; kept stable for existing clients.
const OVERLAP: &str = "Footprint courtyards overlap";
/// Message when unsupported courtyard geometry was compared by its bounds.
const CONSERVATIVE: &str = "Footprint courtyard bounds overlap; unsupported courtyard geometry is compared conservatively";

/// One footprint's courtyard captured from a resolved placement, independent of the snapshot.
#[derive(Clone, Debug)]
pub(super) struct Courtyard {
    id: String,
    bottom: bool,
    /// Bounds of every courtyard graphic: the pair candidate filter.
    bounds: Bounds,
    /// Supported graphics as simple polygons with positive orientation.
    polygons: Vec<Polygon>,
    /// Combined bounds of graphics that cannot be decided exactly.
    unsupported: Option<Bounds>,
}

impl Courtyard {
    /// Capture the placement's transformed courtyard graphics. Returns `None` when the
    /// footprint has no courtyard points, so it takes no part in courtyard checking.
    pub(super) fn capture(footprint: &FootprintView) -> Option<Self> {
        let mut polygons = Vec::new();
        let mut unsupported = None;
        for graphic in footprint.graphics.iter().filter(|graphic| graphic.purpose == GraphicPurpose::Courtyard) {
            match (graphic.exact && graphic.closed).then(|| Polygon::new(&graphic.points)).flatten() {
                Some(polygon) => polygons.push(polygon),
                None => unsupported = join(unsupported, bounds(&graphic.points)),
            }
        }
        let all = polygons.iter().map(|polygon| polygon.bounds).fold(unsupported, |all, b| join(all, Some(b)))?;
        Some(Self { id: footprint.id.clone(), bottom: footprint.bottom, bounds: all, polygons, unsupported })
    }

    /// Report the pair when same-side courtyards overlap. An exact interior overlap of
    /// supported polygons wins; otherwise unsupported bounds are compared conservatively
    /// against the other footprint's whole courtyard. Items keep `self` before `other`.
    pub(super) fn overlap(&self, other: &Self) -> Option<Violation> {
        if self.bottom != other.bottom || !bounds_overlap(self.bounds, other.bounds) {
            return None;
        }
        let exact = self
            .polygons
            .iter()
            .flat_map(|a| other.polygons.iter().map(move |b| (a, b)))
            .filter(|(a, b)| bounds_overlap(a.bounds, b.bounds))
            .find_map(|(a, b)| a.overlap_witness(b));
        let (message, at) = match exact {
            Some(at) => (OVERLAP, at),
            None => {
                let pairs = [(self.unsupported, other.bounds), (other.unsupported, self.bounds)];
                let (a, b) = pairs.into_iter().find_map(|(a, b)| a.filter(|a| bounds_overlap(*a, b)).map(|a| (a, b)))?;
                (CONSERVATIVE, Point { x_nm: (a[0].max(b[0]) + a[2].min(b[2])) / 2, y_nm: (a[1].max(b[1]) + a[3].min(b[3])) / 2 })
            }
        };
        Some(Violation {
            kind: Kind::CourtyardOverlap,
            message: message.into(),
            at,
            items: vec![self.id.clone(), other.id.clone()],
            nets: Vec::new(),
        })
    }
}

/// A simple polygon whose interior lies on the positive side of each edge (`cross > 0`).
#[derive(Clone, Debug)]
struct Polygon {
    points: Vec<Point>,
    bounds: Bounds,
}

/// Where a point lies relative to a polygon, decided exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Location {
    Outside,
    Inside,
    Vertex(usize),
    /// Strictly between the ends of edge `i`, from vertex `i` to vertex `i + 1`.
    Edge(usize),
}

impl Polygon {
    /// Normalize a closed path: drop repeated points (including a closing copy of the first
    /// point) and straight-through vertices, and orient the interior positively. Returns `None`
    /// for fewer than three corners, zero area, a zero-width spike or a self-intersection.
    fn new(path: &[Point]) -> Option<Self> {
        let mut points: Vec<Point> = Vec::with_capacity(path.len());
        for &point in path {
            if points.last() != Some(&point) {
                points.push(point);
            }
        }
        while points.len() > 1 && points.first() == points.last() {
            points.pop();
        }
        let n = points.len();
        if n < 3 {
            return None;
        }
        let mut kept = Vec::with_capacity(n);
        for i in 0..n {
            let (u, v, w) = (points[(i + n - 1) % n], points[i], points[(i + 1) % n]);
            if cross(u, v, w) == 0 {
                if dot(u, v, w) > 0 {
                    return None;
                }
                continue;
            }
            kept.push(v);
        }
        if kept.len() < 3 {
            return None;
        }
        let area: i128 = (0..kept.len()).map(|i| det(kept[i], kept[(i + 1) % kept.len()])).sum();
        match area.cmp(&0) {
            Ordering::Equal => return None,
            Ordering::Less => kept.reverse(),
            Ordering::Greater => {}
        }
        let polygon = Self { bounds: bounds(&kept)?, points: kept };
        polygon.is_simple().then_some(polygon)
    }

    /// Edge `i` runs from vertex `i` to the next vertex, wrapping around.
    fn edge(&self, i: usize) -> (Point, Point) {
        (self.points[i], self.points[(i + 1) % self.points.len()])
    }

    /// Non-adjacent edges must not meet; adjacent edges meet only at their shared corner
    /// because straight-through and spike vertices were removed.
    fn is_simple(&self) -> bool {
        let n = self.points.len();
        (0..n).all(|i| {
            (i + 2..n).filter(|&j| !(i == 0 && j == n - 1)).all(|j| {
                let ((a, b), (c, d)) = (self.edge(i), self.edge(j));
                !segments_meet(a, b, c, d)
            })
        })
    }

    /// Classify `p` with exact integer predicates; boundary contact is distinct from interior.
    fn locate(&self, p: Point) -> Location {
        if let Some(i) = self.points.iter().position(|&v| v == p) {
            return Location::Vertex(i);
        }
        let mut winding = 0i32;
        for i in 0..self.points.len() {
            let (a, b) = self.edge(i);
            let side = cross(a, b, p);
            if side == 0 && within(a, b, p) {
                return Location::Edge(i);
            }
            if a.y_nm <= p.y_nm {
                if b.y_nm > p.y_nm && side > 0 {
                    winding += 1;
                }
            } else if b.y_nm <= p.y_nm && side < 0 {
                winding -= 1;
            }
        }
        if winding == 0 { Location::Outside } else { Location::Inside }
    }

    /// The open interior sector at a vertex, from the outgoing edge round to the incoming one.
    fn vertex_sector(&self, i: usize) -> Sector {
        let n = self.points.len();
        let (u, v, w) = (self.points[(i + n - 1) % n], self.points[i], self.points[(i + 1) % n]);
        Sector { start: direction(v, w), end: direction(v, u) }
    }

    /// The open interior half-plane at a point strictly inside edge `i`.
    fn edge_sector(&self, i: usize) -> Sector {
        let (a, b) = self.edge(i);
        Sector { start: direction(a, b), end: direction(b, a) }
    }

    /// The interior sector of this polygon at `p`, when `p` lies on its boundary.
    fn sector_at(&self, location: Location) -> Option<Sector> {
        match location {
            Location::Vertex(i) => Some(self.vertex_sector(i)),
            Location::Edge(i) => Some(self.edge_sector(i)),
            Location::Inside | Location::Outside => None,
        }
    }

    /// A board point inside both interiors, or `None` when the polygons are disjoint or only
    /// touch. Interiors overlap exactly when edges cross properly, a vertex of one lies strictly
    /// inside the other, or the two interior sectors overlap at a boundary contact point.
    fn overlap_witness(&self, other: &Self) -> Option<Point> {
        for i in 0..self.points.len() {
            let (a, b) = self.edge(i);
            for j in 0..other.points.len() {
                let (c, d) = other.edge(j);
                if crosses_properly(a, b, c, d) {
                    return Some(intersection(a, b, c, d));
                }
            }
        }
        for (polygon, against) in [(self, other), (other, self)] {
            for (i, &v) in polygon.points.iter().enumerate() {
                let location = against.locate(v);
                if location == Location::Inside {
                    return Some(v);
                }
                if let Some(sector) = against.sector_at(location)
                    && sector.overlaps(&polygon.vertex_sector(i))
                {
                    return Some(v);
                }
            }
        }
        None
    }
}

/// An open angular sector swept in the positive (`cross > 0`) direction from `start` to `end`.
/// Its angle is strictly between zero and a full turn.
#[derive(Clone, Copy, Debug)]
struct Sector {
    start: (i128, i128),
    end: (i128, i128),
}

impl Sector {
    /// Whether direction `d` lies strictly inside the open sector.
    fn contains(&self, d: (i128, i128)) -> bool {
        compare_from(self.start, d, self.start) == Ordering::Greater && compare_from(self.start, d, self.end) == Ordering::Less
    }

    /// Two open sectors at one point overlap exactly when they share a start direction or one
    /// start lies strictly inside the other sector. Sectors meeting only along a ray do not.
    fn overlaps(&self, other: &Self) -> bool {
        same_direction(self.start, other.start) || self.contains(other.start) || other.contains(self.start)
    }
}

/// Compare the positive angles of `x` and `y` measured from `origin`, each in `[0, 2π)`.
fn compare_from(origin: (i128, i128), x: (i128, i128), y: (i128, i128)) -> Ordering {
    let half = |d: (i128, i128)| {
        let c = vcross(origin, d);
        u8::from(!(c > 0 || (c == 0 && vdot(origin, d) > 0)))
    };
    half(x).cmp(&half(y)).then_with(|| 0.cmp(&vcross(x, y)))
}

/// Same direction, not merely parallel.
fn same_direction(a: (i128, i128), b: (i128, i128)) -> bool {
    vcross(a, b) == 0 && vdot(a, b) > 0
}

/// The direction vector from `a` to `b`.
fn direction(a: Point, b: Point) -> (i128, i128) {
    (i128::from(b.x_nm) - i128::from(a.x_nm), i128::from(b.y_nm) - i128::from(a.y_nm))
}

fn vcross(a: (i128, i128), b: (i128, i128)) -> i128 {
    a.0 * b.1 - a.1 * b.0
}

fn vdot(a: (i128, i128), b: (i128, i128)) -> i128 {
    a.0 * b.0 + a.1 * b.1
}

/// Orientation of `p` relative to the line from `a` to `b`.
fn cross(a: Point, b: Point, p: Point) -> i128 {
    vcross(direction(a, b), direction(a, p))
}

/// Dot product of the directions from `v` to `u` and from `v` to `w`.
fn dot(u: Point, v: Point, w: Point) -> i128 {
    vdot(direction(v, u), direction(v, w))
}

/// Twice the signed area contribution of one edge.
fn det(a: Point, b: Point) -> i128 {
    i128::from(a.x_nm) * i128::from(b.y_nm) - i128::from(b.x_nm) * i128::from(a.y_nm)
}

/// Whether a point already collinear with segment `a`–`b` lies within its closed extent.
fn within(a: Point, b: Point, p: Point) -> bool {
    a.x_nm.min(b.x_nm) <= p.x_nm && p.x_nm <= a.x_nm.max(b.x_nm) && a.y_nm.min(b.y_nm) <= p.y_nm && p.y_nm <= a.y_nm.max(b.y_nm)
}

/// Each segment's ends lie strictly on opposite sides of the other's line.
fn crosses_properly(a: Point, b: Point, c: Point, d: Point) -> bool {
    cross(a, b, c).signum() * cross(a, b, d).signum() < 0 && cross(c, d, a).signum() * cross(c, d, b).signum() < 0
}

/// Closed segments share at least one point, including touching ends and collinear overlap.
fn segments_meet(a: Point, b: Point, c: Point, d: Point) -> bool {
    crosses_properly(a, b, c, d)
        || (cross(a, b, c) == 0 && within(a, b, c))
        || (cross(a, b, d) == 0 && within(a, b, d))
        || (cross(c, d, a) == 0 && within(c, d, a))
        || (cross(c, d, b) == 0 && within(c, d, b))
}

/// The nearest-nanometer crossing of two properly crossing segments, as a marker location.
fn intersection(a: Point, b: Point, c: Point, d: Point) -> Point {
    let (r, s) = (direction(a, b), direction(c, d));
    let t = vcross(direction(a, c), s) as f64 / vcross(r, s) as f64;
    Point { x_nm: (a.x_nm as f64 + t * r.0 as f64).round() as i64, y_nm: (a.y_nm as f64 + t * r.1 as f64).round() as i64 }
}

/// Bounds of a point set, or `None` when it is empty.
fn bounds(points: &[Point]) -> Option<Bounds> {
    let first = points.first()?;
    Some(
        points
            .iter()
            .fold([first.x_nm, first.y_nm, first.x_nm, first.y_nm], |b, p| [b[0].min(p.x_nm), b[1].min(p.y_nm), b[2].max(p.x_nm), b[3].max(p.y_nm)]),
    )
}

/// The union of two optional bounds.
fn join(a: Option<Bounds>, b: Option<Bounds>) -> Option<Bounds> {
    match (a, b) {
        (Some(a), Some(b)) => Some([a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]),
        (a, b) => a.or(b),
    }
}

/// Bounds whose interiors overlap; shared edges or corners do not count.
fn bounds_overlap(a: Bounds, b: Bounds) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

#[cfg(test)]
#[path = "courtyard_tests.rs"]
mod tests;
