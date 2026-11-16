//! The board's placement origin and the output frame shared by every manufacturing file.
//!
//! Board coordinates grow Y downward on screen. Outputs are written from the top with Y up and
//! measured from one named origin, so fabrication and placement files of the same board agree.
use super::view::BoardView;
use crate::{CommandError, Session, schematic::Point};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

/// Replace the addressed board's placement origin, or restore the built-in one with null.
pub const SET_PLACEMENT_ORIGIN: &str = "pcb.set_placement_origin";
/// The saved vocabulary of a placement origin record.
const FORMAT: &str = "pcbcraft-origin/1";

/// Where output coordinates are measured from. Requests and saved files are read through
/// the same strict path, so both refuse fields an anchor's kind does not use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", try_from = "Value")]
pub enum Anchor {
    /// Board coordinate zero, the built-in default.
    BoardZero,
    /// The lower-left corner of the Board Outline's bounding box, followed when the outline changes.
    OutlineLowerLeft,
    /// A fixed point in board coordinates (integer nanometres, Y down).
    Point { x_nm: i64, y_nm: i64 },
}

/// The tagged form an anchor is read from before its fields are checked.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Tagged {
    BoardZero,
    OutlineLowerLeft,
    Point { x_nm: i64, y_nm: i64 },
}

impl TryFrom<Value> for Anchor {
    type Error = String;

    /// Read an anchor, refusing unknown kinds, fields its kind does not use and out-of-range
    /// points. Tagged unit variants would otherwise accept and drop extra fields.
    fn try_from(value: Value) -> Result<Self, String> {
        let anchor = match serde_json::from_value(value.clone()).map_err(|e| e.to_string())? {
            Tagged::BoardZero => Self::BoardZero,
            Tagged::OutlineLowerLeft => Self::OutlineLowerLeft,
            Tagged::Point { x_nm, y_nm } => Self::Point { x_nm, y_nm },
        };
        if serde_json::to_value(anchor).ok() != Some(value) {
            return Err("The placement origin has fields its kind does not use.".into());
        }
        anchor.validate()?;
        Ok(anchor)
    }
}

impl Anchor {
    /// A fixed point must lie inside the board coordinate range.
    fn validate(self) -> Result<(), String> {
        match self {
            Self::Point { x_nm, y_nm } if !crate::wire::valid_point(&Point { x_nm, y_nm }) => {
                Err("The placement origin must lie inside the board coordinate range.".into())
            }
            _ => Ok(()),
        }
    }
}

/// The board's authored origin with a stable identity that outputs can name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    /// Supported vocabulary, currently pcbcraft-origin/1.
    pub format: String,
    /// Stable identity retained while the anchor changes.
    pub id: Uuid,
    /// What the origin is attached to.
    pub anchor: Anchor,
}

impl Origin {
    /// Unsupported versions, nil identities and out-of-range points are refused on load.
    pub fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT || self.id.is_nil() {
            return Err("Unsupported placement origin format or invalid origin identity.".into());
        }
        self.anchor.validate()
    }
}

/// Whether the origin was authored on the board or is the built-in default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// No origin is saved; outputs use board zero.
    Builtin,
    /// The board's saved origin, qualified by its UUID.
    Board { origin: Uuid },
}

/// The origin an output used: what it is attached to, where that is, and the axis convention.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Resolved {
    /// The authored or built-in anchor.
    pub anchor: Anchor,
    /// Where the anchor came from.
    pub source: Source,
    /// The anchor's location in board coordinates (Y down).
    pub at: Point,
    /// Output axes: always "x_right_y_up", seen from the top.
    pub axes: &'static str,
}

impl Resolved {
    /// The transform from board coordinates to this origin's output frame.
    pub fn frame(self) -> Frame {
        Frame { origin: self.at }
    }
}

/// Board-to-output coordinates: shift to the origin, then point Y up as seen from the top.
/// Top-side rotations need no change; counterclockwise on screen is counterclockwise seen from
/// the top. [`Frame::place`] writes bottom-side placements under a [`BottomSide`] convention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    origin: Point,
}

impl Frame {
    /// Board zero, the frame used when no origin has been chosen.
    pub const BOARD_ZERO: Self = Self { origin: Point { x_nm: 0, y_nm: 0 } };

    /// Express a board point in output coordinates. Exact: both inputs are bounded by the board
    /// coordinate range, so the difference cannot overflow.
    pub fn to_output(self, point: Point) -> Point {
        Point { x_nm: point.x_nm - self.origin.x_nm, y_nm: self.origin.y_nm - point.y_nm }
    }

