//! Sheet paper: an ISO A size, its orientation and where its top-left corner sits on the sheet.
//!
//! The paper is presentation metadata owned by one sheet. It never moves authored geometry:
//! choosing a size keeps the top-left corner, and adopting paper for an older sheet places the
//! paper around the existing drawing. Schematic Y points down, so `at` is the paper's top-left
//! corner and the paper extends towards positive X and Y.
//!
//! Paper sizes are ported from Coline (`crates/format/src/sheet.rs`, MIT OR Apache-2.0); see
//! `ATTRIBUTION.md`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::*;
use crate::schematic::{GRID_NM, MAX_CENTER_NM, MAX_PIN_OFFSET_NM, Point, Shape};

/// Set a sheet's paper size, orientation and optional top-left corner as one undo step.
pub const SET_PAGE: &str = "sheet.set_page";

const NM_PER_MM: i64 = 1_000_000;
/// How far inside the paper's edge its plain border runs, in nanometres. Templates replace this
/// border with their own later.
pub const BORDER_INSET_NM: i64 = 5 * NM_PER_MM;
/// Paper must stay inside the schematic coordinate range, like every pin.
const LIMIT_NM: i64 = MAX_CENTER_NM + MAX_PIN_OFFSET_NM;
/// Space kept between an adopted drawing and the paper's edge: the 5 mm border, its 5 mm zone
/// band, and room for text that extends past the measured geometry.
const ADOPT_MARGIN_NM: i64 = 20 * NM_PER_MM;

pub(crate) const PAPER: Param = Param { required: true, name: "paper", summary: "ISO paper size: a4, a3, a2, a1 or a0", kind: ParamKind::Paper };
pub(crate) const ORIENTATION: Param =
    Param { required: true, name: "orientation", summary: "landscape (wider than tall) or portrait", kind: ParamKind::Orientation };
pub(crate) const AT: Param = Param {
    required: false,
    name: "at",
    summary: "The paper's top-left corner in nanometres; omitted keeps the current corner",
    kind: ParamKind::Point,
};

/// ISO A paper sizes, smallest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Paper {
    /// 297 × 210 mm.
    #[default]
    A4,
    /// 420 × 297 mm.
    A3,
    /// 594 × 420 mm.
    A2,
    /// 841 × 594 mm.
    A1,
    /// 1189 × 841 mm.
    A0,
}

impl Paper {
    /// Every size, smallest first.
    pub const ALL: [Self; 5] = [Self::A4, Self::A3, Self::A2, Self::A1, Self::A0];

    /// The name shown in the interface, such as `A4`.
    pub const fn label(self) -> &'static str {
        match self {
            Self::A4 => "A4",
            Self::A3 => "A3",
            Self::A2 => "A2",
            Self::A1 => "A1",
            Self::A0 => "A0",
        }
    }

    /// The command and file spelling, such as `a4`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A4 => "a4",
            Self::A3 => "a3",
            Self::A2 => "a2",
            Self::A1 => "a1",
            Self::A0 => "a0",
        }
    }

    /// Parse the command and file spelling; anything else is `None`.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|paper| paper.as_str() == value)
    }

    /// Long and short side in nanometres.
    pub const fn sides_nm(self) -> (i64, i64) {
        let (long, short) = match self {
            Self::A4 => (297, 210),
            Self::A3 => (420, 297),
            Self::A2 => (594, 420),
            Self::A1 => (841, 594),
            Self::A0 => (1189, 841),
        };
        (long * NM_PER_MM, short * NM_PER_MM)
    }
}

/// Whether the long side of the paper runs across or down the sheet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    /// Wider than tall.
    #[default]
    Landscape,
    /// Taller than wide.
    Portrait,
}

impl Orientation {
    /// Both orientations, landscape first.
    pub const ALL: [Self; 2] = [Self::Landscape, Self::Portrait];

    /// The name shown in the interface.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Landscape => "Landscape",
            Self::Portrait => "Portrait",
        }
    }

    /// The command and file spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Landscape => "landscape",
            Self::Portrait => "portrait",
        }
    }

    /// Parse the command and file spelling; anything else is `None`.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|orientation| orientation.as_str() == value)
    }
}

/// One sheet's paper. Construct it with [`Page::new`], which checks that the paper stays inside
/// the schematic coordinate range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    paper: Paper,
    orientation: Orientation,
    at: Point,
}

impl Page {
    /// A page whose paper lies entirely inside the schematic coordinate range, or an error
    /// naming the problem. `at` is the paper's top-left corner.
    pub fn new(paper: Paper, orientation: Orientation, at: Point) -> Result<Self, String> {
        let page = Self { paper, orientation, at };
        // Reject a corner beyond the range before adding the paper size, which could overflow.
        let in_range = |p: Point| [p.x_nm, p.y_nm].iter().all(|v| (-LIMIT_NM..=LIMIT_NM).contains(v));
        if !in_range(at) || !in_range(page.corners()[1]) {
            return Err(format!("{} {} paper at that corner extends past the schematic's ±1 m range", paper.label(), orientation.as_str()));
        }
        Ok(page)
    }

