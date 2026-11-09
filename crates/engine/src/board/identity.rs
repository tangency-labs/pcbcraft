//! Explicit optional ownership for the first physical board, with legacy byte preservation.

use crate::{CommandError, Project, Session};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

/// Create the sole physical board explicitly without changing schematic identities.
pub const CREATE_BOARD: &str = "pcb.create_board";

/// Versioned single-board registration; paths remain fixed until a later layout migration.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Registration {
    pub id: Uuid,
    pub name: String,
    pub file: String,
}

/// Legacy files retain implicit ownership; new files explicitly register zero or one board.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Ownership {
    #[default]
    Legacy,
    Explicit(Option<Registration>),
}

impl Ownership {
    /// Encode explicit absence or presence without upgrading untouched legacy documents.
    pub(crate) fn records(&self) -> Option<Vec<Registration>> {
        match self {
            Self::Legacy => None,
            Self::Explicit(registration) => Some(registration.iter().cloned().collect()),
        }
    }

    /// Validate the bounded registry before accepting any physical-data ownership.
    pub(crate) fn decode(records: Option<Vec<Registration>>, project: &Project) -> Result<Self, String> {
        let Some(mut records) = records else { return Ok(Self::Legacy) };
        if records.len() > 1 {
            return Err("this format supports zero or one physical board".into());
        }
        let registration = records.pop();
        if let Some(registration) = &registration {
            registration.validate(project)?;
        }
        Ok(Self::Explicit(registration))
    }
}

impl Registration {
    /// Detect UUID collisions that an older typed-address representation could legally contain.
    pub(crate) fn id_collides(&self, project: &Project) -> bool {
        self.id == project.id || project.sheets.iter().any(|sheet| sheet.sheet_id == self.id)
    }

    /// Reject foreign paths, nil/colliding UUIDs and names outside the project naming rules.
    pub(crate) fn validate(&self, project: &Project) -> Result<(), String> {
        if self.id.is_nil() || self.id_collides(project) {
            return Err("invalid or colliding board ID".into());
        }
        if self.file != "board.json" {
            return Err("the single-board format requires file board.json".into());
        }
        let normalized = crate::name_param(&serde_json::json!({"name":self.name})).map_err(|error| error.to_string())?;
        if normalized != self.name {
            return Err("board name must not contain surrounding whitespace".into());
        }
        Ok(())
    }

    /// Allocate a new board identity without deriving it from a display name or path.
    pub(crate) fn new(name: String) -> Result<Self, CommandError> {
        Ok(Self { id: crate::fresh_id()?, name, file: "board.json".into() })
    }
}

impl super::Board {
    /// Create a schematic-only physical-data container; it has no board target or saved file.
    pub(crate) fn schematic_only() -> Self {
        Self { ownership: Ownership::Explicit(None), ..Self::default() }
    }

    /// Honor explicit New PCB intent while keeping ordinary new projects schematic-only.
    pub(crate) fn for_new_project(create_board: bool) -> Result<Self, CommandError> {
        let mut board = Self::schematic_only();
        if create_board {
            board.ownership = Ownership::Explicit(Some(Registration::new("Board".into())?));
        }
        Ok(board)
    }
}

impl Session {
    /// Register the first board atomically; a second creation is refused without mutation.
    pub(crate) fn create_board(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let mut project = self.open_project()?.clone();
        if project.board_target().is_some() {
            return Err(CommandError::Unavailable("this project already has its supported physical board".into()));
        }
        let name = if params.get("name").is_some() { crate::name_param(params)? } else { "Board".into() };
        let registration = Registration::new(name)?;
        registration.validate(&project).map_err(CommandError::InvalidParams)?;
        Arc::make_mut(&mut project.board).ownership = Ownership::Explicit(Some(registration));
        self.change(project);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