    /// Express a footprint placement in output terms. Top-side placements keep the output X
    /// and the board rotation under both conventions; `convention` decides how bottom-side rows
    /// are written.
    /// Any rotation is reduced to 0 through 359. Exact for points within the board coordinate
    /// range, which every placement and origin is.
    pub fn place(self, at: Point, rotation: u16, on_bottom: bool, convention: BottomSide) -> Placed {
        let at = self.to_output(at);
        let rotation = rotation % 360;
        match (on_bottom, convention) {
            (false, _) | (true, BottomSide::ViewedFromTop) => Placed { at, rotation },
            (true, BottomSide::ViewedFromBottom) => Placed { at: Point { x_nm: -at.x_nm, y_nm: at.y_nm }, rotation: (360 - rotation) % 360 },
        }
    }
}

/// How placement rows of bottom-side parts are written. Top-side rows are the same in both.
///
/// A bottom placement at board rotation `r` is its library footprint with local X mirrored and
/// then turned `r` counterclockwise, seen from the top.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BottomSide {
    /// Seen from the top like every other row: X is not mirrored and the rotation is the board
    /// rotation, counterclockwise from the top; the mirror is implied by the side. Positive
    /// rotation stays counterclockwise as viewed from the top on the bottom side as well
    /// (IPC-2581, 3.3.2).
    #[default]
    ViewedFromTop,
    /// Seen from below, as the part sits on the bottom of the board: X is negated about the
    /// origin and the rotation is counterclockwise seen from below, `(360 - r) mod 360`. Seen
    /// this way the part is its unmirrored library footprint at that rotation.
    ViewedFromBottom,
}

/// A footprint placement in an output frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Placed {
    /// The placement point in output coordinates (integer nanometres).
    pub at: Point,
    /// Counterclockwise whole degrees from the library zero orientation in the convention's view,
    /// 0 through 359.
    pub rotation: u16,
}

impl<'a> BoardView<'a> {
    /// The origin saved on this board, if one has been authored.
    pub fn authored_placement_origin(self) -> Option<&'a Origin> {
        self.project().board.placement_origin.as_ref()
    }

    /// Resolve the placement origin without changing the board. Without a saved origin this is
    /// board zero. An outline-attached origin needs Board Outline geometry and fails without it.
    pub fn placement_origin(self) -> Result<Resolved, String> {
        let (anchor, source) = self
            .authored_placement_origin()
            .map_or((Anchor::BoardZero, Source::Builtin), |origin| (origin.anchor, Source::Board { origin: origin.id }));
        let at = match anchor {
            Anchor::BoardZero => Point { x_nm: 0, y_nm: 0 },
            Anchor::Point { x_nm, y_nm } => Point { x_nm, y_nm },
            Anchor::OutlineLowerLeft => {
                let (min, max) = self.outline_bounds().ok_or("Draw a Board Outline before using its lower-left corner as the placement origin.")?;
                Point { x_nm: min.x_nm, y_nm: max.y_nm }
            }
        };
        Ok(Resolved { anchor, source, at, axes: "x_right_y_up" })
    }
}

/// The shared catalog accepts one anchor object or null.
pub(crate) fn schema() -> Value {
    let coordinate = json!({"type":"integer","minimum":-crate::schematic::MAX_CENTER_NM,"maximum":crate::schematic::MAX_CENTER_NM});
    json!({"oneOf":[
        {"type":"null"},
        {"type":"object","properties":{"kind":{"const":"board_zero"}},"required":["kind"],"additionalProperties":false},
        {"type":"object","properties":{"kind":{"const":"outline_lower_left"}},"required":["kind"],"additionalProperties":false},
        {"type":"object","properties":{"kind":{"const":"point"},"x_nm":coordinate,"y_nm":coordinate},"required":["kind","x_nm","y_nm"],"additionalProperties":false}
    ]})
}

/// Parse a request's anchor, refusing unknown kinds, extra fields and out-of-range points.
pub(crate) fn parse(value: &Value) -> Result<Option<Anchor>, String> {
    if value.is_null() {
        return Ok(None);
    }
    Anchor::try_from(value.clone()).map(Some)
}

impl Session {
    /// Replace the board's placement origin in one revision-checked undo step, keeping an
    /// existing origin UUID. Null removes it. Repeating the current anchor or resetting a board
    /// without one does not advance history.
    pub(crate) fn set_placement_origin(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let next = match parse(&params["origin"]).map_err(CommandError::InvalidParams)? {
            None => None,
            Some(anchor) if project.board.placement_origin.as_ref().is_some_and(|origin| origin.anchor == anchor) => return Ok(()),
            Some(anchor) => Some(Origin {
                format: FORMAT.into(),
                id: match &project.board.placement_origin {
                    Some(origin) => origin.id,
                    None => crate::fresh_id()?,
                },
                anchor,
            }),
        };
        if project.board.placement_origin == next {
            return Ok(());
        }
        let mut candidate = project.clone();
        Arc::make_mut(&mut candidate.board).placement_origin = next;
        self.change(candidate)
    }
}

#[cfg(test)]
mod tests;
