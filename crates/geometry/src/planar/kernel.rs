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
use solidcore_record::{Plane, Region2};

use super::{Operation, TOLERANCE_MM};

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
pub(super) fn boolean(a: &[Region2], b: &[Region2], operation: Operation) -> Result<Vec<Region2>> {
    if a.is_empty() || b.is_empty() {
        return Ok(match operation {
            Operation::Union => a.iter().chain(b).cloned().collect(),
            Operation::Difference => a.to_vec(),
            Operation::Intersection => Vec::new(),
        });
    }
    let mut kernel = context();
    let Some(a) = operand(&mut kernel, a)? else {
        return Err(refused("nonempty operand has no material"));
    };
    let Some(b) = operand(&mut kernel, b)? else {
        return Err(refused("nonempty operand has no material"));
    };
    let operation = match operation {
        Operation::Union => Boolean2d::Union,
        Operation::Difference => Boolean2d::Difference,
        Operation::Intersection => Boolean2d::Intersection,
    };
    let body = combine(&mut kernel, a, b, operation)?;
    read(&kernel, &body)
}

/// Read analytic loops, accepting empty material only after the topology walk proves it has
/// no faces. A generic status/error from either the Boolean or traversal is never swallowed.
pub(super) fn read(kernel: &Kernel, body: &Brep) -> Result<Vec<Region2>> {
    if solidcore::topology::brep::p1::faces(body, None)?.is_empty() {
        return Ok(Vec::new());
    }
    let frame = solidcore_convert::plane(&Plane::XY);
    Ok(sheet::sheet_regions(kernel, body, &frame, TOLERANCE_MM)?.regions.iter().map(solidcore_convert::record_region2).collect())
}

/// Convert each completed offset region once, then combine its owned sheet in balanced
/// rounds. Previously a left fold rebuilt every earlier region on every added island.
pub(super) fn union(kernel: &mut Kernel, regions: &[Region2]) -> Result<Vec<Region2>> {
    match operand(kernel, regions)? {
        Some(body) => read(kernel, &body),
        None => Ok(Vec::new()),
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
pub(super) fn display_mesh(regions: &[Region2], chord_nm: i64) -> crate::Result<super::DisplayMesh> {
    let mut kernel = context();
    let Some(mut body) = operand(&mut kernel, regions).map_err(super::kernel_error)? else {
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
