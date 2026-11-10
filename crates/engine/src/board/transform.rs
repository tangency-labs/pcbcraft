//! Atomic board placement edits shared by the GUI and MCP.
use crate::{CommandError, Session, schematic::Point};
use serde_json::Value;
use std::sync::Arc;

/// Align or evenly distribute the addressed footprints by their bounds, in one undo step.
pub const ALIGN_FOOTPRINTS: &str = "pcb.align_footprints";
/// The alignment modes, as MCP clients name them.
pub const ALIGN_MODES: [&str; 8] = {
    let mut names = [""; 8];
    let mut i = 0;
    while i < names.len() {
        names[i] = AlignMode::ALL[i].name();
        i += 1;
    }
    names
};

/// How `pcb.align_footprints` moves the selection: onto a shared edge or center line, or
/// spread with equal gaps along an axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignMode {
    Left,
    Right,
    Top,
    Bottom,
    CenterX,
    CenterY,
    DistributeX,
    DistributeY,
}

impl AlignMode {
    /// Every mode, in the order the Modify menu lists them.
    pub const ALL: [Self; 8] = [Self::Left, Self::Right, Self::Top, Self::Bottom, Self::CenterX, Self::CenterY, Self::DistributeX, Self::DistributeY];

    /// The mode's MCP name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::CenterX => "center_x",
            Self::CenterY => "center_y",
            Self::DistributeX => "distribute_x",
            Self::DistributeY => "distribute_y",
        }
    }

    /// The mode an MCP name refers to, if any.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.name() == name)
    }

    /// Whether the mode spreads footprints rather than lining them up.
    pub const fn distributes(self) -> bool {
        matches!(self, Self::DistributeX | Self::DistributeY)
    }

    /// The fewest footprints the mode applies to: three to distribute, two to align.
    pub const fn minimum(self) -> usize {
        if self.distributes() { 3 } else { 2 }
    }
}

/// A footprint's physical extent: pad centers and lands plus courtyard and fabrication graphics.
/// Silkscreen, including the reference text, is left out so labels never shift an alignment.
/// Falls back to its origin when it has no geometry.
fn bounds(footprint: &super::view::FootprintView<'_>) -> [i64; 4] {
    let points = footprint.pads.iter().flat_map(|pad| std::iter::once(pad.center).chain(pad.copper.into_iter().flatten())).chain(
        footprint
            .graphics
            .iter()
            .filter(|graphic| graphic.purpose != super::definition::GraphicPurpose::Silkscreen)
            .flat_map(|graphic| graphic.points.iter().copied()),
    );
    let start = [footprint.position.x_nm, footprint.position.y_nm, footprint.position.x_nm, footprint.position.y_nm];
    let mut first = true;
    points.fold(start, |b, p| {
        let b = if std::mem::take(&mut first) { [p.x_nm, p.y_nm, p.x_nm, p.y_nm] } else { b };
        [b[0].min(p.x_nm), b[1].min(p.y_nm), b[2].max(p.x_nm), b[3].max(p.y_nm)]
    })
}

