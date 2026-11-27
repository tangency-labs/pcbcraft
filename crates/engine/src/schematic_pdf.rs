//! Read-only vector export of an addressed sheet or a project hierarchy.

mod navigation;
mod occurrences;
mod pdf;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use crate::{
    Project, SheetTarget,
    library_graphics::{Fill, Geometry, Graphic},
    schematic::{GRID_NM, Point, Shape, SymbolPlacement, TextAlign, WireEndpoint, fields::Layout, place},
    sheets::{page::Page, template::Align},
};

const PT_PER_NM: f64 = 72. / 25_400_000.;
const STROKE_NM: i64 = 180_000;
const LABEL_NM: i64 = GRID_NM;
/// Vertical clearance between a turned body's centre line and its automatic Reference or Value.
const FIELD_GAP_NM: i64 = GRID_NM / 10;

/// A complete schematic PDF could not be produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PdfError {
    UnknownSheet,
    InvalidHierarchy,
    MissingPaper,
    FontUnavailable,
    DocumentTooLarge,
}

impl std::fmt::Display for PdfError {
    /// Report which input or encoding prerequisite prevented a complete document.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UnknownSheet => "The sheet address does not belong to this project.",
            Self::InvalidHierarchy => "The schematic occurrences do not form a valid hierarchy.",
            Self::MissingPaper => "The sheet has no paper; open it with paper adoption before exporting.",
            Self::FontUnavailable => "The bundled regular stroke face could not be loaded.",
            Self::DocumentTooLarge => "The document exceeds the PDF cross-reference offset range.",
        })
    }
}

impl std::error::Error for PdfError {}

/// Encode exactly `target` from an immutable project, independently of the active editor sheet.
/// The sheet's paper defines the PDF page and its clip. The stored frame, labels, wires and
/// symbols are drawn as vectors. Text uses the bundled regular stroke face and a Unicode map.
/// This function never writes files, changes project state, or traverses pages for export.
pub fn export(project: &Project, target: SheetTarget) -> Result<Vec<u8>, PdfError> {
    let sheet = project.for_sheet(target).ok_or(PdfError::UnknownSheet)?;
    draw_page(&sheet, None, None)?.finish()
}

/// Export every project sheet occurrence, with a bookmark per page and nested child bookmarks.
/// Visit roots in project sheet order, then children by ascending parent sheet-symbol ID,
/// depth first. The same order numbers the pages in the title block. Separate occurrences of
/// a definition always get separate pages. Sheet symbols link to their children and global
/// labels and ports link to the next page on their net. The active editor sheet has no effect.
/// If any page cannot be drawn, return no document.
pub fn export_project(project: &Project) -> Result<Vec<u8>, PdfError> {
    export_occurrences(project, &project.occurrence_instances().sheet_occurrences)
}

/// Draw pages from the occurrence projection rather than from a set of unique definitions.
fn export_occurrences(project: &Project, records: &[crate::instances::SheetOccurrence]) -> Result<Vec<u8>, PdfError> {
    let ordered = occurrences::ordered(project, records)?;
    let sheets = ordered.iter().map(|visit| project.for_sheet(visit.sheet.target).ok_or(PdfError::UnknownSheet)).collect::<Result<Vec<_>, _>>()?;
    let links = navigation::Navigation::new(project, &ordered, &sheets);
    let mut document = pdf::Document::new();
    let pages = ordered.len();
    let mut bookmarks = Vec::with_capacity(pages);
    for (index, (visit, sheet)) in ordered.into_iter().zip(&sheets).enumerate() {
        document.add_page(draw_page(sheet, Some((index + 1, pages)), Some((&links, index)))?);
        bookmarks.push(pdf::Bookmark { title: visit.sheet.name, parent: visit.parent.map(|(parent, _)| parent) });
    }
    document.finish(&bookmarks)
}

