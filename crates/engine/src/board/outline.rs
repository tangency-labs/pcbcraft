//! Board-local closed boundaries, independent of footprints and display layers.
use crate::{CommandError, Session, schematic::Point};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A validated physical boundary in board nanometers. The first supported shape is one
/// axis-aligned rectangle without cutouts; closure is implicit rather than a duplicated vertex.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case", try_from = "OutlineRecord")]
pub enum Outline {
    /// Ordered opposite corners; minimum coordinates are strictly less than maximum coordinates.
    Rectangle { min: Point, max: Point },
}

#[derive(Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case", deny_unknown_fields)]
enum OutlineRecord {
    Rectangle { min: Point, max: Point },
}

impl TryFrom<OutlineRecord> for Outline {
    type Error = String;
    /// Reject reversed, collapsed and out-of-range corners before adopting any boundary.
    fn try_from(record: OutlineRecord) -> Result<Self, Self::Error> {
        match record {
            OutlineRecord::Rectangle { min, max } => Self::rectangle(min, max),
        }
    }
}

impl Outline {
    /// Construct a positive-area rectangle with both corners inside the board coordinate range.
    pub fn rectangle(min: Point, max: Point) -> Result<Self, String> {
        if !crate::wire::valid_point(&min) || !crate::wire::valid_point(&max) || min.x_nm >= max.x_nm || min.y_nm >= max.y_nm {
            return Err("outline requires ordered corners within coordinate bounds and positive width and height".into());
        }
        Ok(Self::Rectangle { min, max })
    }

    /// Return the boundary's ordered corners, without repeating the implicit closing vertex.
    pub fn corners(self) -> [Point; 4] {
        match self {
            Self::Rectangle { min, max } => [min, Point { x_nm: max.x_nm, y_nm: min.y_nm }, max, Point { x_nm: min.x_nm, y_nm: max.y_nm }],
        }
    }
}

impl Session {
    /// Replace or clear the addressed board's boundary in one undo step. Invalid targets,
    /// stale revisions and malformed geometry leave the project intact; equal input is a no-op.
    pub(crate) fn set_outline(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let outline: Option<Outline> =
            serde_json::from_value(params["outline"].clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?;
        let project = self.open_project()?;
        if project.board.outline != outline {
            let mut project = project.clone();
            std::sync::Arc::make_mut(&mut project.board).outline = outline;
            self.change(project);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