/// Displacements that align or distribute `boxes` (min x, min y, max x, max y). Top is the
/// smallest board Y. Centers align on the middle of the whole selection's bounds, rounding
/// half-nanometer centers downward consistently across zero so repeating alignment is a no-op.
/// Distribution requires at least three boxes, keeps the physical outermost items fixed and
/// makes the gaps between neighbors' bounds equal to integer precision. An overlapping selection
/// that cannot retain its outer anchors and original span in this order is refused.
fn alignment(mode: AlignMode, boxes: &[[i64; 4]]) -> Result<Vec<(i64, i64)>, CommandError> {
    let min = |i: usize| boxes.iter().map(|b| b[i]).min().unwrap_or(0);
    let max = |i: usize| boxes.iter().map(|b| b[i]).max().unwrap_or(0);
    Ok(match mode {
        AlignMode::Left => boxes.iter().map(|b| (min(0) - b[0], 0)).collect(),
        AlignMode::Right => boxes.iter().map(|b| (max(2) - b[2], 0)).collect(),
        AlignMode::Top => boxes.iter().map(|b| (0, min(1) - b[1])).collect(),
        AlignMode::Bottom => boxes.iter().map(|b| (0, max(3) - b[3])).collect(),
        AlignMode::CenterX => boxes.iter().map(|b| ((min(0) + max(2)).div_euclid(2) - (b[0] + b[2]).div_euclid(2), 0)).collect(),
        AlignMode::CenterY => boxes.iter().map(|b| (0, (min(1) + max(3)).div_euclid(2) - (b[1] + b[3]).div_euclid(2))).collect(),
        AlignMode::DistributeX | AlignMode::DistributeY => {
            if boxes.len() < 3 {
                return Err(CommandError::InvalidParams("distribution needs at least three footprints".into()));
            }
            let axis = if mode == AlignMode::DistributeX { 0 } else { 1 };
            let mut order: Vec<usize> = (0..boxes.len()).collect();
            order.sort_by_key(|&i| (boxes[i][axis] + boxes[i][axis + 2], i));
            // Choose anchors by physical edges, even when overlapping boxes have a different
            // center order. Ties stay deterministic in the caller's footprint order.
            let leading = (0..boxes.len()).min_by_key(|&i| (boxes[i][axis], i)).unwrap_or(0);
            let trailing = (0..boxes.len()).max_by_key(|&i| (boxes[i][axis + 2], i)).unwrap_or(0);
            if leading != trailing {
                order.retain(|&i| i != leading && i != trailing);
                order.insert(0, leading);
                order.push(trailing);
            }
            let (start_edge, end_edge) = (min(axis), max(axis + 2));
            let span = end_edge - start_edge;
            let sizes: i64 = boxes.iter().map(|b| b[axis + 2] - b[axis]).sum();
            let (gaps, free) = ((order.len() - 1) as i64, span - sizes);
            let mut deltas = vec![(0, 0); boxes.len()];
            let mut before = 0;
            for (rank, &i) in order.iter().enumerate() {
                // Spread integer remainders across gaps; the final trailing edge stays exact.
                let start = start_edge + before + free * rank as i64 / gaps;
                let size = boxes[i][axis + 2] - boxes[i][axis];
                let delta = start - boxes[i][axis];
                let anchor = boxes[i][axis] == start_edge || boxes[i][axis + 2] == end_edge;
                if (anchor && delta != 0) || start < start_edge || start + size > end_edge {
                    return Err(CommandError::InvalidParams(
                        "overlapping footprints cannot be distributed while keeping the outer footprints and span fixed".into(),
                    ));
                }
                deltas[i] = if axis == 0 { (delta, 0) } else { (0, delta) };
                before += size;
            }
            deltas
        }
    })
}

impl Session {
    /// Validate every selected placement and the source revision before deleting any. Physical
    /// placements retain their pinned package assignment for re-placement; schematic geometry,
    /// components, drawings and setup remain unchanged. Undo restores exact placement identities.
    pub(crate) fn delete_footprints(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let ids: std::collections::BTreeSet<_> = params["footprints"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|id| id.as_str().and_then(|id| crate::schematic::wiring::parse_id(id, "footprint-")))
            .collect();
        let project = self.open_project()?;
        if ids.is_empty() || ids.iter().any(|id| project.board.footprints.binary_search_by_key(id, |f| f.id).is_err()) {
            return Err(CommandError::InvalidParams("unknown footprint ID".into()));
        }
        let mut next = project.clone();
        let board = Arc::make_mut(&mut next.board);
        for footprint in board.footprints.iter().filter(|f| ids.contains(&f.id)) {
            if let (Some(library), Some(mapping)) = (&footprint.library, &footprint.mapping) {
                board.assignments.entry(footprint.component).or_insert_with(|| super::assignment::Assignment {
                    component: footprint.component,
                    library: library.clone(),
                    definition: footprint.definition.clone(),
                    definition_revision: footprint.definition_revision,
                    mapping: mapping.clone(),
                });
            }
        }
        board.footprints.retain(|f| !ids.contains(&f.id));
        board.by_component = board.footprints.iter().enumerate().map(|(i, f)| (f.component, i)).collect();
        self.change(next);
        Ok(())
    }

