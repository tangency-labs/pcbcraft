//! Derived planar material, backed by the workspace's pinned SolidCore kernel.
//!
//! Inputs use whole nanometres. Curved and intersected boundaries remain private analytic
//! kernel data in millimetres: repeated clipping never rounds them to an integer polygon.
//! A region set is immutable, disposable derived data, with no project-file representation.
//! Operations return a complete replacement or an error without changing either operand.

use pcbcraft_units::{MAX_LENGTH_NM, NM_PER_MM};
use solidcore::{primitive::primitive_creation::p1::offset_profile, prism::profile};
use solidcore_record::{Loop2, Plane, Region2, Vec2};
use std::sync::{Arc, OnceLock};

use crate::{ErrorCode, GeometryError, Result, guard::guard};

mod bounds;
mod kernel;

/// Kernel loop/offset tolerance in nanometres. This is not a clearance allowance: consumers
/// must account for approximation and rounding at their own output/contact boundary.
pub const TOLERANCE_NM: f64 = 0.1;
const TOLERANCE_MM: f64 = TOLERANCE_NM / NM_PER_MM as f64;
const BOX_RESERVE_MM: f64 = 0.000_050;
/// Assembled faces retain many independently checked loops without sending that
/// total edge count through an offset/Boolean operand's profile validator.
const MAX_ASSEMBLED_SEGMENTS: usize = 4 * profile::MAX_SEGMENTS;
type Complement = (Vec<Loop2>, Vec<Region2>);

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

/// Display-only triangles in board nanometres. Connectivity and clearance continue to use
/// the analytic regions, never these approximated surface triangles.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct DisplayMesh {
    pub vertices_nm: Vec<[f64; 2]>,
    pub triangles: Vec<[u32; 3]>,
}

/// A set of planar regions with holes. No kernel type crosses this boundary.
///
/// Cloning copies profile data and shares a lazily prepared operand. Operations copy that
/// operand under a short lock into their own kernel; they never edit the cached body.
/// The cache follows this derived value's lifetime and is excluded from worker transport.
#[derive(Clone, Default)]
pub struct Regions {
    regions: Vec<Region2>,
    prepared: Arc<OnceLock<Result<Option<kernel::Prepared>>>>,
    loop_bounds: Arc<OnceLock<Vec<Vec<bounds::Bounds>>>>,
    assembled: bool,
}

impl std::fmt::Debug for Regions {
    /// Keep disposable kernel topology out of profile diagnostics.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Regions").field("regions", &self.regions).finish()
    }
}

impl PartialEq for Regions {
    /// Preparation does not change material identity.
    fn eq(&self, other: &Self) -> bool {
        self.regions == other.regions
    }
}

