//! Kernel call recipes adapted from `solidcore-ops/src/prism.rs` at the workspace pin
//! (Apache-2.0, solidcore contributors). No clipping or offset algorithm is reimplemented.
//! Each call owns its kernel, avoiding the ops process-global session and fix configuration.
//! Unlike the pinned ops reader, the adapter handles a proved faceless result as empty
//! before calling the public face query, which deliberately refuses empty lists.

use solidcore::{
    Brep, Result, SolidError,
    boolean::api_brep::{Boolean2dOutcome, boolean_2d},
    kernel::{Kernel, KernelOptions, Tolerances},
    primitive::{api_trim_ops::create_planar_faces, primitive_creation::Boolean2d},
    prism::{profile, sheet},
};
use solidcore_record::{Loop2, Plane, Region2, Seg2, Vec2};

use super::{Operation, Regions, TOLERANCE_MM};
use std::sync::Mutex;

/// One immutable prepared operand. Copying may populate kernel geometry memos, so concurrent
/// readers serialize only the independent-body copy, never the Boolean or meshing work.
pub(super) struct Prepared {
    body: Mutex<Brep>,
}

/// Build material/holes once, preserving the existing operand construction and error semantics.
fn prepare(regions: &[Region2]) -> crate::Result<Option<Prepared>> {
    operand(&mut context(), regions).map(|body| body.map(|body| Prepared { body: Mutex::new(body) })).map_err(super::kernel_error)
}

