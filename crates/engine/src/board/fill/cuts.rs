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
    pub(super) shapes: Vec<Regions>,
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

    pub(super) fn candidates(&self, bounds: [f64; 4]) -> Vec<usize> {
        self.tree.candidates(bounds, BOUND_RESERVE_NM)
    }

    /// Subtraction only needs cutters that can reach this operand's envelope.
    pub(super) fn subtract(&self, stock: Regions, canceled: Option<&AtomicBool>) -> Result<Regions, String> {
        let Some(bounds) = stock.bounds_nm() else { return Ok(stock) };
        let candidates = self.candidates(bounds);
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
        let mut groups = Vec::new();
        let mut enclosed = Vec::new();
        let mut boundary = Vec::new();
        for group in components(&self.tree, &self.bounds, &candidates, canceled)? {
            check_cancel(canceled)?;
            let mut inside = true;
            for &index in &group {
                check_cancel(canceled)?;
                if !stock.covers_box(self.bounds[index]).map_err(|e| e.to_string())? {
                    inside = false;
                    break;
                }
            }
            let ids = if inside { &mut enclosed } else { &mut boundary };
            ids.push(groups.len());
            let shapes: Vec<_> = group.into_iter().map(|i| self.shapes[i].clone()).collect();
            groups.push(union(&shapes)?);
        }
        check_cancel(canceled)?;
        if !enclosed.is_empty() {
            // Removing a boundary group cannot touch any enclosed source envelope.
            // Preserve that separation even if the resulting aggregate bounds overlap.
            let result = if boundary.is_empty() {
                stock.subtract_independent_enclosed(&groups)
            } else {
                let outside: Vec<_> = boundary.into_iter().map(|id| groups[id].clone()).collect();
                let clipped = difference(&stock, &outside);
                check_cancel(canceled)?;
                match clipped {
                    Ok(clipped) => {
                        let inside: Vec<_> = enclosed.into_iter().map(|id| groups[id].clone()).collect();
                        clipped.subtract_independent_enclosed(&inside)
                    }
                    Err(_) => Ok(None),
                }
            };
            check_cancel(canceled)?;
            if let Some(result) = result.map_err(|e| e.to_string())? {
                return Ok(result);
            }
        }
        subtract_groups(stock, groups, canceled)
    }
}

/// Envelopes in separate components cannot influence each other's Boolean operands.
/// Keep source order within each component for stable kernel input.
pub(super) fn components(tree: &Tree, bounds: &[[f64; 4]], candidates: &[usize], canceled: Option<&AtomicBool>) -> Result<Vec<Vec<usize>>, String> {
    let mut visited = vec![true; bounds.len()];
    candidates.iter().for_each(|&i| visited[i] = false);
    let mut groups = Vec::new();
    for id in candidates.iter().copied() {
        if std::mem::replace(&mut visited[id], true) {
            continue;
        }
        let mut group = vec![id];
        let mut next = 0;
        while next < group.len() {
            check_cancel(canceled)?;
            for neighbor in tree.candidates(bounds[group[next]], BOUND_RESERVE_NM) {
                if !std::mem::replace(&mut visited[neighbor], true) {
                    group.push(neighbor);
                }
            }
            next += 1;
        }
        group.sort_unstable();
        groups.push(group);
    }
    Ok(groups)
}

/// Apply complex cuts first, so a boundary crossing or one intertwined thermal and
/// track does not send all independent footprints through the same large Boolean.
/// Prove enclosure again against the resulting stock before assembling its holes.
fn subtract_groups(mut stock: Regions, groups: Vec<Regions>, canceled: Option<&AtomicBool>) -> Result<Regions, String> {
    // The covers-box proof applies to mutually separated cutter envelopes. Envelopes
    // may meet even when the individual inputs of two prior unions were disjoint.
    let cuts = Cuts::new(groups);
    let candidates: Vec<_> = (0..cuts.shapes.len()).collect();
    let mut enclosed = Vec::new();
    for group in components(&cuts.tree, &cuts.bounds, &candidates, canceled)? {
        check_cancel(canceled)?;
        let shapes: Vec<_> = group.into_iter().map(|i| cuts.shapes[i].clone()).collect();
        let shape = union(&shapes)?;
        if let Some(bounds) = shape.bounds_nm() {
            enclosed.push((shape, bounds));
        }
    }
    loop {
        let mut remaining = Vec::new();
        let mut exact = Vec::new();
        for (group, bounds) in enclosed {
            check_cancel(canceled)?;
            if stock.covers_box(bounds).map_err(|e| e.to_string())? {
                remaining.push((group, bounds));
            } else {
                exact.push(group);
            }
        }
        if exact.is_empty() {
            enclosed = remaining;
            break;
        }
        stock = difference(&stock, &exact)?;
        if remaining.is_empty() || stock.is_empty() {
            return Ok(stock);
        }
        enclosed = remaining;
    }
    let interior: Vec<_> = enclosed.into_iter().map(|(group, _)| group).collect();
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
pub(super) fn thermal_gaps(
    stock: Option<&Regions>,
    foreign: &Cuts,
    gaps: Vec<Regions>,
    candidates: impl IntoIterator<Item = Result<Regions, String>>,
    canceled: Option<&AtomicBool>,
) -> Result<Vec<Regions>, String> {
    let gaps = Cuts::new(gaps);
    let mut connected = Vec::new();
    for candidate in candidates {
        check_cancel(canceled)?;
        let spoke = candidate?;
        let clipped = match (stock, spoke.bounds_nm()) {
            (Some(stock), Some(bounds)) if !stock.covers_box(bounds).map_err(|e| e.to_string())? => clip(&spoke, stock, Operation::Intersection)?,
            _ => spoke,
        };
        let clipped = foreign.subtract(clipped, canceled)?;
        if !gaps.subtract(clipped.clone(), canceled)?.is_empty() {
            connected.push(clipped);
        }
    }
    let connected = Cuts::new(connected);
    let mut shapes = Vec::with_capacity(gaps.shapes.len());
    for gap in gaps.shapes {
        check_cancel(canceled)?;
        shapes.push(connected.subtract(gap, canceled)?);
    }
    Ok(shapes)
}
