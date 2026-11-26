//! A bounded display triangulation for separated, interior through holes.
use std::collections::BTreeSet;

use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

use super::{BoardMesh, BoardPoint, Region, RoundDrill, contains, segments};

const MAX_PROFILE_VERTICES: usize = 32_768;
const MIN_CIRCLE_VERTICES: usize = 12;

/// Classify strictly interior, disjoint drill circles without a planar Boolean. Contacts,
/// uncertain clearances, exterior drills and larger batches use the general Boolean.
fn separate_drill_owners(regions: &[Region], drills: &BTreeSet<RoundDrill>) -> Option<Vec<usize>> {
    const CLEARANCE_NM: f64 = 1_000.0;
    if drills.len() > MAX_PROFILE_VERTICES / MIN_CIRCLE_VERTICES {
        return None;
    }
    let max_radius = drills.iter().map(|drill| drill.diameter_nm).max().unwrap_or_default() as f64 * 0.5;
    let mut owners = Vec::with_capacity(drills.len());
    let mut fitted = Vec::<&RoundDrill>::with_capacity(drills.len());
    for drill in drills {
        let [x, y] = drill.center.0.map(|v| v as f64);
        let radius = drill.diameter_nm as f64 * 0.5;
        // RoundDrill sorts by its center's X coordinate first. Earlier circles outside
        // this window cannot touch the current one.
        if fitted.iter().rev().take_while(|other| x - other.center.0[0] as f64 <= radius + max_radius + CLEARANCE_NM).any(|other| {
            let [ox, oy] = other.center.0.map(|v| v as f64);
            (x - ox).hypot(y - oy) <= radius + other.diameter_nm as f64 * 0.5 + CLEARANCE_NM
        }) {
            return None;
        }
        let owner = regions
            .iter()
            .position(|region| contains(&region.outer, drill.center) && !region.holes.iter().any(|hole| contains(hole, drill.center)))?;
        let region = &regions[owner];
        for contour in std::iter::once(&region.outer).chain(region.holes.iter()) {
            for (a, b) in segments(contour) {
                let [ax, ay] = a.0.map(|v| v as f64);
                let [bx, by] = b.0.map(|v| v as f64);
                let (dx, dy) = (bx - ax, by - ay);
                let along = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
                if (x - ax - along * dx).hypot(y - ay - along * dy) <= radius + CLEARANCE_NM {
                    return None;
                }
            }
        }
        fitted.push(drill);
        owners.push(owner);
    }
    Some(owners)
}

/// Triangulate a bounded set of proven non-crossing profile loops. Refuse any topology
/// mismatch so callers can fall back to exact solid tessellation.
pub(super) fn mesh(regions: &[Region], drills: &BTreeSet<RoundDrill>, thickness: i64, chord: i64) -> Option<BoardMesh> {
    let owners = separate_drill_owners(regions, drills)?;
    let mut holes: Vec<_> = regions.iter().map(|region| region.holes.clone()).collect();
    let mut count: usize = regions.iter().map(|region| region.outer.len() + region.holes.iter().map(Vec::len).sum::<usize>()).sum();
    for (owner, drill) in owners.into_iter().zip(drills) {
        let ring = circle(*drill, chord)?;
        count += ring.len();
        if count > MAX_PROFILE_VERTICES {
            return None;
        }
        holes[owner].push(ring);
    }
    let mut mesh = BoardMesh { positions_nm: Vec::new(), triangles: Vec::new() };
    for (region, holes) in regions.iter().zip(holes) {
        add_region(&mut mesh, &region.outer, &holes, thickness)?;
    }
    (!mesh.triangles.is_empty()).then_some(mesh)
}

/// Approximate the round opening to the display chord tolerance, reserving 1 nm for rounding.
fn circle(drill: RoundDrill, chord: i64) -> Option<Vec<BoardPoint>> {
    let radius = drill.diameter_nm as f64 * 0.5;
    let permitted = (chord - 1) as f64;
    let angle = (1.0 - permitted / radius).clamp(-1.0, 1.0).acos();
    let count = (std::f64::consts::PI / angle).ceil().max(MIN_CIRCLE_VERTICES as f64) as usize;
    if count > 256 {
        return None;
    }
    Some(
        (0..count)
            .map(|i| {
                let (sin, cos) = (std::f64::consts::TAU * i as f64 / count as f64).sin_cos();
                BoardPoint([drill.center.0[0] + (radius * cos).round() as i64, drill.center.0[1] + (radius * sin).round() as i64])
            })
            .collect(),
    )
}