/// Render the saved paper, frame and geometry. Project export supplies its occurrence's
/// one-based page number and page count, and resolved link targets; addressed export keeps
/// the editor frame values and draws no links.
fn draw_page(sheet: &Project, output_page: Option<(usize, usize)>, nav: Option<(&navigation::Navigation, usize)>) -> Result<pdf::Page, PdfError> {
    let paper = sheet.page().ok_or(PdfError::MissingPaper)?;
    let frame = match output_page {
        Some((page, pages)) => paper.frame(&|text| sheet.resolve_text_for_output_page(sheet.sheet_target(), text, page, pages)),
        None => sheet.frame().ok_or(PdfError::MissingPaper)?,
    };
    let mut page = Drawing::new(paper);
    for line in frame.lines {
        page.path(line, STROKE_NM, false, Fill::None);
    }
    for text in frame.texts {
        let align = match text.align {
            Align::Left => TextAlign::Start,
            Align::Center => TextAlign::Middle,
            Align::Right => TextAlign::End,
        };
        page.text(text.at, text.size_nm, 0, align, &text.text)?;
    }
    draw_wires(sheet, &mut page);
    for symbol in sheet.symbols() {
        draw_symbol(sheet, &mut page, symbol)?;
    }
    draw_labels(sheet, &mut page, nav)?;
    Ok(page.pdf)
}

struct Drawing {
    pdf: pdf::Page,
    left: i64,
    bottom: i64,
}

impl Drawing {
    /// Use the saved paper's dimensions and bottom-left PDF origin, not drawing bounds or zoom.
    fn new(paper: Page) -> Self {
        let [top_left, bottom_right] = paper.corners();
        let size = paper.size();
        Self { pdf: pdf::Page::new([pt(size.x_nm), pt(size.y_nm)]), left: top_left.x_nm, bottom: bottom_right.y_nm }
    }

    /// Project absolute sheet coordinates (Y down) into PDF coordinates (Y up).
    fn point(&self, at: Point) -> [f64; 2] {
        [pt(at.x_nm - self.left), pt(self.bottom - at.y_nm)]
    }

    /// Attach an invisible jump to a physical region of the current sheet.
    fn link(&mut self, a: Point, b: Point, destination: usize) {
        self.pdf.link(self.point(a), self.point(b), destination);
    }

    /// Write one connected vector path in physical units, with a minimum print stroke.
    fn path(&mut self, points: impl IntoIterator<Item = Point>, width_nm: i64, closed: bool, fill: Fill) {
        let (left, bottom) = (self.left, self.bottom);
        self.pdf.path(points.into_iter().map(|p| [pt(p.x_nm - left), pt(bottom - p.y_nm)]), pt(width_nm.max(100_000)), closed, fill);
    }

    /// Anchor regular text at its mid height; angles are counter-clockwise on the finished page.
    fn text(&mut self, at: Point, height: i64, degrees: u16, align: TextAlign, text: &str) -> Result<(), PdfError> {
        self.pdf.text(self.point(at), pt(height), f64::from(degrees).to_radians(), align, text)
    }

    /// Render a vector circle with the same bounded polyline projection as authored graphics.
    fn circle(&mut self, center: Point, radius_nm: i64, fill: Fill) {
        let graphic = Graphic { geometry: Geometry::Circle { center, radius_nm }, stroke_nm: STROKE_NM, fill };
        for line in graphic.paths() {
            self.path(line, graphic.stroke_nm, true, fill);
        }
    }

    /// Render the saved no-connect intent, not transient dangling/selection handles.
    fn no_connect(&mut self, at: Point) {
        let d = GRID_NM / 4;
        for dy in [-d, d] {
            self.path([add(at, -d, dy), add(at, d, -dy)], STROKE_NM, false, Fill::None);
        }
    }
}

/// Draw sheet-local paths from their existing endpoint identities and authored bends. Index
/// anchors once: repeated wire endpoint lookup must not rescan every symbol or sheet interface.
fn draw_wires(sheet: &Project, page: &mut Drawing) {
    let ends: BTreeMap<_, _> = sheet
        .hierarchy_endpoints()
        .chain(sheet.junctions().map(|(id, at)| (WireEndpoint::Junction(id), at)))
        .chain(sheet.symbols().iter().flat_map(|s| s.terminals().into_iter().map(WireEndpoint::Terminal).zip(s.pins())))
        .collect();
    let mut degrees = BTreeMap::<u64, usize>::new();
    for wire in sheet.wires() {
        let endpoints = wire.endpoints();
        if let [Some(&a), Some(&b)] = endpoints.map(|end| ends.get(&end)) {
            page.path(crate::wire::wire_path(wire, a, b), STROKE_NM, false, Fill::None);
            for end in endpoints {
                if let WireEndpoint::Junction(id) = end {
                    *degrees.entry(id).or_default() += 1;
                }
            }
        }
    }
    for (id, at) in sheet.junctions() {
        if sheet.no_connect_ends.contains(&id) {
            page.no_connect(at);
        } else if degrees.get(&id).is_some_and(|degree| *degree >= 3) {
            page.circle(at, GRID_NM / 5, Fill::Solid);
        }
    }
}

