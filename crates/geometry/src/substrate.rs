//! Exact board-outline assembly and the kernel-neutral substrate interface.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Body, ErrorCode, GeometryError, MAX_LENGTH_NM, Result, kernel};

/// A point in the PCB's XY plane, in authored board coordinates and whole nanometres.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BoardPoint(pub [i64; 2]);

/// One Board Outline drawing. Closed drawings omit the repeated end point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlinePath {
    pub points: Vec<BoardPoint>,
    pub closed: bool,
}

/// One through drill, without its copper land or plating, in board nanometres.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RoundDrill {
    pub center: BoardPoint,
    pub diameter_nm: i64,
}

/// A derived triangle mesh, in board coordinates rounded to whole nanometres.
#[derive(Debug)]
pub struct BoardMesh {
    pub positions_nm: Vec<[i64; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

/// Build substrate from the Board Outline centre lines and round through drills. Material starts
/// at Z=0 and ends at `thickness_nm`. Closed contours alternate material and void by nesting,
/// independent of winding; open paths must join at exactly matching endpoints. Drawings that
/// cross, touch or leave an unclosed contour are refused, never replaced with a bounding box.
/// Drills are subtracted as circles, including where they overlap or meet a board edge.
///
/// This does not change the document, write a file, or construct any component models.
/// A build without a kernel returns [`ErrorCode::KernelUnavailable`] for valid inputs.
/// Kernel modelling tolerances can refuse features that are too small or close to resolve.
pub fn build(paths: &[OutlinePath], drills: &[RoundDrill], thickness_nm: i64) -> Result<Body> {
    if !(1..=MAX_LENGTH_NM).contains(&thickness_nm) {
        return Err(invalid("substrate thickness must be positive and within the geometry range"));
    }
    let loops = closed_loops(paths)?;
    let regions = outline_regions(loops)?;
    let mut unique = BTreeSet::new();
    for &drill in drills {
        let [x, y] = drill.center.0;
        if !(1..=MAX_LENGTH_NM).contains(&drill.diameter_nm)
            || !bounded(drill.center)
            || i128::from(x.abs().max(y.abs())) * 2 + i128::from(drill.diameter_nm) > i128::from(MAX_LENGTH_NM) * 2
        {
            return Err(invalid("through drill must be positive and lie within the geometry range"));
        }
        unique.insert(drill);
    }
    let solid = kernel::substrate(&regions, unique, thickness_nm)?;
    Ok(Body { name: "Board substrate".into(), solid })
}

/// Tessellate a substrate body using the same kernel family that constructed it. The tolerance
/// is chord error in nanometres, not a change to the body. A tolerance finer than a millionth of
/// the body's largest extent (100 nm on a 100 mm board) is refused, since the kernel would
/// silently coarsen it. Refused faces return an error instead
/// of a partial mesh; this function never writes a file or consumes the body.
pub fn tessellate(body: &Body, chord_nm: i64) -> Result<BoardMesh> {
    if !(1..=MAX_LENGTH_NM).contains(&chord_nm) {
        return Err(invalid("mesh chord tolerance must be positive and within the geometry range"));
    }
    kernel::substrate_mesh(&body.solid, chord_nm)
}

/// Simple contours with explicitly classified exterior/interior loops for the kernel adapter.
#[derive(Debug)]
#[cfg_attr(all(target_arch = "wasm32", not(feature = "wasm-kernel")), allow(dead_code))]
pub(super) struct Region {
    pub outer: Vec<BoardPoint>,
    pub holes: Vec<Vec<BoardPoint>>,
}

/// Match the authored Board Outline's existing total vertex limit; keep the exact pair checks bounded.
const MAX_OUTLINE_POINTS: usize = 4096;

/// Turn paths into degree-two cycles in board coordinates, without snapping or reversing the
/// authored document. Consuming each path at most once also bounds malformed input.
fn closed_loops(paths: &[OutlinePath]) -> Result<Vec<Vec<BoardPoint>>> {
    if paths.is_empty() {
        return Err(invalid("draw a closed Board Outline before building the substrate"));
    }
    let mut total = 0_usize;
    let mut loops = Vec::new();
    let mut ends = BTreeMap::<_, Vec<usize>>::new();
    for (index, path) in paths.iter().enumerate() {
        let points = &path.points;
        total = total.saturating_add(points.len());
        if total > MAX_OUTLINE_POINTS
            || points.len() < if path.closed { 3 } else { 2 }
            || points.iter().any(|&p| !bounded(p))
            || points.windows(2).any(|pair| pair[0] == pair[1])
        {
            return Err(invalid("outline paths need bounded, distinct vertices within the board drawing limit"));
        }
        let first = points[0];
        let last = points[points.len() - 1];
        if path.closed {
            if first == last {
                return Err(invalid("a closed drawing must not repeat its first vertex"));
            }
            loops.push(points.clone());
        } else if first == last {
            loops.push(points[..points.len() - 1].to_vec());
        } else {
            ends.entry(first).or_default().push(index);
            ends.entry(last).or_default().push(index);
        }
    }
    if ends.values().any(|indices| indices.len() != 2) {
        return Err(invalid("Board Outline endpoints must form closed, unbranched contours"));
    }
    let mut used = vec![false; paths.len()];
    for (index, path) in paths.iter().enumerate() {
        if path.closed || path.points.first() == path.points.last() || used[index] {
            continue;
        }
        used[index] = true;
        let mut points = path.points.clone();
        let first = points[0];
        while let Some(&end) = points.last() {
            if end == first {
                points.pop();
                break;
            }
            let next = ends.get(&end).and_then(|indices| indices.iter().copied().find(|&i| !used[i]));
            let next = next.ok_or_else(|| invalid("Board Outline is not a closed contour"))?;
            used[next] = true;
            let part = &paths[next].points;
            if part.first() == Some(&end) {
                points.extend(part.iter().skip(1));
            } else {
                points.extend(part.iter().rev().skip(1));
            }
        }
        loops.push(points);
    }
    Ok(loops)
}

/// Contours cannot touch or cross (including themselves). Once that is proved, a contour's
/// first vertex determines its nesting. Attach each odd-depth loop to its immediate parent.
fn outline_regions(loops: Vec<Vec<BoardPoint>>) -> Result<Vec<Region>> {
    let mut edges = Vec::new();
    for (owner, points) in loops.iter().enumerate() {
        if points.len() < 3 {
            return Err(invalid("an outline contour needs at least three vertices"));
        }
        for (i, (a, b)) in segments(points).enumerate() {
            let c = points[(i + 2) % points.len()];
            if turn(a, b, c) == 0 && (between(a, b, c) || between(b, c, a)) {
                return Err(invalid("an outline contour doubles back along an edge"));
            }
            edges.push((owner, i, a, b));
        }
    }
    for (index, &(owner, i, a, b)) in edges.iter().enumerate() {
        for &(other, j, c, d) in &edges[index + 1..] {
            if owner == other && (j == i + 1 || (i == 0 && j + 1 == loops[owner].len())) {
                continue;
            }
            if intersects(a, b, c, d) {
                return Err(invalid("Board Outline contours cross or touch"));
            }
        }
    }
    let parents: Vec<Vec<usize>> = loops
        .iter()
        .enumerate()
        .map(|(i, points)| loops.iter().enumerate().filter(|&(j, ring)| i != j && contains(ring, points[0])).map(|(j, _)| j).collect())
        .collect();
    let mut regions = BTreeMap::new();
    for (i, points) in loops.into_iter().enumerate() {
        let nesting = &parents[i];
        if nesting.len().is_multiple_of(2) {
            regions.entry(i).or_insert_with(|| Region { outer: Vec::new(), holes: Vec::new() }).outer = points;
        } else if let Some(parent) = nesting.iter().max_by_key(|&&j| parents[j].len()) {
            regions.entry(*parent).or_insert_with(|| Region { outer: Vec::new(), holes: Vec::new() }).holes.push(points);
        }
    }
    Ok(regions.into_values().collect())
}

/// Traverse all sides, including the closing edge, without duplicating vertices.
fn segments(points: &[BoardPoint]) -> impl Iterator<Item = (BoardPoint, BoardPoint)> + '_ {
    points.iter().copied().zip(points.iter().copied().cycle().skip(1))
}

/// Exact orientation on bounded integer board coordinates, including near-collinear segments.
fn turn(BoardPoint([ax, ay]): BoardPoint, BoardPoint([bx, by]): BoardPoint, BoardPoint([cx, cy]): BoardPoint) -> i128 {
    i128::from(bx - ax) * i128::from(cy - ay) - i128::from(by - ay) * i128::from(cx - ax)
}

/// Whether p lies in the closed box of a segment; callers establish collinearity when required.
fn between(BoardPoint(a): BoardPoint, BoardPoint(b): BoardPoint, BoardPoint(p): BoardPoint) -> bool {
    (0..2).all(|axis| (a[axis].min(b[axis])..=a[axis].max(b[axis])).contains(&p[axis]))
}

/// Count touching and crossing, with a cheap box rejection before exact orientations.
fn intersects(a: BoardPoint, b: BoardPoint, c: BoardPoint, d: BoardPoint) -> bool {
    if (0..2).any(|i| a.0[i].min(b.0[i]) > c.0[i].max(d.0[i]) || c.0[i].min(d.0[i]) > a.0[i].max(b.0[i])) {
        return false;
    }
    let (ab_c, ab_d, cd_a, cd_b) = (turn(a, b, c), turn(a, b, d), turn(c, d, a), turn(c, d, b));
    (ab_c == 0 && between(a, b, c))
        || (ab_d == 0 && between(a, b, d))
        || (cd_a == 0 && between(c, d, a))
        || (cd_b == 0 && between(c, d, b))
        || (ab_c.signum() * ab_d.signum() < 0 && cd_a.signum() * cd_b.signum() < 0)
}

/// Half-open horizontal ray parity; boundary points are excluded by the preceding intersection check.
fn contains(points: &[BoardPoint], p: BoardPoint) -> bool {
    segments(points).fold(false, |inside, (a, b)| {
        let (ay, by, py) = (a.0[1], b.0[1], p.0[1]);
        inside ^ ((ay > py) != (by > py) && ((turn(a, b, p) > 0) == (by > ay)))
    })
}

/// Check each coordinate independently, including both endpoints of the allowed range.
fn bounded(BoardPoint(p): BoardPoint) -> bool {
    p.into_iter().all(|v| (-MAX_LENGTH_NM..=MAX_LENGTH_NM).contains(&v))
}

fn invalid(message: &str) -> GeometryError {
    GeometryError::new(ErrorCode::InvalidParams, message)
}