    /// The paper given to new sheets: A4 landscape, centered on the origin and on the grid.
    pub fn new_sheet() -> Self {
        Self::centered(Paper::A4, Orientation::Landscape, Point { x_nm: 0, y_nm: 0 })
    }

    /// Center `paper` on a point, keeping the top-left corner on the schematic grid. The center
    /// is clamped so the result always lies inside the coordinate range.
    fn centered(paper: Paper, orientation: Orientation, center: Point) -> Self {
        let size = Self { paper, orientation, at: center }.size();
        let corner = |center: i64, extent: i64| {
            let snapped = (center - extent / 2).div_euclid(GRID_NM) * GRID_NM;
            // Grid multiples at both ends keep the clamped corner on the grid too.
            let low = (-LIMIT_NM).div_euclid(GRID_NM) * GRID_NM + GRID_NM;
            let high = (LIMIT_NM - extent).div_euclid(GRID_NM) * GRID_NM;
            snapped.clamp(low, high)
        };
        Self { paper, orientation, at: Point { x_nm: corner(center.x_nm, size.x_nm), y_nm: corner(center.y_nm, size.y_nm) } }
    }

    /// The smallest paper that holds `bounds` (a drawing's top-left and bottom-right corners)
    /// with a margin for the border, centered on the drawing. Landscape is preferred at each
    /// size. A drawing larger than A0 gets A0 landscape, centered; an empty sheet gets
    /// [`Page::new_sheet`].
    pub fn around(bounds: Option<[Point; 2]>) -> Self {
        let Some([min, max]) = bounds else { return Self::new_sheet() };
        let center = Point { x_nm: min.x_nm + (max.x_nm - min.x_nm) / 2, y_nm: min.y_nm + (max.y_nm - min.y_nm) / 2 };
        // Check the placed page: near the range's edge, centering is clamped and can shift the
        // paper off a drawing that its size alone would hold.
        Paper::ALL
            .into_iter()
            .flat_map(|paper| Orientation::ALL.map(|orientation| Self::centered(paper, orientation, center)))
            .find(|page| {
                let [low, high] = page.corners();
                low.x_nm <= min.x_nm - ADOPT_MARGIN_NM
                    && low.y_nm <= min.y_nm - ADOPT_MARGIN_NM
                    && high.x_nm >= max.x_nm + ADOPT_MARGIN_NM
                    && high.y_nm >= max.y_nm + ADOPT_MARGIN_NM
            })
            .unwrap_or_else(|| Self::centered(Paper::A0, Orientation::Landscape, center))
    }

    /// The ISO paper size.
    pub fn paper(self) -> Paper {
        self.paper
    }

    /// Landscape or portrait.
    pub fn orientation(self) -> Orientation {
        self.orientation
    }

    /// The paper's top-left corner in sheet coordinates.
    pub fn at(self) -> Point {
        self.at
    }

    /// Width and height in nanometres, after orientation.
    pub fn size(self) -> Point {
        let (long, short) = self.paper.sides_nm();
        match self.orientation {
            Orientation::Landscape => Point { x_nm: long, y_nm: short },
            Orientation::Portrait => Point { x_nm: short, y_nm: long },
        }
    }

    /// Top-left and bottom-right corners in sheet coordinates.
    pub fn corners(self) -> [Point; 2] {
        let size = self.size();
        [self.at, Point { x_nm: self.at.x_nm + size.x_nm, y_nm: self.at.y_nm + size.y_nm }]
    }

    /// Top-left and bottom-right corners of the plain border, [`BORDER_INSET_NM`] inside the
    /// paper's edge.
    pub fn border_corners(self) -> [Point; 2] {
        let [min, max] = self.corners();
        let inset = BORDER_INSET_NM;
        [Point { x_nm: min.x_nm + inset, y_nm: min.y_nm + inset }, Point { x_nm: max.x_nm - inset, y_nm: max.y_nm - inset }]
    }

    /// The JSON shape used by inspection and the sheet file.
    pub(crate) fn record(self) -> Record {
        Record { paper: self.paper, orientation: self.orientation, at_nm: [self.at.x_nm, self.at.y_nm] }
    }

    /// Inspection output, matching the stored record.
    pub(crate) fn inspect(self) -> Value {
        json!(self.record())
    }
}

/// The stored and inspected form of a [`Page`]; the field order is the file order.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    paper: Paper,
    orientation: Orientation,
    at_nm: [i64; 2],
}

impl TryFrom<Record> for Page {
    type Error = String;

    /// Validate decoded paper with the same range check as the command.
    fn try_from(record: Record) -> Result<Self, String> {
        Page::new(record.paper, record.orientation, Point { x_nm: record.at_nm[0], y_nm: record.at_nm[1] })
    }
}

