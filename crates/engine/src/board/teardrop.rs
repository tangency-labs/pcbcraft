//! In-memory intent and derived copper for a centered straight track entering a round via.
//! Nothing here registers a board field, edit command, or copper consumer.

use super::view::{BoardTarget, BoardView};
use crate::schematic::{MAX_CENTER_NM, Point};
use uuid::Uuid;

#[cfg(test)]
mod tests;

/// Requested axial extension past the via's outer land, before applying any cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Length {
    /// Whole percent of the via's outer diameter. Values over 100 are allowed.
    Percent(u16),
    /// Absolute extension in integer nanometres.
    Nanometres(i64),
}

/// Sizing intent; limits are not design-rule or manufacturing clearances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parameters {
    pub length: Length,
    pub max_length_nm: Option<i64>,
    /// Upper bound on the finished via-side chord as a whole percent of outer diameter.
    pub width_pct: u16,
    pub max_width_nm: Option<i64>,
}

impl Parameters {
    /// Require positive percentages, and absolute dimensions in the board coordinate range.
    pub fn validate(self) -> Result<(), Error> {
        let valid_nm = |value| (1..=MAX_CENTER_NM).contains(&value);
        let length_valid = match self.length {
            Length::Percent(pct) => pct != 0,
            Length::Nanometres(nm) => valid_nm(nm),
        };
        if !length_valid || self.width_pct == 0 || !self.max_length_nm.is_none_or(valid_nm) || !self.max_width_nm.is_none_or(valid_nm) {
            return Err(Error::InvalidSize);
        }
        Ok(())
    }

    /// Resolve the extension and chord cap. Round positive percentages to nearest nm, ties up.
    fn for_via(self, diameter_nm: i64) -> (i64, i64) {
        let percent = |value| (diameter_nm * i64::from(value) + 50) / 100;
        let length = match self.length {
            Length::Percent(pct) => percent(pct),
            Length::Nanometres(nm) => nm,
        };
        (length.min(self.max_length_nm.unwrap_or(MAX_CENTER_NM)), percent(self.width_pct).min(self.max_width_nm.unwrap_or(MAX_CENTER_NM)))
    }
}

/// Board-local intent for a straight via teardrop. Neither coordinates nor nets are authored here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViaTeardrop {
    pub id: Uuid,
    pub board: BoardTarget,
    pub via: Uuid,
    pub track: Uuid,
    pub parameters: Parameters,
}

/// A disposable copper outline for one board view, not a second authored copper item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StraightCopper {
    pub board: BoardTarget,
    pub source: Uuid,
    pub layer: Uuid,
    /// A strictly convex, clockwise quad in board coordinates (Y down), with no repeated close point.
    /// Recompute from the current view rather than reusing these vertices after an edit.
    pub vertices: [Point; 4],
}

/// Reasons the local record or straight-via contour could not be constructed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidSize,
    InvalidIdentity,
    WrongBoard,
    MissingTrack,
    MissingVia,
    NotCentered,
    TrackTooShort,
    NoTaper,
    WidthLimit,
    UnrepresentableContour,
}

impl ViaTeardrop {
    /// Resolve both source objects on the addressed immutable board. This only constructs
    /// an outline; it does not check clearance, fit to other objects, or change state.
    pub fn derive(self, board: BoardView<'_>) -> Result<StraightCopper, Error> {
        self.parameters.validate()?;
        if self.id.is_nil() || self.track.is_nil() || self.via.is_nil() || self.id == self.track || self.id == self.via || self.track == self.via {
            return Err(Error::InvalidIdentity);
        }
        if self.board != board.target {
            return Err(Error::WrongBoard);
        }
        let tracks = board.tracks();
        let vias = board.vias();
        let track = &tracks[tracks.binary_search_by_key(&self.track, |track| track.id).map_err(|_| Error::MissingTrack)?];
        let via = &vias[vias.binary_search_by_key(&self.via, |via| via.id).map_err(|_| Error::MissingVia)?];
        let outer = if track.start == via.at {
            track.end
        } else if track.end == via.at {
            track.start
        } else {
            return Err(Error::NotCentered);
        };
        let (extension, width_cap) = self.parameters.for_via(via.diameter_nm);
        let vertices = tangent_quad(via.at, outer, via.diameter_nm, track.width_nm, extension, width_cap)?;
        Ok(StraightCopper { board: self.board, source: self.id, layer: track.layer, vertices })
    }
}

/// Join each track-side corner to the same-side tangent on the circular land.
/// Use the via center as origin, +X along the track and +Y clockwise from the track.
/// Projection rounds each board coordinate to nearest nm, ties away from zero.
fn tangent_quad(
    center: Point,
    outer: Point,
    diameter_nm: i64,
    track_width_nm: i64,
    extension_nm: i64,
    width_cap_nm: i64,
) -> Result<[Point; 4], Error> {
    let r = diameter_nm as f64 / 2.;
    let h = track_width_nm as f64 / 2.;
    if h >= r {
        return Err(Error::NoTaper);
    }
    let track_len2 = distance_squared(center, outer);
    let twice_x = i128::from(diameter_nm) + 2 * i128::from(extension_nm);
    if twice_x.pow(2) > 4 * track_len2 {
        return Err(Error::TrackTooShort);
    }
    let x = twice_x as f64 / 2.;
    // For corner B=(x,h), a tangent point T satisfies |T|=r and B·T=r².
    // Choose the outer solution using the perpendicular J B=(-h,x).
    let d2 = x * x + h * h;
    let along = r * r / d2;
    let across = r * (d2 - r * r).sqrt() / d2;
    let (tx, ty) = (along * x - across * h, along * h + across * x);
    let track_len = (track_len2 as f64).sqrt();
    let (ux, uy) = ((outer.x_nm - center.x_nm) as f64 / track_len, (outer.y_nm - center.y_nm) as f64 / track_len);
    let vertices = [(tx, -ty), (x, -h), (x, h), (tx, ty)].map(|(a, b)| Point {
        x_nm: (center.x_nm as f64 + (a * ux - b * uy)).round() as i64,
        y_nm: (center.y_nm as f64 + (a * uy + b * ux)).round() as i64,
    });
    if !representable_quad(vertices) {
        return Err(Error::UnrepresentableContour);
    }
    let chord_len2 = distance_squared(vertices[0], vertices[3]);
    if chord_len2 <= distance_squared(vertices[1], vertices[2]) {
        return Err(Error::NoTaper);
    }
    if chord_len2 > i128::from(width_cap_nm).pow(2) {
        return Err(Error::WidthLimit);
    }
    Ok(vertices)
}

/// Squared distance of bounded board points; keep the endpoint and cap tests in integer nm.
fn distance_squared(a: Point, b: Point) -> i128 {
    i128::from(b.x_nm - a.x_nm).pow(2) + i128::from(b.y_nm - a.y_nm).pow(2)
}

/// Reject a rounded vertex beyond board bounds, or any collapsed or reversed turn.
fn representable_quad(vertices: [Point; 4]) -> bool {
    vertices.iter().all(crate::wire::valid_point)
        && (0..4).all(|index| {
            let (a, b, c) = (vertices[index], vertices[(index + 1) % 4], vertices[(index + 2) % 4]);
            i128::from(b.x_nm - a.x_nm) * i128::from(c.y_nm - b.y_nm) > i128::from(b.y_nm - a.y_nm) * i128::from(c.x_nm - b.x_nm)
        })
}
