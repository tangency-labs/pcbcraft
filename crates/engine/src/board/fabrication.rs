//! Fabrication outputs: Gerber X2 layers and an Excellon drill file, written from one board
//! snapshot with deterministic content.
//!
//! Each copper layer, the top and bottom solder mask, paste and silkscreen, and the
//! Board Outline get one Gerber file in millimeters with nanometer resolution (format 4.6).
//! Plated holes (vias and through-hole pads) go in one Excellon file. Board Y grows downward on
//! screen, so every output negates Y, giving the usual Y-up fabrication view seen from the top.
//! Mask and paste openings follow the pad copper exactly; any expansion is left to the
//! fabricator's process. Vias are tented (no mask opening). Bottom-side footprints put their
//! pads on back copper, mask and paste and their silkscreen on the bottom legend. Arcs and
//! zones are not produced yet.
use super::{
    definition::{GraphicPurpose, PadShape},
    setup::{LayerPurpose, Side, Stratum},
    view::{BoardView, CopperLayers, PadView},
};
use crate::{CommandError, Session, schematic::Point};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    io::Write as _,
};

/// Generate the board's fabrication files; returns them without editing anything.
pub const EXPORT_FABRICATION: &str = "pcb.export_fabrication";
/// Silkscreen strokes without an authored width use 0.12 mm.
const DEFAULT_SILK_NM: i64 = 120_000;
/// The outline is drawn with a thin 0.1 mm line, the conventional profile width.
const OUTLINE_NM: i64 = 100_000;
/// Segments used to approximate each rounded pad corner.
const CORNER_SEGMENTS: usize = 6;

/// One generated file: a portable name and its text.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Output {
    pub name: String,
    pub contents: String,
}

/// Accumulates Gerber drawing operations and the circular apertures they use.
struct Gerber {
    apertures: BTreeMap<i64, usize>,
    body: String,
    current: Option<usize>,
}

impl Gerber {
    fn new() -> Self {
        Self { apertures: BTreeMap::new(), body: String::new(), current: None }
    }

    /// Select (defining on first use) the circular aperture of this diameter.
    fn aperture(&mut self, diameter_nm: i64) {
        let next = 10 + self.apertures.len();
        let code = *self.apertures.entry(diameter_nm.max(1)).or_insert(next);
        if self.current != Some(code) {
            let _ = writeln!(self.body, "D{code}*");
            self.current = Some(code);
        }
    }

    /// Stroke a polyline with a round aperture; a single point is flashed.
    fn stroke(&mut self, points: &[Point], width_nm: i64) {
        self.aperture(width_nm);
        let Some(first) = points.first() else { return };
        if points.len() == 1 {
            let _ = writeln!(self.body, "{}D03*", xy(*first));
            return;
        }
        let _ = writeln!(self.body, "{}D02*", xy(*first));
        for point in &points[1..] {
            let _ = writeln!(self.body, "{}D01*", xy(*point));
        }
    }

    /// Fill a closed polygon as a region.
    fn region(&mut self, polygon: &[Point]) {
        let Some(first) = polygon.first() else { return };
        self.body.push_str("G36*\n");
        let _ = writeln!(self.body, "{}D02*", xy(*first));
        for point in polygon[1..].iter().chain(std::iter::once(first)) {
            let _ = writeln!(self.body, "{}D01*", xy(*point));
        }
        self.body.push_str("G37*\n");
    }

    /// Draw one pad's copper exactly, by shape: a region for rectangles (rounded corners as
    /// short chords), a flash for circles and a round-ended stroke for ovals.
    fn pad(&mut self, pad: &PadView) {
        let Some(corners) = pad.copper else { return };
        let side = |a: Point, b: Point| (((b.x_nm - a.x_nm) as f64).hypot((b.y_nm - a.y_nm) as f64)).round() as i64;
        let (w, h) = (side(corners[0], corners[1]), side(corners[1], corners[2]));
        match pad.shape {
            PadShape::Rectangle => self.region(&corners),
            PadShape::RoundedRectangle => self.region(&rounded(&corners, pad.corner_radius_nm.min(w.min(h) / 2))),
            PadShape::Circle => self.stroke(&[pad.center], w.min(h)),
            PadShape::Oval => {
                // An obround is a stroke along the long axis with the short side as its width.
                let (long, short, along) = if w >= h { (w, h, (corners[0], corners[1])) } else { (h, w, (corners[1], corners[2])) };
                let length = side(along.0, along.1).max(1) as f64;
                let half = (long - short) as f64 / 2.;
                let (ux, uy) = ((along.1.x_nm - along.0.x_nm) as f64 / length, (along.1.y_nm - along.0.y_nm) as f64 / length);
                let end = |sign: f64| Point {
                    x_nm: pad.center.x_nm + (sign * half * ux).round() as i64,
                    y_nm: pad.center.y_nm + (sign * half * uy).round() as i64,
                };
                if half > 0. { self.stroke(&[end(-1.), end(1.)], short) } else { self.stroke(&[pad.center], short) }
            }
        }
    }

