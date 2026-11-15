//! Native copper command adapters shared by the original example layouts.
use pcbcraft_engine::{ADD_TRACKS, ADD_VIA, Session, schematic::Point};
use serde_json::json;
use std::error::Error;
use uuid::Uuid;

/// Add a visible route through the same validation/history command used by the editor.
/// Report its geometry on rejection so generator failures identify the route to review.
pub(crate) fn route(session: &mut Session, layer: Uuid, width_nm: u64, start: Point, bends: &[(f64, f64)], end: Point) -> Result<(), Box<dyn Error>> {
    let points: Vec<_> = std::iter::once(start).chain(bends.iter().map(|&(x, y)| mm(x, y))).chain(std::iter::once(end)).collect();
    session
        .apply(ADD_TRACKS, &json!({"layer":layer,"width_nm":width_nm,"points":points,"expected_revision":session.revision()}))
        .map_err(|e| format!("copper {points:?}: {e}"))?;
    Ok(())
}
/// Finish a short front-side fanout with an explicit 0.6/0.3 mm through via, outside its pad.
pub(crate) fn fanout(
    session: &mut Session,
    layer: Uuid,
    width_nm: u64,
    start: Point,
    bends: &[(f64, f64)],
    end: Point,
) -> Result<(), Box<dyn Error>> {
    route(session, layer, width_nm, start, bends, end)?;
    session.apply(ADD_VIA, &json!({"at":end,"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":session.revision()}))?;
    Ok(())
}
/// Convert original layout dimensions in millimetres into the native integer nanometre grid.
pub(crate) fn mm(x: f64, y: f64) -> Point {
    Point { x_nm: (x * 1_000_000.).round() as i64, y_nm: (y * 1_000_000.).round() as i64 }
}