impl Regions {
    /// Encode ephemeral worker transport for this kernel pin. This opaque representation is
    /// private to identically built worker/host modules and must never enter native files.
    pub fn worker_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&(1_u32, "73bdb6cdd39efc3899fb54e7217289a630a89efe", &self.regions)).map_err(|e| invalid(&e.to_string()))
    }

    /// Validate ephemeral worker transport before admitting it to the geometry cache. The
    /// size, version, pin, finite profiles and conservative bounds are all checked.
    pub fn from_worker_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(invalid("planar worker payload is too large"));
        }
        let (version, pin, regions): (u32, String, Vec<Region2>) = serde_json::from_slice(bytes).map_err(|e| invalid(&e.to_string()))?;
        if version != 1 || pin != "73bdb6cdd39efc3899fb54e7217289a630a89efe" {
            return Err(invalid("incompatible planar worker payload"));
        }
        guard("planar worker transport", || checked_assembled(regions))
    }
    /// Expand an existing physical copper core by its exact represented radius, including
    /// half-nanometre values. Repeated capsule/point vertices are collapsed before choosing
    /// the kernel's polygon offset or the analytic circle/capsule primitive.
    pub fn rounded_core(core_nm: &[[i64; 2]], radius_nm: f64) -> Result<Self> {
        if core_nm.is_empty() || core_nm.len() > 256 || !radius_nm.is_finite() || radius_nm < 0. || radius_nm > MAX_LENGTH_NM as f64 {
            return Err(invalid("a rounded core needs bounded vertices and a finite nonnegative radius"));
        }
        guard("rounded copper core", || {
            let mut points = core_nm.to_vec();
            points.dedup();
            if points.len() > 1 && points.first() == points.last() {
                points.pop();
            }
            if points.len() >= 3 {
                return Self::polygon(&points)?.offset_distance(radius_nm);
            }
            if radius_nm == 0. {
                return Ok(Self::default());
            }
            let a = point_mm(points[0])?;
            let radius = radius_nm / NM_PER_MM as f64;
            if points.len() == 1 {
                return checked(vec![Region2 { outer: Loop2::circle(a, radius), holes: Vec::new() }]);
            }
            let b = point_mm(points[1])?;
            let angle = (b.y - a.y).atan2(b.x - a.x);
            let normal = Vec2::new(-angle.sin() * radius, angle.cos() * radius);
            let segments = vec![
                solidcore_record::Seg2::Line { a: a - normal, b: b - normal },
                solidcore_record::Seg2::Arc { center: b, radius, start: angle - std::f64::consts::FRAC_PI_2, sweep: std::f64::consts::PI },
                solidcore_record::Seg2::Line { a: b + normal, b: a + normal },
                solidcore_record::Seg2::Arc { center: a, radius, start: angle + std::f64::consts::FRAC_PI_2, sweep: std::f64::consts::PI },
            ];
            checked(vec![Region2 { outer: Loop2 { segs: segments }, holes: Vec::new() }])
        })
    }

    /// Tessellate material and holes with SolidCore's shared display mesher. Missing, lenient
    /// or over-budget faces are errors rather than a silently partial copper picture.
    pub fn display_mesh(&self, chord_nm: i64) -> Result<DisplayMesh> {
        if !(1..=1_000_000).contains(&chord_nm) {
            return Err(invalid("display chord tolerance must be 1–1000000 nm"));
        }
        guard("planar display mesh", || kernel::display_mesh(self, chord_nm))
    }
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
            Ok(Self { regions: vec![solidcore_convert::record_region2(&kp)], ..Self::default() })
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
        Ok(Self { regions: vec![Region2 { outer: Loop2::circle(center, radius_nm / NM_PER_MM as f64), holes: Vec::new() }], ..Self::default() })
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
        self.regions.iter().cloned().map(|region| {
            if self.regions.len() == 1 { self.clone() } else { Self { regions: vec![region], assembled: self.assembled, ..Self::default() } }
        })
    }

    /// Conservative aggregate envelope in board nanometres, with outward rounding.
    pub fn bounds_nm(&self) -> Option<[f64; 4]> {
        (!self.is_empty()).then(|| {
            let b = self.profile_bounds().iter().map(|region| region[0]).fold(bounds::empty(), bounds::join);
            envelope_nm(b)
        })
    }

    /// Place a local line/arc profile on the board without recomputing its curves.
    /// Bounds and closure are checked in the new frame. Unsupported curves or a
    /// translation outside the model range can use the caller's absolute construction.
    pub fn place_profile(&self, origin_nm: [i64; 2]) -> Option<Self> {
        use solidcore_record::Seg2;

        let shift = point_mm(origin_nm).ok()?;
        let mut regions = self.regions.clone();
        for lp in regions.iter_mut().flat_map(|region| std::iter::once(&mut region.outer).chain(&mut region.holes)) {
            for segment in &mut lp.segs {
                match segment {
                    Seg2::Line { a, b } => {
                        *a += shift;
                        *b += shift;
                    }
                    Seg2::Arc { center, .. } => *center += shift,
                    _ => return None,
                }
            }
        }
        checked(regions).ok()
    }

    /// Reuse immutable loop envelopes during repeated point queries and candidate pruning.
    fn profile_bounds(&self) -> &[Vec<bounds::Bounds>] {
        self.loop_bounds.get_or_init(|| self.regions.iter().map(|region| region.loops().map(bounds::loop_bounds).collect()).collect())
    }

    /// Prove that an entire box is in material. An inconclusive bound returns false, so
    /// callers can fall back to an exact Boolean. A box cannot cross a contour that it
    /// does not meet, and its corner must also be strictly inside the outer loop.
    pub fn covers_box(&self, envelope_nm: [f64; 4]) -> Result<bool> {
        guard("planar box containment", || Ok(self.covering_region(envelope_nm.map(|value| value / 1e6)).is_some()))
    }

    /// Reject a contact envelope only when it is entirely outside material or inside a hole.
    /// A boundary match or uncertain containment still requires an exact intersection.
    pub fn may_intersect_box(&self, envelope_nm: [f64; 4]) -> Result<bool> {
        guard("planar box contact", || {
            let envelope = envelope_nm.map(|value| value / 1e6);
            if envelope.iter().any(|value| !value.is_finite()) || envelope[0] > envelope[2] || envelope[1] > envelope[3] {
                return Err(invalid("contact box must be finite and ordered"));
            }
            let point = solidcore_convert::vec2(Vec2::new(envelope[0], envelope[1]));
            Ok(self.regions.iter().zip(self.profile_bounds()).any(|(region, profiles)| {
                if !bounds::near(profiles[0], envelope, BOX_RESERVE_MM) {
                    return false;
                }
                let meets = |lp: &Loop2| lp.segs.iter().any(|seg| bounds::meets_box(seg, envelope, BOX_RESERVE_MM));
                if !meets(&region.outer) && solidcore_convert::loop2(&region.outer).holds(point) == Some(false) {
                    return false;
                }
                !region.holes.iter().zip(&profiles[1..]).any(|(hole, hole_bounds)| {
                    bounds::near(*hole_bounds, envelope, BOX_RESERVE_MM) && !meets(hole) && solidcore_convert::loop2(hole).holds(point) == Some(true)
                })
            }))
        })
    }

    /// Work in kernel units and protect the proof from near-coincident boundary cases.
    fn covering_region(&self, envelope: bounds::Bounds) -> Option<usize> {
        let point = solidcore_convert::vec2(Vec2::new(envelope[0], envelope[1]));
        self.regions.iter().zip(self.profile_bounds()).position(|(region, region_bounds)| {
            bounds::near(region_bounds[0], envelope, BOX_RESERVE_MM)
                && !region_bounds[1..].iter().any(|hole| bounds::near(*hole, envelope, BOX_RESERVE_MM))
                && !region.outer.segs.iter().any(|seg| bounds::meets_box(seg, envelope, BOX_RESERVE_MM))
                && solidcore_convert::loop2(&region.outer).holds(point) == Some(true)
        })
    }

    /// Subtract independently enclosed cutters from their owning faces. A cutter with nested
    /// or overlapping region envelopes gets a Boolean in its own local rectangle. The result
    /// retains its copper islands without passing unrelated holes through that Boolean.
    /// None means that the enclosure proof could not establish this case.
    pub fn subtract_enclosed(&self, cutters: &[Self]) -> Result<Option<Self>> {
        guard("planar enclosed holes", || {
            const RESERVE: f64 = 0.000_050;
            let mut assembled = self.regions.clone();
            let mut retained = Vec::new();
            let mut envelopes = Vec::new();
            for cutter in cutters {
                let Some(envelope_nm) = cutter.bounds_nm() else { continue };
                let envelope = envelope_nm.map(|coord| coord / 1e6);
                let Some(owner) = self.covering_region(envelope) else { return Ok(None) };
                if envelopes.iter().any(|previous| bounds::near(*previous, envelope, RESERVE)) {
                    return Ok(None);
                }
                envelopes.push(envelope);
                let Some((holes, islands)) = cutter.complement_loops(envelope_nm)? else { return Ok(None) };
                assembled[owner].holes.extend(holes);
                retained.extend(islands);
            }
            assembled.extend(retained);
            let mut value = checked_assembled(assembled)?;
            match kernel::prepare_enclosed(&value.regions) {
                Ok(operand) => {
                    let _ = value.prepared.set(Ok(Some(operand)));
                    value.assembled = true;
                    Ok(Some(value))
                }
                Err(_) => Ok(None),
            }
        })
    }

    /// Subtract independently grouped cuts whose source envelopes were proved wholly inside
    /// this stock and separated from every other group's sources. Result envelopes can
    /// overlap, or one group can sit inside a void of another. Keep that nesting without
    /// uniting the entire cut set. A nonsimple or ambiguous result uses the caller's Boolean.
    pub fn subtract_independent_enclosed(&self, cutters: &[Self]) -> Result<Option<Self>> {
        guard("planar independent holes", || Ok(self.assemble_independent(cutters)))
    }

    /// Attach each independent cut to its innermost material face. No boundary of another
    /// group can cross the cut, so one point on its contour classifies the whole contour.
    fn assemble_independent(&self, cutters: &[Self]) -> Option<Self> {
        let mut hosts: Vec<_> = self.regions.iter().cloned().map(|region| (None, region)).collect();
        let mut cuts = Vec::new();
        for (group, cutter) in cutters.iter().enumerate() {
            let Some(envelope) = cutter.bounds_nm() else { continue };
            let (holes, islands) = cutter.complement_loops(envelope).ok()??;
            hosts.extend(islands.into_iter().map(|island| (Some(group), island)));
            cuts.extend(holes.into_iter().map(|hole| (group, hole)));
        }
        let mut host_order: Vec<_> = (0..hosts.len())
            .map(|id| {
                let envelope = bounds::loop_bounds(&hosts[id].1.outer);
                let box_area = (envelope[2] - envelope[0]) * (envelope[3] - envelope[1]);
                (id, envelope, box_area)
            })
            .collect();
        host_order.sort_by(|a, b| a.2.total_cmp(&b.2));
        for (group, hole) in cuts {
            let point = bounds::first_point(&hole)?;
            let sample = [point.x, point.y, point.x, point.y];
            let point = solidcore_convert::vec2(point);
            let mut owner = None;
            for &(id, envelope, _) in &host_order {
                let (source, region) = &hosts[id];
                if *source == Some(group) || !bounds::near(envelope, sample, BOX_RESERVE_MM) {
                    continue;
                }
                if solidcore_convert::loop2(&region.outer).holds(point)? {
                    owner = Some(id);
                    break;
                }
            }
            hosts[owner?].1.holes.push(hole);
        }
        let mut value = checked_assembled(hosts.into_iter().map(|(_, region)| region).collect()).ok()?;
        let operand = kernel::prepare_enclosed(&value.regions).ok()?;
        let _ = value.prepared.set(Ok(Some(operand)));
        value.assembled = true;
        Some(value)
    }

    /// Get the hole loops and the material trapped inside an individual cutter. A nested
    /// operand is resolved by the analytic kernel within its own envelope.
    fn complement_loops(&self, envelope_nm: [f64; 4]) -> Result<Option<Complement>> {
        let parts = self.profile_bounds();
        if parts.iter().enumerate().any(|(i, part)| parts[..i].iter().any(|other| bounds::near(part[0], other[0], BOX_RESERVE_MM))) {
            return Ok(self.local_complement(envelope_nm));
        }
        let mut holes = Vec::new();
        let mut islands = Vec::new();
        for region in &self.regions {
            let Some(hole) = oriented_loop(&region.outer, false)? else { return Ok(None) };
            holes.push(hole);
            for inner in &region.holes {
                let Some(outer) = oriented_loop(inner, true)? else { return Ok(None) };
                islands.push(Region2 { outer, holes: Vec::new() });
            }
        }
        Ok(Some((holes, islands)))
    }

    /// Extract the inside boundary of a local complement. The cutter is strictly inside
    /// the rectangle, so exactly one result face owns the rectangle's perimeter. Only its
    /// holes and the other, enclosed faces affect the original stock.
    fn local_complement(&self, envelope_nm: [f64; 4]) -> Option<Complement> {
        const MARGIN_NM: i64 = 1_000;
        let [x0, y0, x1, y1] = envelope_nm;
        let (x0, y0) = (x0.floor() as i64 - MARGIN_NM, y0.floor() as i64 - MARGIN_NM);
        let (x1, y1) = (x1.ceil() as i64 + MARGIN_NM, y1.ceil() as i64 + MARGIN_NM);
        let rectangle = Self::polygon(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]]).ok()?;
        let mut complement = rectangle.boolean(self, Operation::Difference).ok()?;
        let index = complement
            .profile_bounds()
            .iter()
            .position(|bounds| bounds[0].iter().zip(rectangle.profile_bounds()[0][0]).all(|(a, b)| (a - b).abs() < 0.000_050))?;
        let shell = complement.regions.remove(index);
        Some((shell.holes, complement.regions))
    }

    /// Clip two immutable operands, preserving curved boundaries. Kernel profile/resource
    /// refusals or failed operations return an error, never a partial set of material.
    pub fn boolean(&self, other: &Self, operation: Operation) -> Result<Self> {
        guard("planar Boolean", || kernel::boolean(self, other, operation))
    }

    /// Unite a batch in balanced kernel rounds, converting each source region once. Holes
    /// remain attached to their own material; empty inputs produce empty material.
    pub fn union_all(inputs: &[Self]) -> Result<Self> {
        guard("planar union batch", || {
            let regions = inputs.iter().flat_map(|input| input.regions.iter().cloned()).collect();
            let input = checked(regions)?;
            let mut kernel = kernel::context();
            kernel::union(&mut kernel, &input.regions)
        })
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
        self.offset_distance(distance_nm as f64)
    }

    /// Preserve fractional physical pad radii at the kernel boundary; authored dimensions
    /// continue to use the public whole-nanometre offset entry point.
    fn offset_distance(&self, distance_nm: f64) -> Result<Self> {
        if self.is_empty() || distance_nm == 0. {
            return Ok(self.clone());
        }
        guard("planar offset", || {
            let mut kernel = kernel::context();
            let frame = solidcore_convert::plane(&Plane::XY);
            let mut regions = Vec::new();
            // Offset each material region, then union the results: expanded islands can merge.
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
                    distance_nm.abs() / NM_PER_MM as f64,
                    if distance_nm < 0. { 1 } else { 2 },
                    true,
                    false,
                )
                .map_err(kernel_error)?;
                regions.extend(kernel::read(&kernel, &body).map_err(kernel_error)?);
            }
            let expanded = checked(regions)?;
            kernel::union(&mut kernel, &expanded.regions)
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
            let query = [point.x, point.y, point.x, point.y];
            for (region, region_bounds) in self.regions.iter().zip(self.profile_bounds()) {
                if !bounds::near(query, region_bounds[0], TOLERANCE_MM) {
                    continue;
                }
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
                for (hole, envelope) in region.holes.iter().zip(&region_bounds[1..]) {
                    if !bounds::near(query, *envelope, TOLERANCE_MM) {
                        continue;
                    }
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

/// Orient a loop as material or as a hole, refusing an indeterminate area.
fn oriented_loop(record: &Loop2, positive: bool) -> Result<Option<Loop2>> {
    let lp = solidcore_convert::loop2(record);
    Ok(lp
        .signed_area()
        .map_err(kernel_error)?
        .sign()
        .map(|sign| if sign == positive { record.clone() } else { solidcore_convert::record_loop2(&lp.reversed()) }))
}

/// Round envelopes outward when moving from kernel millimetres to board nanometres.
fn envelope_nm(b: bounds::Bounds) -> [f64; 4] {
    [(b[0] * 1e6).floor(), (b[1] * 1e6).floor(), (b[2] * 1e6).ceil(), (b[3] * 1e6).ceil()]
}

/// Check the complete kernel result before publication; no output bypasses the kernel's
/// finite-coordinate, closure, segment or work-budget checks.
fn checked(regions: Vec<Region2>) -> Result<Regions> {
    let profiles: Vec<_> = regions.iter().map(solidcore_convert::region2).collect();
    profile::check_regions(profiles.iter(), TOLERANCE_MM)
        .map_err(|e| GeometryError::new(ErrorCode::OperationFailed, format!("planar result: {e:?}")))?;
    checked_coordinates(regions)
}

/// Validate a face assembled from disjoint, independently enclosed loops. The
/// total face has its own resource bound, while each constituent loop still
/// goes through the kernel's ordinary segment and closure validation.
fn checked_assembled(regions: Vec<Region2>) -> Result<Regions> {
    let count = regions.iter().flat_map(Region2::loops).map(|lp| lp.segs.len()).sum::<usize>();
    if count <= profile::MAX_SEGMENTS {
        return checked(regions);
    }
    if count > MAX_ASSEMBLED_SEGMENTS {
        return Err(GeometryError::new(ErrorCode::OperationFailed, "assembled planar result exceeds its segment budget"));
    }
    for lp in regions.iter().flat_map(Region2::loops) {
        let profile = profile::Region2 { outer: solidcore_convert::loop2(lp), holes: Vec::new() };
        profile::check_regions(std::iter::once(&profile), TOLERANCE_MM)
            .map_err(|e| GeometryError::new(ErrorCode::OperationFailed, format!("planar result: {e:?}")))?;
    }
    checked_coordinates(regions)
}

/// Point, arc and control-hull coordinates remain bounded for every result and transport.
fn checked_coordinates(regions: Vec<Region2>) -> Result<Regions> {
    if regions.iter().flat_map(Region2::loops).flat_map(|lp| &lp.segs).any(|s| !bounded_segment(s)) {
        return Err(GeometryError::new(ErrorCode::OperationFailed, "planar result exceeds the supported coordinate range"));
    }
    Ok(Regions { regions, ..Regions::default() })
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