    /// Wrap the body in an X2 header and footer with the file's function and polarity.
    fn finish(self, function: &str, polarity: &str) -> String {
        let mut text = String::new();
        let _ = writeln!(text, "G04 PCBcraft fabrication output*");
        let _ = writeln!(text, "%TF.GenerationSoftware,PCBcraft,PCBcraft,{}*%", env!("CARGO_PKG_VERSION"));
        text.push_str("%TF.SameCoordinates,Original*%\n");
        let _ = writeln!(text, "%TF.FileFunction,{function}*%");
        let _ = writeln!(text, "%TF.FilePolarity,{polarity}*%");
        text.push_str("%FSLAX46Y46*%\n%MOMM*%\n%LPD*%\nG01*\n");
        for (diameter, code) in &self.apertures {
            let _ = writeln!(text, "%ADD{code}C,{}*%", mm(*diameter, 6));
        }
        text.push_str(&self.body);
        text.push_str("M02*\n");
        text
    }
}

/// A Gerber coordinate pair in integer nanometers (format 4.6 mm), with Y pointing up.
fn xy(point: Point) -> String {
    format!("X{}Y{}", point.x_nm, -point.y_nm)
}

/// Millimeters with a fixed number of decimals, from nanometers.
fn mm(nm: i64, decimals: usize) -> String {
    format!("{:.*}", decimals, nm as f64 / 1_000_000.)
}

/// A rectangle's outline with each corner replaced by a quarter arc of `radius`.
fn rounded(corners: &[Point; 4], radius: i64) -> Vec<Point> {
    if radius <= 0 {
        return corners.to_vec();
    }
    let r = radius as f64;
    let mut points = Vec::with_capacity(4 * (CORNER_SEGMENTS + 1));
    for i in 0..4 {
        let (corner, previous, next) = (corners[i], corners[(i + 3) % 4], corners[(i + 1) % 4]);
        let unit = |to: Point| {
            let (dx, dy) = ((to.x_nm - corner.x_nm) as f64, (to.y_nm - corner.y_nm) as f64);
            let length = dx.hypot(dy).max(1.);
            (dx / length, dy / length)
        };
        let (u, v) = (unit(previous), unit(next));
        let center = (corner.x_nm as f64 + (u.0 + v.0) * r, corner.y_nm as f64 + (u.1 + v.1) * r);
        for step in 0..=CORNER_SEGMENTS {
            let t = std::f64::consts::FRAC_PI_2 * step as f64 / CORNER_SEGMENTS as f64;
            let (c, s) = (t.cos(), t.sin());
            points
                .push(Point { x_nm: (center.0 - r * (v.0 * c + u.0 * s)).round() as i64, y_nm: (center.1 - r * (v.1 * c + u.1 * s)).round() as i64 });
        }
    }
    points
}

/// Add explicit board artwork to its manufacturing surface. Documentation-only layers
/// stay in the native board; copper is already represented by validated conductive tracks.
fn board_graphics(gerber: &mut Gerber, view: BoardView<'_>, purpose: LayerPurpose, side: Option<Side>) {
    let layers = view.layers();
    for drawing in view
        .drawings()
        .into_iter()
        .filter(|drawing| layers.iter().any(|layer| layer.id == drawing.layer && layer.purpose == purpose && layer.side == side))
    {
        let mut points = drawing.points;
        if drawing.closed
            && let Some(first) = points.first().copied()
        {
            points.push(first);
        }
        gerber.stroke(&points, drawing.stroke_nm.unwrap_or(OUTLINE_NM));
    }
}

/// Keep file names portable: letters, digits, `-` and `_` only.
fn file_part(text: &str) -> String {
    let cleaned: String = text.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    if cleaned.trim_matches('_').is_empty() { "board".into() } else { cleaned }
}

