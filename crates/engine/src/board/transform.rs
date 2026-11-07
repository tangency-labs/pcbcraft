//! Atomic board placement edits shared by the GUI and MCP.
use crate::{CommandError, Session};
use serde_json::Value;
use std::sync::Arc;

impl Session {
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