/// Keep only material triangles, then extrude exactly the recovered cap boundary as side walls.
fn add_region(mesh: &mut BoardMesh, outer: &[BoardPoint], holes: &[Vec<BoardPoint>], thickness: i64) -> Option<()> {
    let mut points = Vec::new();
    let mut constraints = Vec::new();
    for contour in std::iter::once(outer).chain(holes.iter().map(Vec::as_slice)) {
        let start = points.len();
        points.extend(contour.iter().map(|p| Point2::new(p.0[0] as f64, p.0[1] as f64)));
        constraints.extend((0..contour.len()).map(|i| [start + i, start + (i + 1) % contour.len()]));
    }
    let mut conflict = false;
    let cdt = ConstrainedDelaunayTriangulation::<Point2<f64>>::try_bulk_load_cdt(points.clone(), constraints.clone(), |_| conflict = true).ok()?;
    if conflict || cdt.num_vertices() != points.len() {
        return None;
    }
    let hole_bounds: Vec<_> = holes
        .iter()
        .map(|hole| {
            hole.iter().fold([i64::MAX, i64::MAX, i64::MIN, i64::MIN], |[min_x, min_y, max_x, max_y], point| {
                let [x, y] = point.0;
                [min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y)]
            })
        })
        .collect();
    let mut caps = Vec::new();
    let mut sides = BTreeSet::new();
    for face in cdt.inner_faces() {
        let ix = face.vertices().map(|v| v.index());
        let [a, b, c] = face.positions();
        let sample = Point2::new((a.x + b.x + c.x) / 3.0, (a.y + b.y + c.y) / 3.0);
        if in_loop(outer, sample)
            && !holes.iter().zip(&hole_bounds).any(|(hole, &[min_x, min_y, max_x, max_y])| {
                sample.x >= min_x as f64 && sample.x <= max_x as f64 && sample.y >= min_y as f64 && sample.y <= max_y as f64 && in_loop(hole, sample)
            })
        {
            caps.push(ix);
            for (u, v) in [(ix[0], ix[1]), (ix[1], ix[2]), (ix[2], ix[0])] {
                if !sides.remove(&(v, u)) {
                    sides.insert((u, v));
                }
            }
        }
    }
    let undirected = |a: usize, b: usize| (a.min(b), a.max(b));
    if sides.iter().map(|&(a, b)| undirected(a, b)).collect::<BTreeSet<_>>() != constraints.into_iter().map(|[a, b]| undirected(a, b)).collect() {
        return None;
    }
    let top = mesh.positions_nm.len() as u32;
    let bottom = top + points.len() as u32;
    for z in [0, -thickness] {
        mesh.positions_nm.extend(points.iter().map(|p| [p.x as i64, p.y as i64, z]));
    }
    for [a, b, c] in caps {
        let [a, b, c] = [a as u32, b as u32, c as u32];
        mesh.triangles.extend([[top + a, top + b, top + c], [bottom + a, bottom + c, bottom + b]]);
    }
    for (a, b) in sides {
        let (a, b) = (a as u32, b as u32);
        mesh.triangles.extend([[top + a, bottom + a, top + b], [top + b, bottom + a, bottom + b]]);
    }
    Some(())
}

/// Classify a triangle's interior sample against a constraint loop using half-open crossings.
fn in_loop(contour: &[BoardPoint], p: Point2<f64>) -> bool {
    segments(contour).fold(false, |inside, (a, b)| {
        let [ax, ay] = a.0.map(|v| v as f64);
        let [bx, by] = b.0.map(|v| v as f64);
        inside ^ ((ay > p.y) != (by > p.y) && p.x < ax + (bx - ax) * (p.y - ay) / (by - ay))
    })
}
