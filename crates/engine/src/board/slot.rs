//! Geometry of finished plated obround holes, independent of the surrounding copper land.
use super::{definition::MAX_OFFSET_NM, geometry::Contour};
use crate::{
    CommandError,
    schematic::{MAX_CENTER_NM, Point},
};

const FULL_TURN_MDEG: u32 = 360_000;
const QUARTER_TURN_MDEG: u32 = FULL_TURN_MDEG / 4;
// A placed pad centre can include both rotated axes of the bounded local footprint offset.
const MAX_PAD_CENTER_NM: i64 = MAX_CENTER_NM + 2 * MAX_OFFSET_NM;

/// Finished size and orientation of a plated through-slot in footprint coordinates.
/// The angle is independent of the land angle and uses counterclockwise millidegrees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(try_from = "SlotFields")]
pub struct PlatedSlot {
    width_nm: i64,
    length_nm: i64,
    rotation_mdeg: u32,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SlotFields {
    width_nm: i64,
    length_nm: i64,
    rotation_mdeg: u32,
}

impl TryFrom<SlotFields> for PlatedSlot {
    type Error = CommandError;

    /// Decode native and command fields through the same dimension and angle validation.
    fn try_from(fields: SlotFields) -> Result<Self, Self::Error> {
        Self::new(fields.width_nm, fields.length_nm, fields.rotation_mdeg)
    }
}

impl PlatedSlot {
    /// Require a positive finished width, total length at least the width, and a canonical angle.
    /// Width and length use the existing footprint dimension limit. Equal dimensions yield a
    /// round outline without changing the authored hole kind.
    pub fn new(width_nm: i64, length_nm: i64, rotation_mdeg: u32) -> Result<Self, CommandError> {
        if !(1..=MAX_OFFSET_NM).contains(&width_nm) || !(width_nm..=MAX_OFFSET_NM).contains(&length_nm) || rotation_mdeg >= FULL_TURN_MDEG {
            return Err(CommandError::InvalidParams(
                "a plated slot needs positive bounded dimensions, length at least width, and rotation in 0..360000 millidegrees".into(),
            ));
        }
        Ok(Self { width_nm, length_nm, rotation_mdeg })
    }

    /// Finished width, overall length (including both caps) and local rotation, in nm and mdeg.
    pub fn dimensions(self) -> (i64, i64, u32) {
        (self.width_nm, self.length_nm, self.rotation_mdeg)
    }

    /// Carry the independent slot axis through the same authoring transform as its land,
    /// preserving the fractional-degree part rather than rounding it to the land's unit.
    pub(crate) fn transformed(self, transform: crate::library_graphics::Transform) -> Self {
        let whole = self.rotation_mdeg / 1_000;
        let fraction = (self.rotation_mdeg % 1_000) as i32;
        let sign = if transform.mirror_x != transform.mirror_y { -1 } else { 1 };
        let angle = i32::from(transform.angle(whole as i16)) * 1_000 + sign * fraction;
        Self { rotation_mdeg: angle.rem_euclid(FULL_TURN_MDEG as i32) as u32, ..self }
    }

    /// Report the native board-space angle with the same mirror-then-rotate convention as the hole.
    pub(crate) fn placement_angle(self, footprint_rotation: u16, bottom: bool) -> u32 {
        let angle = if bottom { FULL_TURN_MDEG + 2 * QUARTER_TURN_MDEG - self.rotation_mdeg } else { self.rotation_mdeg };
        (angle + u32::from(footprint_rotation) * 1_000) % FULL_TURN_MDEG
    }

    /// Resolve about the already-placed pad centre: mirror local X on the back, then apply
    /// the footprint rotation. Offsets round once to the board's integer nanometer grid;
    /// the cap radius retains half-nanometer precision. Out-of-range centres are refused.
    pub fn resolve(self, center: Point, footprint_rotation: u16, bottom: bool) -> Option<ResolvedPlatedSlot> {
        if !(-MAX_PAD_CENTER_NM..=MAX_PAD_CENTER_NM).contains(&center.x_nm) || !(-MAX_PAD_CENTER_NM..=MAX_PAD_CENTER_NM).contains(&center.y_nm) {
            return None;
        }
        let angle = self.placement_angle(footprint_rotation, bottom);
        let spine = self.length_nm - self.width_nm;
        let lower = -spine / 2;
        let upper = lower + spine;
        let offsets = if angle.is_multiple_of(QUARTER_TURN_MDEG) {
            [lower, upper].map(|x| super::rotate(x, 0, (angle / 1_000) as u16))
        } else {
            let half = spine as f64 / 2.;
            let (sin, cos) = (f64::from(angle) / 1_000.).to_radians().sin_cos();
            [-half, half].map(|x| ((x * cos).round() as i64, (-x * sin).round() as i64))
        };
        let cap_centres = offsets.map(|(dx, dy)| Point { x_nm: center.x_nm + dx, y_nm: center.y_nm + dy });
        Some(ResolvedPlatedSlot { cap_centres, width_nm: self.width_nm })
    }
}

/// A board-space capsule, shared between inspection, painting and geometric checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedPlatedSlot {
    cap_centres: [Point; 2],
    width_nm: i64,
}

impl ResolvedPlatedSlot {
    /// Centres of the two round caps, not the extreme ends of the finished hole.
    pub fn cap_centres(self) -> [Point; 2] {
        self.cap_centres
    }

    /// Move disposable preview geometry with its pad; the isometry must preserve width.
    pub fn map_cap_centres(self, map: impl FnMut(Point) -> Point) -> Self {
        Self { cap_centres: self.cap_centres.map(map), ..self }
    }

    /// Diameter of both round caps and the perpendicular width of the straight section.
    pub fn width_nm(self) -> i64 {
        self.width_nm
    }

    /// The same capsule primitives used for board copper clearance and contact queries.
    pub fn contour(self) -> Contour {
        let [start, end] = self.cap_centres;
        Contour::capsule(start, end, self.width_nm as f64 / 2.)
    }

    /// Minimum signed copper margin around the entire hole; negative means breakout.
    /// A supported land is convex. Its inset by the cap radius plus the required margin
    /// is convex too, so enclosing both cap discs also encloses their connecting strip.
    pub fn annular_ring_nm(self, land: &Contour) -> f64 {
        let [start, end] = self.cap_centres;
        land.boundary_margin_nm(start).min(land.boundary_margin_nm(end)) - self.width_nm as f64 / 2.
    }
}

#[cfg(test)]
mod fixture;
#[cfg(test)]
mod tests;
