//! Derived planar material, backed by the workspace's pinned SolidCore kernel.
//!
//! Inputs use whole nanometres. Curved and intersected boundaries remain private analytic
//! kernel data in millimetres: repeated clipping never rounds them to an integer polygon.
//! A region set is immutable, disposable derived data, with no project-file representation.
//! Operations return a complete replacement or an error without changing either operand.

use pcbcraft_units::{MAX_LENGTH_NM, NM_PER_MM};
use solidcore::{kernel::Kernel, primitive::primitive_creation::p1::offset_profile, prism::profile};
use solidcore_record::{Loop2, Plane, Region2, Vec2};

use crate::{ErrorCode, GeometryError, Result, guard::guard};

mod kernel;

/// Kernel loop/offset tolerance in nanometres. This is not a clearance allowance: consumers
/// must account for approximation and rounding at their own output/contact boundary.
pub const TOLERANCE_NM: f64 = 0.1;
const TOLERANCE_MM: f64 = TOLERANCE_NM / NM_PER_MM as f64;

/// The planar Boolean operation applied to two sets of material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    /// Material in either operand.
    Union,
    /// Material in the first operand outside the second.
    Difference,
    /// Material shared by both operands.
    Intersection,
}

/// A measured area, with the kernel's absolute numerical error bound, in square nanometres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub value_nm2: f64,
    pub error_bound_nm2: f64,
}

/// A set of planar regions with holes. No kernel type crosses this boundary.
///
/// Cloning copies derived profile data, not a live kernel session or mutable B-rep. Sets can
/// therefore be captured in immutable worker results without sharing kernel ownership.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Regions {
    regions: Vec<Region2>,
}

impl Regions {
    /// Construct an implicitly closed polygon from bounded, distinct integer-nanometre vertices.
    /// Winding is normalized; fewer than three vertices, repeated/zero edges, indeterminate
    /// area, and resource/range violations return `invalid_params`. The caller supplies a simple
    /// polygon (authored zones already validate crossings); this is not a self-intersection repair.
    pub fn polygon(points_nm: &[[i64; 2]]) -> Result<Self> {
        if !(3..=256).contains(&points_nm.len()) {
            return Err(invalid("a polygon needs 3–256 vertices"));
        }
        let points = points_nm.iter().copied().map(point_mm).collect::<Result<Vec<_>>>()?;
        if points_nm.iter().enumerate().any(|(i, p)| points_nm[..i].contains(p)) {
            return Err(invalid("polygon vertices must be distinct; closure is implicit"));
        }
        guard("planar polygon", || {
            let record = Region2 { outer: Loop2::polygon(&points), holes: Vec::new() };
            let mut kp = solidcore_convert::region2(&record);
            let positive = kp
                .outer
                .signed_area()
                .map_err(|e| invalid(&e.to_string()))?
                .sign()
                .ok_or_else(|| invalid("polygon area is indeterminate or zero"))?;
            if !positive {
                kp.outer = kp.outer.reversed();
            }
            profile::check_regions(std::iter::once(&kp), TOLERANCE_MM).map_err(|e| invalid(&format!("{e:?}")))?;
            Ok(Self { regions: vec![solidcore_convert::record_region2(&kp)] })
        })
    }

    /// Construct a circular land or clearance obstacle. Its radius must be positive and the
    /// complete circle must lie in the supported coordinate range; half-nanometre radii are
    /// represented by `diameter_nm`, rather than rounding an odd copper width.
    pub fn circle(center_nm: [i64; 2], diameter_nm: i64) -> Result<Self> {
        let center = point_mm(center_nm)?;
        if diameter_nm <= 0 || diameter_nm > MAX_LENGTH_NM * 2 {
            return Err(invalid("a circle needs a positive bounded diameter"));
        }
        let radius_nm = diameter_nm as f64 / 2.;
        if center_nm.iter().any(|v| v.unsigned_abs() as f64 + radius_nm > MAX_LENGTH_NM as f64) {
            return Err(invalid("circle exceeds the supported coordinate range"));
        }
        Ok(Self { regions: vec![Region2 { outer: Loop2::circle(center, radius_nm / NM_PER_MM as f64), holes: Vec::new() }] })
    }

