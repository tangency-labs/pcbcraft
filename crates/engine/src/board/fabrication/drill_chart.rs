//! The drill chart of one fabrication export: a row for every tool of each drill file written.

use super::{DRILL_DECIMALS, DrillPlating, mm};
use crate::schematic::Point;
use serde::Serialize;
use std::collections::BTreeMap;

/// One drill tool as its drill file declares it, and how many holes that file drills with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DrillChartRow {
    /// The drill file the tool belongs to.
    pub plating: DrillPlating,
    /// The tool number declared in that file; `T1` is 1.
    pub tool: usize,
    /// The hole diameter the tool was generated from.
    pub diameter_nm: i64,
    /// The diameter exactly as the tool declaration writes it, in millimeters.
    pub size_mm: String,
    /// Holes drilled with this tool.
    pub count: usize,
}

/// The drill tools of one export: the plated file's tools first, then the non-plated file's,
/// each in declaration order. It is generated with the drill files, never edited by hand.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DrillChart {
    pub rows: Vec<DrillChartRow>,
}

impl DrillChart {
    /// Describe the hole census handed to the drill writer. Tools are numbered as the writer
    /// numbers them: from 1 in ascending diameter, separately in each file. A non-plated census
    /// with no holes writes no file and contributes no rows.
    pub(super) fn new(plated: &BTreeMap<i64, Vec<Point>>, non_plated: &BTreeMap<i64, Vec<Point>>) -> Self {
        let rows = [(DrillPlating::Plated, plated), (DrillPlating::NonPlated, non_plated)]
            .into_iter()
            .flat_map(|(plating, holes)| {
                holes.iter().enumerate().map(move |(index, (&diameter_nm, points))| DrillChartRow {
                    plating,
                    tool: index + 1,
                    diameter_nm,
                    size_mm: mm(diameter_nm, DRILL_DECIMALS),
                    count: points.len(),
                })
            })
            .collect();
        Self { rows }
    }
}