/// Draw the selected unit of a placed symbol in its authored pose, then its instance fields.
fn draw_symbol(sheet: &Project, page: &mut Drawing, symbol: &SymbolPlacement) -> Result<(), PdfError> {
    let mut bounds = [symbol.position.y_nm; 2];
    for shape in sheet.symbol_graphics(symbol) {
        draw_shape(page, symbol, &shape, &mut bounds)?;
    }
    let Some(part) = sheet.component(symbol) else { return Ok(()) };
    let reference = symbol.display_reference(part.reference());
    let value = part.display_value();
    let fields = [("reference", "Reference", reference.as_str()), ("value", "Value", value.as_ref())];
    for (i, (id, name, value)) in
        fields.into_iter().chain(part.metadata.fields.iter().map(|(id, f)| (id.as_str(), f.name.as_str(), f.value.as_str()))).enumerate()
    {
        let default = Layout { visible: i < 2, ..Layout::default() };
        let layout = symbol.field_layout.get(id).unwrap_or(&default);
        if !layout.visible || value.is_empty() {
            continue;
        }
        let (anchor, align) = match layout.offset {
            Some(offset) => (posed(symbol, offset), layout.align),
            // As on the canvas, a quarter-turned body keeps Reference and Value beside it,
            // left-aligned, so they do not sit on the vertical pin axis where wires continue.
            None if i < 2 && symbol.turns() % 2 == 1 => {
                let [low, high] = body_bounds(symbol, &symbol.unit_definition().drawing);
                let middle = (low.y_nm + high.y_nm) / 2;
                let rise = FIELD_GAP_NM + layout.height_nm / 2;
                let y_nm = if i == 0 { middle - rise } else { middle + rise };
                (Point { x_nm: high.x_nm + 2 * GRID_NM / 5, y_nm }, TextAlign::Start)
            }
            None => (
                Point {
                    x_nm: symbol.position.x_nm,
                    y_nm: if i == 0 { bounds[0] - layout.height_nm } else { bounds[1] + i as i64 * layout.height_nm * 2 },
                },
                layout.align,
            ),
        };
        let shown = if layout.show_name { format!("{name}: {value}") } else { value.to_owned() };
        page.text(anchor, layout.height_nm, layout.rotation, align, &shown)?;
    }
    for (terminal, at) in symbol.terminals().into_iter().zip(symbol.pins()) {
        if part.no_connects.contains_key(terminal.number()) {
            page.no_connect(at);
        }
    }
    Ok(())
}

/// Bound the posed body drawing without generated pin ink, as the canvas does for automatic
/// field placement. An empty or zero-area drawing uses a two-grid square around the symbol center.
fn body_bounds(symbol: &SymbolPlacement, drawing: &[Shape]) -> [Point; 2] {
    let mut points = Vec::new();
    for shape in drawing {
        match shape {
            Shape::Line(a, b) | Shape::Rect(a, b) => points.extend([*a, *b]),
            Shape::Polyline(line) => points.extend_from_slice(line),
            Shape::Circle(center, radius) => points.extend([add(*center, -radius, -radius), add(*center, *radius, *radius)]),
            Shape::Text(at, _, _) => points.push(*at),
            Shape::Graphic(graphic) => points.extend(graphic.paths().into_iter().flatten()),
        }
    }
    let mut points = points.into_iter().map(|p| posed(symbol, p));
    if let Some(first) = points.next() {
        let [low, high] = points.fold([first; 2], |[low, high], p| {
            [Point { x_nm: low.x_nm.min(p.x_nm), y_nm: low.y_nm.min(p.y_nm) }, Point { x_nm: high.x_nm.max(p.x_nm), y_nm: high.y_nm.max(p.y_nm) }]
        });
        if low.x_nm < high.x_nm && low.y_nm < high.y_nm {
            return [low, high];
        }
    }
    [add(symbol.position, -GRID_NM, -GRID_NM), add(symbol.position, GRID_NM, GRID_NM)]
}

