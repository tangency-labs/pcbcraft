//! Compose the PCB document's read-only board view with the solid geometry provider.
//! No editor, saved representation, component model or interchange format is involved.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::BTreeSet;

use pcbcraft_engine::board::{
    setup::LayerPurpose,
    view::{BoardTarget, BoardView},
};
use pcbcraft_geometry::{
    Body, Result,
    substrate::{self, BoardPoint, OutlinePath, RoundDrill},
};

/// The nominal fallback used when the stackup has no finished thickness declaration.
/// This is PCBcraft's two-layer setup convention, not a fabrication guarantee.
/// See `docs/board-materials.md`.
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
    /// Outline layer (including legacy rectangles), through via drills, resolved pad drills and
    /// the finished openings of resolved footprint non-plated holes.
    /// Use the stackup's finished thickness declaration, not the minimum or sum of stock plies.
    /// XY stays in board coordinates; Z runs from zero to the chosen thickness. An invalid outline
    /// or unavailable kernel is returned to the caller without changing board state or undo.
    pub fn build(board: BoardView<'_>) -> Result<Self> {
        let thickness =
            board.setup().and_then(|setup| setup.stackup().finished_thickness_nm).map_or(Thickness::NominalFallback, Thickness::SpecifiedFinishedNm);
        let outline_layers: BTreeSet<_> =
            board.layers().into_iter().filter(|layer| layer.purpose == LayerPurpose::Outline).map(|layer| layer.id).collect();
        let outline: Vec<_> = board
            .drawings()
            .into_iter()
            .filter(|drawing| outline_layers.contains(&drawing.layer))
            .map(|drawing| OutlinePath { points: drawing.points.into_iter().map(|p| BoardPoint([p.x_nm, p.y_nm])).collect(), closed: drawing.closed })
            .collect();
        let vias = board.vias().iter().map(|via| RoundDrill { center: BoardPoint([via.at.x_nm, via.at.y_nm]), diameter_nm: via.drill_nm });
        let footprints = board.footprints().flat_map(|footprint| {
            let pads = footprint
                .pads
                .into_iter()
                .filter_map(|pad| Some(RoundDrill { center: BoardPoint([pad.center.x_nm, pad.center.y_nm]), diameter_nm: pad.drill_nm? }));
            let holes = footprint
                .non_plated_holes
                .into_iter()
                .map(|hole| RoundDrill { center: BoardPoint([hole.center.x_nm, hole.center.y_nm]), diameter_nm: hole.diameter_nm });
            pads.chain(holes)
        });
        let drills: Vec<_> = vias.chain(footprints).collect();
        let body = substrate::build(&outline, &drills, thickness.nm())?;
        Ok(Self { target: board.target, thickness, body })
    }
}

#[cfg(test)]
mod tests;