    /// Whether this result contains no material. Empty material is a successful Boolean result.
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// Number of separate regions; each can contain its own holes.
    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    /// Owned individual regions, retaining holes, for island filtering and connectivity.
    pub fn islands(&self) -> impl Iterator<Item = Self> + '_ {
        self.regions.iter().cloned().map(|region| Self { regions: vec![region] })
    }

    /// Clip two immutable operands, preserving curved boundaries. Kernel profile/resource
    /// refusals or failed operations return an error, never a partial set of material.
    pub fn boolean(&self, other: &Self, operation: Operation) -> Result<Self> {
        guard("planar Boolean", || checked(kernel::boolean(&self.regions, &other.regions, operation).map_err(kernel_error)?))
    }

    /// Expand material by a positive distance, or erode it by a negative distance, with rounded
    /// corners. Hole boundaries move in the opposite sense to outer boundaries. A fresh kernel
    /// owns all temporary B-reps, so a failed operation cannot poison a later offset or operand.
    ///
    /// The pinned kernel may refuse degenerate/collapsed offsets or profiles it cannot read
    /// back. Such a refusal is `operation_failed`, not an empty-fill or successful-clearance
    /// claim. Callers must keep that result unavailable until a complete calculation succeeds.
    pub fn offset(&self, distance_nm: i64) -> Result<Self> {
        if distance_nm.unsigned_abs() > MAX_LENGTH_NM as u64 {
            return Err(invalid("offset exceeds the supported range"));
        }
        if self.is_empty() || distance_nm == 0 {
            return Ok(self.clone());
        }
        guard("planar offset", || {
            let mut kernel = Kernel::default();
            let frame = solidcore_convert::plane(&Plane::XY);
            let mut regions = Vec::new();
            // Offset each material region, then union the results: expanded islands can merge.
            let mut result = Self::default();
            for region in &self.regions {
                let profile = solidcore_convert::region2(region);
                let curves = profile
                    .loops()
                    .flat_map(|lp| &lp.segs)
                    .map(|seg| profile::curve_on_frame(&kernel, &frame, 0., seg))
                    .collect::<solidcore::Result<Vec<_>>>()
                    .map_err(kernel_error)?;
                let body = offset_profile(
                    &mut kernel,
                    &curves,
                    TOLERANCE_MM,
                    distance_nm.unsigned_abs() as f64 / NM_PER_MM as f64,
                    if distance_nm < 0 { 1 } else { 2 },
                    true,
                    false,
                )
                .map_err(kernel_error)?;
                regions.extend(kernel::read(&kernel, &body).map_err(kernel_error)?);
            }
            for region in regions {
                result = result.boolean(&checked(vec![region])?, Operation::Union)?;
            }
            Ok(result)
        })
    }

    /// Analytic signed area of the material (holes subtracted), with accumulated numerical
    /// error. This does not use a tessellated approximation or a rounded output polygon.
    pub fn area(&self) -> Result<Area> {
        guard("planar area", || {
            let mut total = Area { value_nm2: 0., error_bound_nm2: 0. };
            for region in &self.regions {
                let area = solidcore_convert::region2(region).signed_area().map_err(kernel_error)?;
                let value = area.value * (NM_PER_MM as f64).powi(2);
                let error = area.bound * (NM_PER_MM as f64).powi(2);
                // Include unit conversion and accumulation rounding in the numerical bound.
                total.error_bound_nm2 += error + 4. * f64::EPSILON * (total.value_nm2.abs() + value.abs() + error);
                total.value_nm2 += value;
            }
            Ok(total)
        })
    }

    /// Classify an integer point: `Some(true)` in material, `Some(false)` outside, `None` on
    /// a boundary or within the kernel predicate's rounding. Boundary uncertainty must not be
    /// promoted to physical connectivity. Invalid coordinates are refused before the kernel.
    pub fn contains(&self, point_nm: [i64; 2]) -> Result<Option<bool>> {
        let point = point_mm(point_nm)?;
        guard("planar containment", || {
            let mut boundary = false;
            for region in &self.regions {
                match solidcore_convert::loop2(&region.outer).holds(solidcore_convert::vec2(point)) {
                    Some(false) => continue,
                    None => {
                        boundary = true;
                        continue;
                    }
                    Some(true) => {}
                }
                let mut in_hole = false;
                let mut hole_boundary = false;
                for hole in &region.holes {
                    match solidcore_convert::loop2(hole).holds(solidcore_convert::vec2(point)) {
                        Some(true) => in_hole = true,
                        None => hole_boundary = true,
                        Some(false) => {}
                    }
                }
                if !in_hole && !hole_boundary {
                    return Ok(Some(true));
                }
                boundary |= hole_boundary;
            }
            Ok(if boundary { None } else { Some(false) })
        })
    }
}