/// Reuse physical graphics polylines, but keep all lettering as PDF text rather than outlining it.
fn draw_shape(page: &mut Drawing, symbol: &SymbolPlacement, shape: &Shape, bounds: &mut [i64; 2]) -> Result<(), PdfError> {
    let mut path = |local: &[Point], width_nm, closed, fill| {
        let path = local.iter().map(|&p| posed(symbol, p)).inspect(|p| {
            bounds[0] = bounds[0].min(p.y_nm);
            bounds[1] = bounds[1].max(p.y_nm);
        });
        page.path(path, width_nm, closed, fill);
    };
    match shape {
        Shape::Line(a, b) => path(&[*a, *b], STROKE_NM, false, Fill::None),
        Shape::Polyline(points) => path(points, STROKE_NM, false, Fill::None),
        Shape::Rect(a, b) => path(&rectangle(*a, *b), STROKE_NM, true, Fill::None),
        Shape::Circle(center, radius) => {
            let at = posed(symbol, *center);
            bounds[0] = bounds[0].min(at.y_nm - radius);
            bounds[1] = bounds[1].max(at.y_nm + radius);
            page.circle(at, *radius, Fill::None);
        }
        Shape::Text(at, text, align) => {
            let align = match (symbol.turns() % 4, symbol.is_mirrored()) {
                (1 | 3, _) => TextAlign::Middle,
                (0, true) | (2, false) => reverse_align(*align),
                _ => *align,
            };
            page.text(posed(symbol, *at), LABEL_NM, 0, align, text)?;
        }
        Shape::Graphic(graphic) => {
            if let Geometry::Text { at, text, height_nm, rotation, align } = &graphic.geometry {
                let local = if symbol.is_mirrored() { 180 - i32::from(*rotation) } else { i32::from(*rotation) };
                let (degrees, align) = upright(local - 90 * i32::from(symbol.turns()), *align);
                page.text(posed(symbol, *at), *height_nm, (360 - degrees) % 360, align, text)?;
            } else {
                for line in graphic.paths() {
                    path(&line, graphic.stroke_nm, graphic.closed(), graphic.fill);
                }
            }
        }
    }
    Ok(())
}

