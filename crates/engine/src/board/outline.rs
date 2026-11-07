//! Read compatibility for the superseded local rectangle preview. New edits use layer drawings.
use crate::schematic::Point;
use serde::{Deserialize, Serialize};

/// The earlier local rectangle record, retained for byte-stable read compatibility.
/// New edits materialize layer-addressed drawings, without inferring manufacturing validity.
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

#[cfg(test)]
mod tests;
