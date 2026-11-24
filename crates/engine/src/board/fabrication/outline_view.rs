//! The outline view of one fabrication export: the board's closed outline contours and their
//! overall dimensions, in the same output frame as the files.

use super::{Frame, mm};
use crate::schematic::Point;
use serde::Serialize;

/// Overall dimensions are written to 0.01 mm, the usual precision of a fabrication drawing.
const DIMENSION_DECIMALS: usize = 2;

/// The closed Board Outline drawings of one export and the box that encloses them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OutlineView {
    /// Every closed outline drawing as its vertices in the output frame, in drawing order; the
    /// last vertex joins the first. Cut-outs are contours too.
    pub contours: Vec<Vec<Point>>,
    /// The lower-left corner of the enclosing box in the output frame.
    pub min: Point,
    /// The upper-right corner of the enclosing box in the output frame.
    pub max: Point,
    /// Overall width, `max.x_nm - min.x_nm`.
    pub width_nm: i64,
    /// Overall height, `max.y_nm - min.y_nm`.
    pub height_nm: i64,
    /// The overall width in millimeters, rounded to 0.01 mm.
    pub width_mm: String,
    /// The overall height in millimeters, rounded to 0.01 mm.
    pub height_mm: String,
}

impl OutlineView {
    /// Describe the closed contours among `drawings`, each given as its vertices and whether it
    /// is closed. Open drawings are legacy artwork, not the board edge, and are left out, as
    /// the profile file leaves them out. Outline strokes are linear, so the
    /// vertices bound the drawn edge. Returns `None` without a closed contour.
    pub(super) fn new<'a>(frame: Frame, drawings: impl IntoIterator<Item = (&'a [Point], bool)>) -> Option<Self> {
        let contours: Vec<Vec<Point>> = drawings
            .into_iter()
            .filter(|(points, closed)| *closed && !points.is_empty())
            .map(|(points, _)| points.iter().map(|point| frame.to_output(*point)).collect())
            .collect();
        let mut points = contours.iter().flatten();
        let first = *points.next()?;
        let (min, max) = points.fold((first, first), |(min, max), point| {
            (
                Point { x_nm: min.x_nm.min(point.x_nm), y_nm: min.y_nm.min(point.y_nm) },
                Point { x_nm: max.x_nm.max(point.x_nm), y_nm: max.y_nm.max(point.y_nm) },
            )
        });
        let (width_nm, height_nm) = (max.x_nm - min.x_nm, max.y_nm - min.y_nm);
        Some(Self {
            contours,
            min,
            max,
            width_nm,
            height_nm,
            width_mm: mm(width_nm, DIMENSION_DECIMALS),
            height_mm: mm(height_nm, DIMENSION_DECIMALS),
        })
    }
}