/// Draw saved net names and hierarchy interfaces on this sheet only, including turned name
/// offsets and directional child-side markers at their wire contacts. In project exports,
/// attach only resolved destinations; hidden global label text does not remove the drawn
/// linkable symbol.
fn draw_labels(sheet: &Project, page: &mut Drawing, nav: Option<(&navigation::Navigation, usize)>) -> Result<(), PdfError> {
    for label in sheet.net_labels() {
        let global = label.style.global();
        let default = Layout {
            align: if global {
                TextAlign::Middle
            } else if label.rotation == 180 {
                TextAlign::End
            } else {
                TextAlign::Start
            },
            ..Layout::default()
        };
        let layout = label.text.as_ref().unwrap_or(&default);
        let pose = |x, y| {
            let (dx, dy) = place(x, y, (label.rotation / 90) as u8, false);
            add(label.at, dx, dy)
        };
        let shown = if layout.show_name { format!("Net name: {}", label.name) } else { label.name.clone() };
        let at = if global {
            let h = layout.height_nm.max(LABEL_NM);
            let width = (page.pdf.text_width(pt(h), &shown)? / PT_PER_NM).ceil() as i64 + 2 * h;
            page.path([pose(0, 0), pose(h, -h), pose(width, -h), pose(width, h), pose(h, h)], STROKE_NM, true, Fill::None);
            if let Some(destination) = nav.and_then(|(nav, _)| nav.net(WireEndpoint::NetLabel(label.id))) {
                page.link(pose(0, -h), pose(width, h), destination);
            }
            pose((h + width) / 2, 0)
        } else {
            pose(LABEL_NM / 3, -layout.height_nm)
        };
        if layout.visible {
            let at = layout.offset.map_or(at, |offset| pose(offset.x_nm, offset.y_nm));
            let (degrees, align) = if label.text.is_some() {
                (layout.rotation, layout.align)
            } else {
                let rotation = if global { i32::from(label.rotation) } else { 0 };
                upright(rotation, layout.align)
            };
            page.text(at, layout.height_nm, degrees, align, &shown)?;
        }
    }
    for label in sheet.hierarchical_labels() {
        let (dx, align) = if label.direction.right() { (GRID_NM / 2, TextAlign::Start) } else { (-GRID_NM / 2, TextAlign::End) };
        let wing = GRID_NM / 4;
        page.path([add(label.at, dx, -wing), label.at, add(label.at, dx, wing)], STROKE_NM, false, Fill::None);
        page.text(add(label.at, dx + dx / 2, 0), LABEL_NM, 0, align, &label.name)?;
        if let Some(destination) = nav.and_then(|(nav, _)| nav.net(WireEndpoint::Label(label.id))) {
            // Cover the marker and the name on whichever side the direction puts it.
            let width = (page.pdf.text_width(pt(LABEL_NM), &label.name)? / PT_PER_NM).ceil() as i64;
            let end = dx + dx / 2 + dx.signum() * width;
            page.link(add(label.at, 0, -LABEL_NM), add(label.at, end, LABEL_NM), destination);
        }
    }
    for block in sheet.sheet_symbols() {
        let size = sheet.sheet_symbol_size(block);
        let extent = add(block.at, size.x_nm, size.y_nm);
        page.path(rectangle(block.at, extent), STROKE_NM, true, Fill::None);
        if let Some(destination) = nav.and_then(|(nav, index)| nav.child(index, block.id)) {
            page.link(block.at, extent, destination);
        }
        page.text(add(block.at, 0, -LABEL_NM), LABEL_NM, 0, TextAlign::Start, sheet.child_name(block))?;
        for pin in sheet.sheet_pins(block) {
            let (dx, align) = if pin.label.direction.right() { (-GRID_NM, TextAlign::End) } else { (GRID_NM, TextAlign::Start) };
            page.path([pin.at, add(pin.at, dx, 0)], STROKE_NM, false, Fill::None);
            page.text(add(pin.at, dx, 0), LABEL_NM, 0, align, &pin.label.name)?;
        }
    }
    Ok(())
}

/// Keep text upright while preserving which side of its authored anchor it occupies.
fn upright(degrees: i32, align: TextAlign) -> (u16, TextAlign) {
    let angle = degrees.rem_euclid(360) as u16;
    if (91..=270).contains(&angle) { ((angle + 180) % 360, reverse_align(align)) } else { (angle, align) }
}

/// Swap the leading and trailing edges without moving a centered anchor.
fn reverse_align(align: TextAlign) -> TextAlign {
    match align {
        TextAlign::Start => TextAlign::End,
        TextAlign::End => TextAlign::Start,
        TextAlign::Middle => TextAlign::Middle,
    }
}

/// Place a library point with the same reflection and quarter-turn as its terminals.
fn posed(symbol: &SymbolPlacement, at: Point) -> Point {
    let (x, y) = place(at.x_nm, at.y_nm, symbol.turns(), symbol.is_mirrored());
    add(symbol.position, x, y)
}

/// Four corners in perimeter order, from two opposite corners.
fn rectangle(a: Point, b: Point) -> [Point; 4] {
    [a, Point { x_nm: b.x_nm, y_nm: a.y_nm }, b, Point { x_nm: a.x_nm, y_nm: b.y_nm }]
}

/// Apply bounded saved geometry offsets in schematic space.
fn add(at: Point, x: i64, y: i64) -> Point {
    Point { x_nm: at.x_nm + x, y_nm: at.y_nm + y }
}

/// Convert nanometres to PDF points (72 points in the exact 25.4 mm inch).
fn pt(nm: i64) -> f64 {
    nm as f64 * PT_PER_NM
}
