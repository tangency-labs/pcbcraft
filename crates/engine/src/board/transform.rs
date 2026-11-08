//! Atomic board placement edits shared by the GUI and MCP.
use crate::{CommandError, Session};
use serde_json::Value;
use std::sync::Arc;

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

    /// Validate revision, identity and transform before replacing a board snapshot. Mapping,
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
        if !footprint.valid() {
            return Err(CommandError::InvalidParams("invalid footprint transform".to_owned()));
        }
        if (current.x_nm, current.y_nm, current.rotation) == (footprint.x_nm, footprint.y_nm, footprint.rotation) {
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
