//! Compose the PCB document's read-only board view with the solid geometry provider.
//! No editor or saved representation is involved. [`assembly`] writes the board with
//! caller-supplied component models as a STEP assembly; it reads no model record.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::BTreeSet;

pub mod assembly;

use pcbcraft_engine::board::{
    setup::LayerPurpose,
    view::{BoardTarget, BoardView},
};
use pcbcraft_engine::schematic::Point;
use pcbcraft_geometry::{
    Body, Result, Transform,
    substrate::{self, BoardPoint, OutlinePath, RoundDrill},
};

/// The nominal fallback used when the stackup has no finished thickness declaration.
/// This is PCBcraft's two-layer setup convention, not a fabrication guarantee.
/// See `docs/pcb/board-materials.md`.
pub const NOMINAL_THICKNESS_NM: i64 = 1_600_000;

/// The source of the Z dimension, which downstream consumers should report with the solid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Thickness {
    /// The stackup's declared nominal finished thickness, in nanometres.
    SpecifiedFinishedNm(i64),
    /// No finished thickness declared; use [`NOMINAL_THICKNESS_NM`] and label it nominal.
    NominalFallback,
}

impl Thickness {
    /// Return the Z dimension of the substrate in nanometres.
    pub const fn nm(self) -> i64 {
        match self {
            Self::SpecifiedFinishedNm(nm) => nm,
            Self::NominalFallback => NOMINAL_THICKNESS_NM,
        }
    }
}

/// A disposable board-only solid; never store it in the project or history.
#[derive(Debug)]
pub struct BoardSolid {
    pub target: BoardTarget,
    pub thickness: Thickness,
    pub body: Body,
}

impl BoardSolid {
    /// Make the substrate of the explicitly addressed immutable board. Use the resolved Board
    /// Outline layer (including legacy rectangles), through via drills, resolved circular pad
    /// drills and the finished openings of resolved footprint non-plated holes. Plated slot
    /// openings are not yet subtracted from the substrate solid.
    /// Use the stackup's finished thickness declaration, not the minimum or sum of stock plies.
    /// The solid is in the shared board/1 frame: the origin at board zero, +X along board X, +Y
    /// along board −Y, and Z = 0 at the finished top surface, so the body spans −thickness..0.
    /// An invalid outline or unavailable kernel is returned to the caller without changing board
    /// state or undo.
    pub fn build(board: BoardView<'_>) -> Result<Self> {
        let (thickness, outline, drills) = board_profile(board);
        // The substrate is built from Z = 0 up; lower it so the finished top lies at Z = 0.
        let body = substrate::build(&outline, &drills, thickness.nm())?.transformed(&Transform::translation([0, 0, -thickness.nm()]))?;
        Ok(Self { target: board.target, thickness, body })
    }
}

/// Build a disposable 3D display mesh and report the same thickness source as the solid.
pub fn display_board(board: BoardView<'_>, chord_nm: i64) -> Result<(substrate::BoardMesh, Thickness)> {
    let (thickness, outline, drills) = board_profile(board);
    Ok((substrate::display_board(&outline, &drills, thickness.nm(), chord_nm)?, thickness))
}

/// Collect precisely the same board/1 outlines and round openings for both geometry paths.
fn board_profile(board: BoardView<'_>) -> (Thickness, Vec<OutlinePath>, Vec<RoundDrill>) {
    let thickness =
        board.setup().and_then(|setup| setup.stackup().finished_thickness_nm).map_or(Thickness::NominalFallback, Thickness::SpecifiedFinishedNm);
    let outline_layers: BTreeSet<_> =
        board.layers().into_iter().filter(|layer| layer.purpose == LayerPurpose::Outline).map(|layer| layer.id).collect();
    let outline: Vec<_> = board
        .drawings()
        .into_iter()
        .filter(|drawing| outline_layers.contains(&drawing.layer))
        .map(|drawing| OutlinePath { points: drawing.points.into_iter().map(board1_point).collect(), closed: drawing.closed })
        .collect();
    let vias = board.vias().iter().map(|via| RoundDrill { center: board1_point(via.at), diameter_nm: via.drill_nm });
    let footprints = board.footprints().flat_map(|footprint| {
        // Circular pad drills only; plated slot openings are not yet subtracted.
        let pads = footprint.pads.into_iter().filter_map(|pad| Some(RoundDrill { center: board1_point(pad.center), diameter_nm: pad.drill_nm? }));
        let holes =
            footprint.non_plated_holes.into_iter().map(|hole| RoundDrill { center: board1_point(hole.center), diameter_nm: hole.diameter_nm });
        pads.chain(holes)
    });
    let drills: Vec<_> = vias.chain(footprints).collect();
    (thickness, outline, drills)
}

/// Convert an editor point to board/1 XY in nanometres. Exchange and display share the
/// same board origin, with X unchanged and Y negated.
pub fn board1_xy(point: Point) -> [i64; 2] {
    [point.x_nm, point.y_nm.saturating_neg()]
}

/// Wrap board/1 XY in the substrate coordinate type without changing its units.
fn board1_point(point: Point) -> BoardPoint {
    BoardPoint(board1_xy(point))
}

#[cfg(test)]
mod tests;