/// Check the complete kernel result before publication; no output bypasses the kernel's
/// finite-coordinate, closure, segment or work-budget checks.
fn checked(regions: Vec<Region2>) -> Result<Regions> {
    let profiles: Vec<_> = regions.iter().map(solidcore_convert::region2).collect();
    profile::check_regions(profiles.iter(), TOLERANCE_MM)
        .map_err(|e| GeometryError::new(ErrorCode::OperationFailed, format!("planar result: {e:?}")))?;
    if regions.iter().flat_map(Region2::loops).flat_map(|lp| &lp.segs).any(|s| !bounded_segment(s)) {
        return Err(GeometryError::new(ErrorCode::OperationFailed, "planar result exceeds the supported coordinate range"));
    }
    Ok(Regions { regions })
}

/// Conservatively bound defining coordinates. Positive rational weights keep a free-form
/// curve in its control hull; an arc is bounded by its complete supporting circle. Refusal of
/// a near-limit partial arc is preferable to publishing out-of-range derived geometry.
fn bounded_segment(segment: &solidcore_record::Seg2) -> bool {
    use solidcore_record::Seg2;
    let limit = MAX_LENGTH_NM as f64 / NM_PER_MM as f64;
    let bounded = |p: Vec2| p.x.is_finite() && p.y.is_finite() && p.x.abs() <= limit && p.y.abs() <= limit;
    match *segment {
        Seg2::Line { a, b } => [a, b].into_iter().all(bounded),
        Seg2::Arc { center, radius, .. } => bounded(center) && center.x.abs() + radius <= limit && center.y.abs() + radius <= limit,
        Seg2::Cubic { p0, p1, p2, p3 } => [p0, p1, p2, p3].into_iter().all(bounded),
        Seg2::Conic { a, apex, b, .. } => [a, apex, b].into_iter().all(bounded),
        Seg2::Bezier { n, ref p } => p.get(..=n as usize).is_some_and(|points| points.iter().copied().all(bounded)),
    }
}

/// Convert exact supported integer coordinates at the adapter boundary, without rounding
/// intermediate intersections back to nanometres.
fn point_mm(point: [i64; 2]) -> Result<Vec2> {
    if point.iter().any(|v| v.unsigned_abs() > MAX_LENGTH_NM as u64) {
        return Err(invalid("coordinate exceeds the supported range"));
    }
    Ok(Vec2::new(point[0] as f64 / NM_PER_MM as f64, point[1] as f64 / NM_PER_MM as f64))
}

/// Invalid arguments have one stable command-facing error code.
fn invalid(message: &str) -> GeometryError {
    GeometryError::new(ErrorCode::InvalidParams, message)
}

/// Lower-level failures retain their explanation and never become empty material.
fn kernel_error(error: solidcore::SolidError) -> GeometryError {
    GeometryError::new(ErrorCode::OperationFailed, format!("planar operation: {error}"))
}

#[cfg(test)]
mod tests;
