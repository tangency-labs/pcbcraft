//! Local Boolean operands for plane keepouts and thermal reliefs.
use super::*;
use crate::board::clearance::index::Tree;

/// Match the planar adapter's reserve for routing close or touching envelopes through
/// the same exact Boolean. The index never decides the shape of a cut.
pub(super) const BOUND_RESERVE_NM: f64 = 50.;

/// Inclusive envelope overlap, expanded by the required cut distance.
pub(super) fn near(a: [f64; 4], b: [f64; 4], grow_nm: f64) -> bool {
    a[0] <= b[2] + grow_nm && b[0] <= a[2] + grow_nm && a[1] <= b[3] + grow_nm && b[1] <= a[3] + grow_nm
}

pub(super) struct Cuts {
    shapes: Vec<Regions>,
    bounds: Vec<[f64; 4]>,
    tree: Tree,
}

impl Cuts {
    /// Preserve stable source order so grouping and Boolean input do not depend on index order.
    pub(super) fn new(shapes: Vec<Regions>) -> Self {
        let (shapes, bounds): (Vec<_>, Vec<_>) = shapes.into_iter().filter_map(|shape| shape.bounds_nm().map(|bounds| (shape, bounds))).unzip();
        let tree = Tree::new(bounds.clone());
        Self { shapes, bounds, tree }
    }

    /// Subtraction only needs cutters that can reach this operand's envelope.
    pub(super) fn subtract(&self, stock: Regions, canceled: Option<&AtomicBool>) -> Result<Regions, String> {
        let Some(bounds) = stock.bounds_nm() else { return Ok(stock) };
        let candidates = self.tree.candidates(bounds, BOUND_RESERVE_NM);
        if candidates.is_empty() {
            return Ok(stock);
        }
        // Small operations already benefit from the kernel's balanced union and need
        // no second spatial pass.
        if candidates.len() < 12 {
            let shapes: Vec<_> = candidates.into_iter().map(|i| self.shapes[i].clone()).collect();
            return difference(&stock, &shapes);
        }

        // Only neighboring cutters need a Boolean union. A group may split into several
        // analytic components, all of which are retained in the resulting local operand.
        let mut visited = vec![false; self.shapes.len()];
        let mut groups = Vec::new();
        for id in candidates.iter().copied() {
            check_cancel(canceled)?;
            if std::mem::replace(&mut visited[id], true) {
                continue;
            }
            let mut queue = vec![id];
            let mut cursor = 0;
            while cursor < queue.len() {
                check_cancel(canceled)?;
                for neighbor in self.tree.candidates(self.bounds[queue[cursor]], BOUND_RESERVE_NM) {
                    if !visited[neighbor] && near(bounds, self.bounds[neighbor], BOUND_RESERVE_NM) {
                        visited[neighbor] = true;
                        queue.push(neighbor);
                    }
                }
                cursor += 1;
            }
            queue.sort_unstable();
            let shapes: Vec<_> = queue.into_iter().map(|i| self.shapes[i].clone()).collect();
            groups.push(union(&shapes)?);
        }
        check_cancel(canceled)?;
        subtract_groups(stock, groups, canceled)
    }
}

/// Apply boundary cuts first so one track crossing the outline does not send all
/// interior footprints through a single large Boolean. Reprove enclosure against the
/// resulting stock before adding holes.
fn subtract_groups(stock: Regions, groups: Vec<Regions>, canceled: Option<&AtomicBool>) -> Result<Regions, String> {
    let mut interior = Vec::new();
    let mut boundary = Vec::new();
    for group in groups {
        check_cancel(canceled)?;
        if let Some(bounds) = group.bounds_nm()
            && stock.covers_box(bounds).map_err(|e| e.to_string())?
        {
            interior.push(group);
        } else {
            boundary.push(group);
        }
    }
    let stock = if boundary.is_empty() { stock } else { difference(&stock, &boundary)? };
    if interior.is_empty() || stock.is_empty() {
        return Ok(stock);
    }
    check_cancel(canceled)?;
    if let Some(result) = stock.subtract_enclosed(&interior).map_err(|e| format!("Could not assemble zone clearance holes: {e}"))? {
        return Ok(result);
    }
    difference(&stock, &interior).map_err(|e| format!("Could not subtract overlapping or boundary clearance cuts: {e}"))
}

/// Keep the original exact Boolean path for small, overlapping, boundary or uncertain cuts.
fn difference(stock: &Regions, cutters: &[Regions]) -> Result<Regions, String> {
    clip(stock, &union(cutters)?, Operation::Difference)
}

fn union(regions: &[Regions]) -> Result<Regions, String> {
    match regions {
        [only] => Ok(only.clone()),
        _ => Regions::union_all(regions).map_err(|e| e.to_string()),
    }
}

/// Evaluate a spoke over its local stock. Its overlap with the body is exactly the
/// spoke's remaining material after subtracting the nearby thermal gaps. At publication
/// subtract each gap minus the accepted spokes, the same set expression as adding
/// those spokes back after clearing all gaps.
pub(super) fn thermal_cuts(
    stock: &Regions,
    foreign: Vec<Regions>,
    gaps: Vec<Regions>,
    candidates: impl IntoIterator<Item = Result<Regions, String>>,
    canceled: Option<&AtomicBool>,
) -> Result<Cuts, String> {
    let foreign = Cuts::new(foreign);
    let gaps = Cuts::new(gaps);
    let mut connected = Vec::new();
    for candidate in candidates {
        check_cancel(canceled)?;
        let spoke = candidate?;
        let clipped = match spoke.bounds_nm() {
            Some(bounds) if stock.covers_box(bounds).map_err(|e| e.to_string())? => spoke,
            _ => clip(&spoke, stock, Operation::Intersection)?,
        };
        let clipped = foreign.subtract(clipped, canceled)?;
        if !gaps.subtract(clipped.clone(), canceled)?.is_empty() {
            connected.push(clipped);
        }
    }
    let connected = Cuts::new(connected);
    let mut shapes = foreign.shapes;
    for gap in gaps.shapes {
        check_cancel(canceled)?;
        shapes.push(connected.subtract(gap, canceled)?);
    }
    Ok(Cuts::new(shapes))
}
