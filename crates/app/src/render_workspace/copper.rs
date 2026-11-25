//! Derived exterior copper for the 3D board view, in the same frame as the substrate.

use std::collections::{BTreeMap, BTreeSet};

use pcbcraft_board_solid::board1_xy;
use pcbcraft_engine::board::{
    setup::{LayerPurpose, Side},
    view::BoardView,
};
use pcbcraft_geometry::{
    Result,
    planar::{DisplayMesh, Operation, Regions},
};
use solidcore_record::Vec3;

const MM_PER_NM: f64 = 1e-6;
/// Preserve depth separation in the raster pass. Ray tracing can keep copper closer to the laminate.
pub(super) const RASTER_OFFSET_MM: f64 = 0.035;
pub(super) const RAYTRACE_OFFSET_MM: f64 = 0.001;
const CHORD_NM: i64 = 5_000;

/// Add surface-track copper and both annular lands of through vias. Inner-layer tracks have
/// no visible surface. Subtract nearby round drills from tracks so they cannot close a hole.
pub(super) fn visit(board: BoardView<'_>, thickness_nm: i64, offset_mm: f64, mut emit: impl FnMut([Vec3; 3])) -> Result<()> {
    let faces: BTreeMap<_, _> =
        board.layers().into_iter().filter(|layer| layer.purpose == LayerPurpose::Copper).filter_map(|layer| Some((layer.id, layer.side?))).collect();
    let drills = round_drills(board);

    for track in board.tracks() {
        let Some(side) = faces.get(&track.layer).copied() else { continue };
        let mut copper = Regions::rounded_core(&[board1_xy(track.start), board1_xy(track.end)], track.width_nm as f64 * 0.5)?;
        subtract_drills(&mut copper, &drills)?;
        visit_face(&copper.display_mesh(CHORD_NM)?, side, thickness_nm, offset_mm, &mut emit);
    }

    // Reuse the display contour of each via size. Translate every instance before clipping,
    // since overlapping or coincident drills can remove material from a neighboring land.
    let mut meshes = BTreeMap::new();
    for via in board.vias() {
        let at = board1_xy(via.at);
        let dims = (via.diameter_nm, via.drill_nm);
        let radius = via.diameter_nm as f64 * 0.5;
        let envelope = [at[0] as f64 - radius, at[1] as f64 - radius, at[0] as f64 + radius, at[1] as f64 + radius];
        let overlaps =
            drills.iter().any(|(point, diameter)| (*point, *diameter) != (at, via.drill_nm) && overlaps_circle(envelope, *point, *diameter));
        let mesh = if overlaps {
            let mut copper = Regions::circle(at, via.diameter_nm)?;
            subtract_drills(&mut copper, &drills)?;
            copper.display_mesh(CHORD_NM)?
        } else {
            let mesh = match meshes.entry(dims) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => entry.insert(
                    Regions::circle([0, 0], dims.0)?.boolean(&Regions::circle([0, 0], dims.1)?, Operation::Difference)?.display_mesh(CHORD_NM)?,
                ),
            };
            let mut mesh = mesh.clone();
            for p in &mut mesh.vertices_nm {
                p[0] += at[0] as f64;
                p[1] += at[1] as f64;
            }
            mesh
        };
        for side in [Side::Front, Side::Back] {
            visit_face(&mesh, side, thickness_nm, offset_mm, &mut emit);
        }
    }
    Ok(())
}

/// Collect the same supported round openings as the board substrate, suppressing duplicates.
fn round_drills(board: BoardView<'_>) -> BTreeSet<([i64; 2], i64)> {
    let vias = board.vias().iter().map(|via| (board1_xy(via.at), via.drill_nm));
    let footprint_drills = board.footprints().flat_map(|footprint| {
        let pads = footprint.pads.into_iter().filter_map(|pad| Some((board1_xy(pad.center), pad.drill_nm?)));
        let holes = footprint.non_plated_holes.into_iter().map(|hole| (board1_xy(hole.center), hole.diameter_nm));
        pads.chain(holes)
    });
    vias.chain(footprint_drills).collect()
}

/// Reject disjoint candidates without asking the Boolean kernel to clip them.
fn overlaps_circle(bounds: [f64; 4], point: [i64; 2], diameter: i64) -> bool {
    let [x, y] = point.map(|v| v as f64);
    let r = diameter as f64 * 0.5;
    x + r >= bounds[0] && y + r >= bounds[1] && x - r <= bounds[2] && y - r <= bounds[3]
}

/// Every intersecting drill is air on the face, including holes centered at a track endpoint.
fn subtract_drills(copper: &mut Regions, drills: &BTreeSet<([i64; 2], i64)>) -> Result<()> {
    let Some(bounds) = copper.bounds_nm() else { return Ok(()) };
    for &(point, diameter) in drills {
        if overlaps_circle(bounds, point, diameter) {
            *copper = copper.boolean(&Regions::circle(point, diameter)?, Operation::Difference)?;
        }
    }
    Ok(())
}

/// Map an oriented planar copper face onto the board surface and keep the underside winding outward.
fn visit_face(mesh: &DisplayMesh, side: Side, thickness_nm: i64, offset_mm: f64, emit: &mut impl FnMut([Vec3; 3])) {
    let z = match side {
        Side::Front => offset_mm,
        Side::Back => -thickness_nm as f64 * MM_PER_NM - offset_mm,
    };
    for &[a, b, c] in &mesh.triangles {
        let indices = if side == Side::Front { [a, b, c] } else { [a, c, b] };
        emit(indices.map(|i| {
            let [x, y] = mesh.vertices_nm[i as usize];
            Vec3::new(x * MM_PER_NM, y * MM_PER_NM, z)
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A copper track ending at a through via must leave the drilled air open on the face.
    #[test]
    fn a_track_ending_at_a_via_leaves_the_drill_open() {
        let mut copper = Regions::rounded_core(&[[-1_000_000, 0], [0, 0]], 200_000.0).unwrap();
        subtract_drills(&mut copper, &BTreeSet::from([([0, 0], 300_000)])).unwrap();
        assert_eq!(copper.contains([0, 0]).unwrap(), Some(false));
        assert_eq!(copper.contains([-500_000, 0]).unwrap(), Some(true));
    }
}