/// Allocate portable, case-insensitively unique names without changing file order or contents.
/// Fixed-purpose files take precedence over the first `copper_count` layer-named files;
/// collisions receive deterministic numeric suffixes after sanitization.
fn unique_file_names(files: &mut [Output], copper_count: usize) {
    let mut used = BTreeSet::new();
    for index in (copper_count..files.len()).chain(0..copper_count) {
        let original = &files[index].name;
        let (stem, extension) = original.rsplit_once('.').unwrap_or((original, ""));
        let mut name = original.clone();
        let mut suffix = 2;
        while !used.insert(name.to_ascii_lowercase()) {
            name = format!("{stem}-{suffix}.{extension}");
            suffix += 1;
        }
        files[index].name = name;
    }
}

impl BoardView<'_> {
    /// Build every fabrication file for this board, in a fixed order. Requires a board setup
    /// with copper layers and a closed Board Outline; returns why when either is missing.
    /// Sanitized layer names are disambiguated against all outputs, preserving fixed file names.
    pub fn fabrication_outputs(self) -> Result<Vec<Output>, String> {
        let setup = self.setup().ok_or("Initialize the board setup before generating fabrication outputs.")?;
        let copper: Vec<_> = setup.layers().iter().filter(|layer| layer.purpose == LayerPurpose::Copper).collect();
        // Copper layers in physical stack order, top first.
        let order: Vec<uuid::Uuid> = setup
            .stackup()
            .strata
            .iter()
            .filter_map(|stratum| if let Stratum::Copper { layer, .. } = stratum { Some(*layer) } else { None })
            .collect();
        let layers = self.layers();
        let outline: Vec<_> = self
            .drawings()
            .into_iter()
            .filter(|drawing| layers.iter().any(|layer| layer.id == drawing.layer && layer.purpose == LayerPurpose::Outline))
            .collect();
        if !outline.iter().any(|drawing| drawing.closed) {
            return Err("Draw a closed Board Outline before generating fabrication outputs.".into());
        }
        let base = file_part(&self.project().name);
        let pads: Vec<PadView> = self.footprints().flat_map(|footprint| footprint.pads).filter(|pad| pad.copper.is_some()).collect();
        let on =
            |pad: &PadView, layer: uuid::Uuid| matches!(&pad.copper_layers, Some(CopperLayers::Resolved { layers, .. }) if layers.contains(&layer));
        let mut files = Vec::new();

        for (index, layer_id) in order.iter().enumerate() {
            let Some(layer) = copper.iter().find(|layer| layer.id == *layer_id) else { continue };
            let mut gerber = Gerber::new();
            for pad in pads.iter().filter(|pad| on(pad, layer.id)) {
                gerber.pad(pad);
            }
            let mut tracks: Vec<_> = self.tracks().iter().filter(|track| track.layer == layer.id).collect();
            tracks.sort_by_key(|track| (track.width_nm, track.id));
            for track in tracks {
                gerber.stroke(&[track.start, track.end], track.width_nm);
            }
            for via in self.vias() {
                gerber.stroke(&[via.at], via.diameter_nm);
            }
            let position = match (index, layer.side) {
                (_, Some(Side::Front)) => "Top",
                (_, Some(Side::Back)) => "Bot",
                _ => "Inr",
            };
            files.push(Output {
                name: format!("{base}-{}.gbr", file_part(&layer.name)),
                contents: gerber.finish(&format!("Copper,L{},{position}", index + 1), "Positive"),
            });
        }

        let copper_count = files.len();
        let front = order.first().copied();
        let back = order.last().copied().filter(|_| order.len() > 1);
        for (side, label, layer) in [("Top", "Mask_Top", front), ("Bot", "Mask_Bottom", back)] {
            let mut gerber = Gerber::new();
            for pad in pads.iter().filter(|pad| layer.is_some_and(|layer| on(pad, layer))) {
                gerber.pad(pad);
            }
            board_graphics(&mut gerber, self, LayerPurpose::Mask, Some(if side == "Top" { Side::Front } else { Side::Back }));
            files.push(Output { name: format!("{base}-{label}.gbr"), contents: gerber.finish(&format!("Soldermask,{side}"), "Negative") });
        }
        for (side, label, layer) in [("Top", "Paste_Top", front), ("Bot", "Paste_Bottom", back)] {
            let mut gerber = Gerber::new();
            for pad in pads.iter().filter(|pad| !pad.through_hole && layer.is_some_and(|layer| on(pad, layer))) {
                gerber.pad(pad);
            }
            board_graphics(&mut gerber, self, LayerPurpose::Paste, Some(if side == "Top" { Side::Front } else { Side::Back }));
            files.push(Output { name: format!("{base}-{label}.gbr"), contents: gerber.finish(&format!("Paste,{side}"), "Positive") });
        }

        for (side, position, label) in [(Side::Front, "Top", "Silkscreen_Top"), (Side::Back, "Bot", "Silkscreen_Bottom")] {
            let mut silk = Gerber::new();
            for footprint in self.footprints() {
                for graphic in footprint.graphics.iter().filter(|graphic| graphic.purpose == GraphicPurpose::Silkscreen && graphic.side == side) {
                    let mut points = graphic.points.clone();
                    if graphic.closed && points.len() > 2 {
                        points.push(points[0]);
                    }
                    silk.stroke(&points, graphic.stroke_nm.unwrap_or(DEFAULT_SILK_NM));
                }
            }
            board_graphics(&mut silk, self, LayerPurpose::Silkscreen, Some(side));
            files.push(Output { name: format!("{base}-{label}.gbr"), contents: silk.finish(&format!("Legend,{position}"), "Positive") });
        }

        let mut profile = Gerber::new();
        for drawing in &outline {
            let mut points = drawing.points.clone();
            if drawing.closed {
                points.push(points[0]);
            }
            profile.stroke(&points, drawing.stroke_nm.unwrap_or(OUTLINE_NM));
        }
        files.push(Output { name: format!("{base}-Outline.gbr"), contents: profile.finish("Profile,NP", "Positive") });

        files.push(Output { name: format!("{base}-PTH.drl"), contents: self.drill_file() });
        unique_file_names(&mut files, copper_count);
        Ok(files)
    }

    /// One Excellon file for every plated hole: vias and through-hole pads, grouped by tool.
    fn drill_file(self) -> String {
        let mut holes: BTreeMap<i64, Vec<Point>> = BTreeMap::new();
        for via in self.vias() {
            holes.entry(via.drill_nm).or_default().push(via.at);
        }
        for pad in self.footprints().flat_map(|footprint| footprint.pads) {
            if let Some(drill) = pad.drill_nm {
                holes.entry(drill).or_default().push(pad.center);
            }
        }
        let mut text = String::from("M48\n; PCBcraft plated drill file\n; TYPE=PLATED\nFMAT,2\nMETRIC\n");
        for (index, diameter) in holes.keys().enumerate() {
            let _ = writeln!(text, "T{}C{}", index + 1, mm(*diameter, 3));
        }
        text.push_str("%\nG90\nG05\n");
        for (index, points) in holes.values_mut().enumerate() {
            points.sort_by_key(|point| (point.x_nm, point.y_nm));
            let _ = writeln!(text, "T{}", index + 1);
            for point in points.iter() {
                let _ = writeln!(text, "X{}Y{}", mm(point.x_nm, 3), mm(-point.y_nm, 3));
            }
        }
        text.push_str("M30\n");
        text
    }
}

/// Pack outputs with fixed timestamps and Unix regular-file metadata on every platform.
/// The same outputs give the same bytes; ZIP encoding failures are returned as errors.
pub fn zip(outputs: &[Output]) -> Result<Vec<u8>, String> {
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};
    let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .system(zip::System::Unix)
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    for output in outputs {
        writer.start_file(&output.name, options).map_err(|e| e.to_string())?;
        writer.write_all(output.contents.as_bytes()).map_err(|e| e.to_string())?;
    }
    writer.finish().map(|cursor| cursor.into_inner()).map_err(|e| e.to_string())
}

impl Session {
    /// Generate the addressed board's fabrication files without editing anything.
    pub fn export_fabrication(&self, params: &Value) -> Result<Vec<Output>, CommandError> {
        let project = self.open_project()?;
        let target = project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let view = project.board_view(target).ok_or_else(|| CommandError::Unavailable("board unavailable".into()))?;
        view.fabrication_outputs().map_err(CommandError::Unavailable)
    }

    /// The execute result for MCP: every file's name and text.
    pub(crate) fn fabrication_report(&self, params: &Value) -> Result<Value, CommandError> {
        Ok(json!({"files": self.export_fabrication(params)?}))
    }
}

#[cfg(test)]
mod tests;