/// Copy a prepared operand into a fresh operation context. Never reuse poisoned topology after
/// an unwound copy, and never let an operation mutate another reader's source body.
fn copy_operand(kernel: &mut Kernel, regions: &Regions) -> crate::Result<Option<Brep>> {
    if regions.is_empty() {
        return Ok(None);
    }
    let prepared = regions
        .prepared
        .get_or_init(|| {
            if regions.assembled { prepare_enclosed(&regions.regions).map(Some).map_err(super::kernel_error) } else { prepare(&regions.regions) }
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let Some(prepared) = prepared else { return Ok(None) };
    let body =
        prepared.body.lock().map_err(|_| crate::GeometryError::new(crate::ErrorCode::KernelInternal, "planar operand copy was interrupted"))?;
    solidcore::topology::brep::p3::brep_copy(kernel, &body).map(Some).map_err(super::kernel_error)
}

/// PCB model units are millimetres, so configure the B-rep/intersection defaults as well as
/// the profile reader. Passing a loop tolerance alone leaves the default 10 nm body tolerance.
pub(super) fn context() -> Kernel {
    Kernel::new(&KernelOptions {
        tolerances: Tolerances {
            zone_tol_3d: TOLERANCE_MM,
            xsect_tol_3d: 2. * TOLERANCE_MM,
            approx_tol_3d: TOLERANCE_MM / 2.,
            ..Tolerances::default()
        },
        ..KernelOptions::default()
    })
}

/// Clip material using a fresh kernel and propagate every error before returning any result.
pub(super) fn boolean(a: &Regions, b: &Regions, operation: Operation) -> crate::Result<Regions> {
    if a.is_empty() || b.is_empty() {
        return super::checked(match operation {
            Operation::Union => a.regions.iter().chain(&b.regions).cloned().collect(),
            Operation::Difference => a.regions.clone(),
            Operation::Intersection => Vec::new(),
        });
    }
    let mut kernel = context();
    let Some(a) = copy_operand(&mut kernel, a)? else {
        return Err(super::kernel_error(refused("nonempty operand has no material")));
    };
    let Some(b) = copy_operand(&mut kernel, b)? else {
        return Err(super::kernel_error(refused("nonempty operand has no material")));
    };
    let operation = match operation {
        Operation::Union => Boolean2d::Union,
        Operation::Difference => Boolean2d::Difference,
        Operation::Intersection => Boolean2d::Intersection,
    };
    let body = combine(&mut kernel, a, b, operation).map_err(super::kernel_error)?;
    finish(&kernel, body)
}

/// Validate analytic output before retaining its already constructed operand. The body belongs
/// only to this new value, so subsequent queries need no profile-to-topology reconstruction.
fn finish(kernel: &Kernel, body: Brep) -> crate::Result<Regions> {
    let regions = super::checked(read(kernel, &body).map_err(super::kernel_error)?)?;
    let _ = regions.prepared.set(Ok(Some(Prepared { body: Mutex::new(body) })));
    Ok(regions)
}

/// Read analytic loops, accepting empty material only after the topology walk proves it has
/// no faces. A generic status/error from either the Boolean or traversal is never swallowed.
pub(super) fn read(kernel: &Kernel, body: &Brep) -> Result<Vec<Region2>> {
    if solidcore::topology::brep::p1::faces(body, None)?.is_empty() {
        return Ok(Vec::new());
    }
    let frame = solidcore_convert::plane(&Plane::XY);
    let mut regions: Vec<Region2> =
        sheet::sheet_regions(kernel, body, &frame, TOLERANCE_MM)?.regions.iter().map(solidcore_convert::record_region2).collect();
    // SolidCore resolves some crossings within its fixed internal zone tolerance (1e-5 mm, not
    // the configured one): a crossing 2 nm from an arc's end joins that end vertex while the
    // arc keeps its full sweep, and the edge records the tolerance it is held to. Read back
    // without that tolerance, such a loop is open by up to twice the largest edge tolerance.
    // The body is queried only when a loop is open: the result is cached as a later operand,
    // and a closed read must leave it exactly as the Boolean produced it.
    if !regions.iter().flat_map(Region2::loops).any(has_gap) {
        return Ok(regions);
    }
    let mut within = 0f64;
    for edge in solidcore::topology::brep::p1::edges(body, None)? {
        within = within.max(solidcore::topology::edge::tolerance(body, edge)?);
    }
    for region in &mut regions {
        close(&mut region.outer, 2. * within);
        region.holes.iter_mut().for_each(|hole| close(hole, 2. * within));
    }
    Ok(regions)
}

/// Close each junction gap the kernel holds within edge tolerance by trimming the segment left
/// untrimmed: the computed crossing lies on both curves, the stale end on only one, so the end
/// whose curve passes closer to the other's point moves. No connector is added; the kernel
/// would merge its ends within its zone tolerance when the profile is read back as an operand.
/// Larger gaps stay open for `checked` to refuse.
fn close(lp: &mut Loop2, within: f64) {
    let n = lp.segs.len();
    for i in 0..n {
        let j = (i + 1) % n;
        if i == j {
            continue;
        }
        let (Some((_, end)), Some((start, _))) = (ends(&lp.segs[i]), ends(&lp.segs[j])) else { continue };
        let gap = (start.x - end.x).hypot(start.y - end.y);
        if gap <= TOLERANCE_MM || gap > within {
            continue;
        }
        if off_curve(&lp.segs[i], start) <= off_curve(&lp.segs[j], end) {
            lp.segs[i] = moved(lp.segs[i], start, false);
        } else {
            lp.segs[j] = moved(lp.segs[j], end, true);
        }
    }
}

/// Whether any junction between consecutive lines/arcs is open by more than the read tolerance.
fn has_gap(lp: &Loop2) -> bool {
    let n = lp.segs.len();
    (0..n).filter(|&i| (i + 1) % n != i).any(|i| match (ends(&lp.segs[i]), ends(&lp.segs[(i + 1) % n])) {
        (Some((_, end)), Some((start, _))) => (start.x - end.x).hypot(start.y - end.y) > TOLERANCE_MM,
        _ => false,
    })
}

/// Start and end points of a line or arc; `None` for other curves, whose gaps stay open.
fn ends(seg: &Seg2) -> Option<(Vec2, Vec2)> {
    match *seg {
        Seg2::Line { a, b } => Some((a, b)),
        Seg2::Arc { center, radius, start, sweep } => {
            let at = |angle: f64| Vec2::new(center.x + radius * angle.cos(), center.y + radius * angle.sin());
            Some((at(start), at(start + sweep)))
        }
        _ => None,
    }
}

/// Distance from `p` to the segment's supporting line or circle.
fn off_curve(seg: &Seg2, p: Vec2) -> f64 {
    match *seg {
        Seg2::Line { a, b } => ((b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)).abs() / (b.x - a.x).hypot(b.y - a.y),
        Seg2::Arc { center, radius, .. } => ((p.x - center.x).hypot(p.y - center.y) - radius).abs(),
        _ => f64::INFINITY,
    }
}

/// Move the segment's start (or end) to `p`; an arc keeps its circle and turns to `p`'s angle.
fn moved(seg: Seg2, p: Vec2, at_start: bool) -> Seg2 {
    let wrap = |d: f64| d - std::f64::consts::TAU * (d / std::f64::consts::TAU).round();
    match seg {
        Seg2::Line { a, b } => {
            if at_start {
                Seg2::Line { a: p, b }
            } else {
                Seg2::Line { a, b: p }
            }
        }
        Seg2::Arc { center, radius, start, sweep } => {
            let angle = (p.y - center.y).atan2(p.x - center.x);
            if at_start {
                let delta = wrap(angle - start);
                Seg2::Arc { center, radius, start: start + delta, sweep: sweep - delta }
            } else {
                Seg2::Arc { center, radius, start, sweep: sweep + wrap(angle - (start + sweep)) }
            }
        }
        other => other,
    }
}

/// Convert each completed offset region once, then combine its owned sheet in balanced
/// rounds. Previously a left fold rebuilt every earlier region on every added island.
pub(super) fn union(kernel: &mut Kernel, regions: &[Region2]) -> crate::Result<Regions> {
    match operand(kernel, regions).map_err(super::kernel_error)? {
        Some(body) => finish(kernel, body),
        None => Ok(Regions::default()),
    }
}

/// Consume two complete temporary sheets and retain only a successfully assigned result.
fn combine(kernel: &mut Kernel, a: Brep, b: Brep, operation: Boolean2d) -> Result<Brep> {
    match boolean_2d(kernel, a, b, operation)? {
        Boolean2dOutcome::Result(body) => Ok(body),
        Boolean2dOutcome::Unassigned { .. } => Err(refused("Boolean did not assign a result")),
    }
}

/// Unite sheets in balanced rounds, avoiding a left fold over progressively larger operands.
fn unite(kernel: &mut Kernel, mut sheets: Vec<Brep>) -> Result<Option<Brep>> {
    while sheets.len() > 1 {
        let mut next = Vec::new();
        let mut iter = sheets.into_iter();
        while let Some(a) = iter.next() {
            match iter.next() {
                Some(b) => next.push(combine(kernel, a, b, Boolean2d::Union)?),
                None => next.push(a),
            }
        }
        sheets = next;
    }
    Ok(sheets.pop())
}

/// Build faces from explicitly oriented loops after the adapter has proved that all
/// holes are inside the outer contour and separated. The kernel receives the grouping
/// directly and need not union the holes or rediscover containment.
pub(super) fn prepare_enclosed(regions: &[Region2]) -> Result<Prepared> {
    use solidcore::{
        error::EntityRef,
        geom::{tol::USE_DEFAULT, vector3d::Vec3},
        nurbs::b_spline_surface_n_lib::p2::OrientType,
        surface::{plane::Plane as KernelPlane, surface::Surface},
        topology::{
            brep::{
                p2::ArenaNotifyState,
                p3::new_brep,
                p7::{FaceCurvesInput, make_face_with_curves},
            },
            loopuse::notify_ref,
        },
    };

    let mut kernel = context();
    let mut body = new_brep(&mut kernel, USE_DEFAULT, USE_DEFAULT)?;
    let frame = solidcore_convert::plane(&Plane::XY);
    for region in regions {
        let profile = solidcore_convert::region2(region);
        let loops = sheet::region_loops(&profile, TOLERANCE_MM)?;
        if loops.material.len() != 1 || loops.holes.len() != region.holes.len() {
            return Err(refused("enclosed loops must be simple"));
        }
        let loops = loops.material.into_iter().chain(loops.holes.into_iter().map(|hole| hole.reversed()));
        let mut input = FaceCurvesInput::default();
        let mut curves = Vec::new();
        for lp in loops {
            input.loops.push(lp.segs.len());
            for segment in &lp.segs {
                curves.push(Some(profile::curve_on_frame(&kernel, &frame, 0., segment)?));
            }
        }
        input.orientations = vec![OrientType::Same; curves.len()];
        input.curves_3d = Some(curves);
        let surface = Surface::from(KernelPlane::from_normal(Vec3::ZERO, Vec3::Z));
        let domain = surface.natural_uv_domain_with_nurb(&kernel)?;
        let mut notify = ArenaNotifyState::new(&body);
        let unbounded = body.infinite_region();
        let made = make_face_with_curves(
            &mut kernel,
            &mut body,
            unbounded,
            input,
            surface,
            &domain,
            OrientType::Same,
            None,
            &|face| notify_ref(EntityRef::Face(face)),
            &mut notify.env(None),
        )?;
        if made.face.is_none() {
            return Err(refused("enclosed loops did not make a planar face"));
        }
    }
    Ok(Prepared { body: Mutex::new(body) })
}

/// Build one sheet per simple loop using the kernel's profile conversion and face constructor.
fn faces(kernel: &mut Kernel, loops: &[profile::Loop2]) -> Result<Vec<Brep>> {
    let frame = solidcore_convert::plane(&Plane::XY);
    loops
        .iter()
        .map(|lp| {
            let curves = lp.segs.iter().map(|seg| profile::curve_on_frame(kernel, &frame, 0., seg)).collect::<Result<Vec<_>>>()?;
            create_planar_faces(kernel, &curves)
        })
        .collect()
}

/// Make each region's material minus its own holes before uniting regions. This preserves a
/// separate island inside another region's hole instead of subtracting the hole from both.
fn operand(kernel: &mut Kernel, regions: &[Region2]) -> Result<Option<Brep>> {
    let mut sheets = Vec::new();
    for region in regions {
        let region = solidcore_convert::region2(region);
        let loops = sheet::region_loops(&region, TOLERANCE_MM)?;
        let material = faces(kernel, &loops.material)?;
        let Some(material) = unite(kernel, material)? else {
            return Err(refused("region has no material"));
        };
        let holes = faces(kernel, &loops.holes)?;
        sheets.push(match unite(kernel, holes)? {
            Some(holes) => combine(kernel, material, holes, Boolean2d::Difference)?,
            None => material,
        });
    }
    unite(kernel, sheets)
}

/// Produce complete display triangles from the same analytic region sheet. The mesher's
/// refusal/cap reports are checked before handing a mesh to the canvas.
pub(super) fn display_mesh(regions: &Regions, chord_nm: i64) -> crate::Result<super::DisplayMesh> {
    let mut kernel = context();
    let Some(mut body) = copy_operand(&mut kernel, regions)? else {
        return Ok(super::DisplayMesh::default());
    };
    let fail = |e: String| crate::GeometryError::new(crate::ErrorCode::OperationFailed, e);
    let (record, _) = solidcore_convert::brep::to_record(&mut kernel, &mut body).map_err(|e| fail(e.to_string()))?;
    let (plan, stats) =
        solidcore_tess::plan(&record, &solidcore_tess::MeshTolerance::display(chord_nm as f64 / 1e6), 1.).map_err(|e| fail(e.to_string()))?;
    if !stats.unmeshed.is_empty() || !stats.lenient.is_empty() || !stats.over_tolerance.is_empty() || !stats.edges_capped.is_empty() {
        return Err(fail("SolidCore could not produce a complete planar display mesh at the requested tolerance".into()));
    }
    let (mesh, refused) = plan.evaluate();
    if !refused.is_empty() {
        return Err(fail("SolidCore refused planar display evaluation".into()));
    }
    Ok(super::DisplayMesh { vertices_nm: mesh.positions.into_iter().map(|p| [p.x * 1e6, p.y * 1e6]).collect(), triangles: mesh.triangles })
}

/// An unsupported topology construction is explicit instead of an inferred empty region.
fn refused(reason: &'static str) -> SolidError {
    SolidError::Unsupported { operation: "pcbcraft::planar", reason }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// An interrupted copy poisons only that derived operand; an independently decoded source
    /// still works. A caught kernel unwind must never authorize reuse of possibly changed memos.
    #[test]
    fn poisoned_operand_is_refused_without_poisoning_an_independent_source() {
        let region = Regions::polygon(&[[0, 0], [1_000_000, 0], [1_000_000, 1_000_000], [0, 1_000_000]]).unwrap();
        let result = region.boolean(&region, Operation::Intersection).unwrap();
        let bytes = result.worker_bytes().unwrap();
        let prepared = result.prepared.get().unwrap().as_ref().unwrap().as_ref().unwrap();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _guard = prepared.body.lock().unwrap();
                panic!("simulated interrupted operand copy");
            }))
            .is_err()
        );
        assert_eq!(result.boolean(&region, Operation::Intersection).unwrap_err().code(), crate::ErrorCode::KernelInternal);
        let independent = Regions::from_worker_bytes(&bytes).unwrap();
        assert!(!independent.boolean(&region, Operation::Intersection).unwrap().is_empty());
    }
}