    /// Align (two or more) or distribute (three or more) the addressed footprints by their
    /// resolved bounds, moving only their origins. Unknown or repeated IDs, an unknown mode, too
    /// few footprints, or a result outside the coordinate range leave the board unchanged; an
    /// alignment that moves nothing is not an edit. Overlapping distribution is refused when its
    /// physical outer anchors or original span would change.
    pub(crate) fn align_footprints(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let mode = params["mode"].as_str().and_then(AlignMode::parse).ok_or_else(|| CommandError::InvalidParams("unknown alignment mode".into()))?;
        let ids: Vec<u64> = params["footprints"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|id| id.as_str().and_then(|id| crate::schematic::wiring::parse_id(id, "footprint-")))
            .collect::<Option<_>>()
            .ok_or_else(|| CommandError::InvalidParams("footprints must be footprint IDs".into()))?;
        let needed = mode.minimum();
        if ids.len() < needed || ids.iter().collect::<std::collections::BTreeSet<_>>().len() != ids.len() {
            return Err(CommandError::InvalidParams(format!("{} needs at least {needed} different footprints", mode.name())));
        }
        let target = project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let view = project.board_view(target).ok_or_else(|| CommandError::Unavailable("board unavailable".into()))?;
        let wanted: std::collections::BTreeSet<String> = ids.iter().map(|id| format!("footprint-{id}")).collect();
        let mut found: std::collections::BTreeMap<String, [i64; 4]> =
            view.footprints().filter(|f| wanted.contains(&f.id)).map(|f| (f.id.clone(), bounds(&f))).collect();
        let boxes = ids
            .iter()
            .map(|id| found.remove(&format!("footprint-{id}")))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| CommandError::InvalidParams("unknown footprint ID".into()))?;
        let deltas = alignment(mode, &boxes)?;
        if deltas.iter().all(|delta| *delta == (0, 0)) {
            return Ok(());
        }
        let mut next = project.clone();
        let board = Arc::make_mut(&mut next.board);
        for (id, (dx, dy)) in ids.iter().zip(deltas) {
            let index =
                board.footprints.binary_search_by_key(id, |f| f.id).map_err(|_| CommandError::InvalidParams("unknown footprint ID".into()))?;
            let footprint = &mut board.footprints[index];
            let moved = Point { x_nm: footprint.x_nm.saturating_add(dx), y_nm: footprint.y_nm.saturating_add(dy) };
            (footprint.x_nm, footprint.y_nm) = (moved.x_nm, moved.y_nm);
            if !footprint.valid() {
                return Err(CommandError::InvalidParams("the alignment would move a footprint outside the coordinate range".into()));
            }
        }
        self.change(next);
        Ok(())
    }

    /// Validate revision, identity and transform before replacing a board snapshot. An optional
    /// `side` (front or back) places the footprint on that side; omitted keeps its side. Mapping,
    /// component identity and schematic geometry are retained; unchanged input creates no history.
    pub(crate) fn set_footprint_transform(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let id = params["footprint"].as_str().and_then(|id| crate::schematic::wiring::parse_id(id, "footprint-"));
        let index = project
            .board
            .footprints
            .binary_search_by_key(&id.unwrap_or_default(), |f| f.id)
            .map_err(|_| CommandError::InvalidParams("unknown footprint ID".to_owned()))?;
        let current = &project.board.footprints[index];
        let mut footprint = current.clone();
        footprint.x_nm = params["x_nm"].as_i64().unwrap_or_default();
        footprint.y_nm = params["y_nm"].as_i64().unwrap_or_default();
        footprint.rotation = params["rotation"].as_u64().unwrap_or_default() as u16;
        match params.get("side").and_then(Value::as_str) {
            None => {}
            Some("front") => footprint.bottom = false,
            Some("back") => footprint.bottom = true,
            Some(_) => return Err(CommandError::InvalidParams("side must be front or back".to_owned())),
        }
        if !footprint.valid() {
            return Err(CommandError::InvalidParams("invalid footprint transform".to_owned()));
        }
        if (current.x_nm, current.y_nm, current.rotation, current.bottom) == (footprint.x_nm, footprint.y_nm, footprint.rotation, footprint.bottom) {
            return Ok(());
        }
        let mut project = project.clone();
        Arc::make_mut(&mut project.board).footprints[index] = footprint;
        self.change(project);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