impl SchematicSheet {
    /// This sheet's paper. `None` only on a sheet decoded from a format older than sheet/9 that
    /// has not been through [`adopt`] yet.
    pub fn page(&self) -> Option<Page> {
        self.page
    }
}

impl Project {
    /// The top-left and bottom-right corners of everything drawn on the current sheet: symbol
    /// pins and strokes after their pose, wire routes, net and hierarchical labels and sheet
    /// symbols. Text that extends past its anchor is not measured. `None` for an empty sheet.
    pub fn drawing_bounds(&self) -> Option<[Point; 2]> {
        let mut bounds: Option<[Point; 2]> = None;
        let mut add = |p: Point| {
            let [min, max] = bounds.get_or_insert([p, p]);
            *min = Point { x_nm: min.x_nm.min(p.x_nm), y_nm: min.y_nm.min(p.y_nm) };
            *max = Point { x_nm: max.x_nm.max(p.x_nm), y_nm: max.y_nm.max(p.y_nm) };
        };
        for symbol in self.symbols() {
            let (turns, mirrored, at) = (symbol.turns(), symbol.is_mirrored(), symbol.position);
            let mut local = |p: &Point| {
                let (x, y) = crate::schematic::place(p.x_nm, p.y_nm, turns, mirrored);
                add(Point { x_nm: at.x_nm + x, y_nm: at.y_nm + y });
            };
            for shape in symbol.unit_definition().graphics() {
                match &shape {
                    Shape::Graphic(graphic) => graphic.paths().iter().flatten().for_each(&mut local),
                    Shape::Line(a, b) | Shape::Rect(a, b) => [a, b].into_iter().for_each(&mut local),
                    Shape::Polyline(points) => points.iter().for_each(&mut local),
                    Shape::Circle(c, r) => {
                        [Point { x_nm: c.x_nm - r, y_nm: c.y_nm - r }, Point { x_nm: c.x_nm + r, y_nm: c.y_nm + r }].iter().for_each(&mut local)
                    }
                    Shape::Text(at, _, _) => local(at),
                }
            }
            symbol.pins().into_iter().for_each(&mut add);
        }
        for wire in self.wires() {
            self.wire_path(wire).into_iter().for_each(&mut add);
        }
        self.net_labels().iter().map(|label| label.at).chain(self.hierarchical_labels().iter().map(|label| label.at)).for_each(&mut add);
        for block in self.sheet_symbols() {
            let size = self.sheet_symbol_size(block);
            add(block.at);
            add(Point { x_nm: block.at.x_nm + size.x_nm, y_nm: block.at.y_nm + size.y_nm });
        }
        bounds
    }
}

/// Give every sheet without paper the smallest paper around its drawing (see [`Page::around`]).
/// Nothing moves and no history is recorded; the host keeps the original snapshot as its save
/// baseline, so the next save writes sheet/9. Returns the same `Arc` when every sheet already
/// has paper.
pub(crate) fn adopt(mut project: Arc<Project>) -> Arc<Project> {
    if project.sheets.iter().all(|sheet| sheet.page.is_some()) {
        return project;
    }
    let pages: Vec<_> = project
        .sheets
        .iter()
        .zip(project.sheets())
        .map(|(sheet, info)| sheet.page.unwrap_or_else(|| Page::around(project.for_sheet(info.target).and_then(|sheet| sheet.drawing_bounds()))))
        .collect();
    let candidate = Arc::make_mut(&mut project);
    for (sheet, page) in Arc::make_mut(&mut candidate.sheets).iter_mut().zip(pages) {
        sheet.page = Some(page);
    }
    project
}

impl Session {
    /// Change one sheet's paper as one undo step. Without `at` the top-left corner stays put, so
    /// the drawing keeps its place on the paper. Setting the current paper changes nothing.
    pub(crate) fn set_page(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let mut project = self.open_project()?.clone();
        let target = project.resolve_sheet(params["sheet"].as_str())?;
        let index =
            project.sheets().position(|sheet| sheet.target == target).ok_or_else(|| CommandError::InvalidParams("unknown sheet ID".into()))?;
        let paper = params["paper"].as_str().and_then(Paper::parse).ok_or_else(|| CommandError::InvalidParams("unknown paper size".into()))?;
        let orientation =
            params["orientation"].as_str().and_then(Orientation::parse).ok_or_else(|| CommandError::InvalidParams("unknown orientation".into()))?;
        let current = project.sheets[index].page.unwrap_or_else(Page::new_sheet);
        let at = match params.get("at") {
            Some(at) => serde_json::from_value::<Point>(at.clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?,
            None => current.at(),
        };
        let page = Page::new(paper, orientation, at).map_err(CommandError::InvalidParams)?;
        if project.sheets[index].page == Some(page) {
            return Ok(());
        }
        Arc::make_mut(&mut project.sheets)[index].page = Some(page);
        project.sheet_index = 0;
        self.change(project)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
