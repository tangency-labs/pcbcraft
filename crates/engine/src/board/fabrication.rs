//! Fabrication outputs: Gerber X2 layers and an Excellon drill file, written from one board
//! snapshot with deterministic content.
//!
//! Each copper layer, the top and bottom solder mask, paste and silkscreen, and the
//! Board Outline get one Gerber file in millimeters with nanometer resolution (format 4.6).
//! Plated holes (vias and through-hole pads) go in one Excellon file. Every coordinate is written
//! in the board's output frame: measured from its placement origin with Y up, seen from the top.
//! Openings use the pad exporter without process expansion: mask includes pads on the outer
//! copper layer, while paste excludes through-hole pads. Obround strokes and chorded corners
//! retain their existing approximations. Vias receive no automatic mask opening; authored mask
//! artwork can still open them. Mounting-side pads follow the placement side; silkscreen follows
//! the top or bottom legend. Arcs and zones are not produced yet.
use super::{
    definition::{GraphicPurpose, PadShape},
    origin::{Frame, Resolved},
    setup::{LayerPurpose, Side, Stratum},
    view::{BoardView, CopperLayers, PadView},
};
use crate::{CommandError, Session, schematic::Point};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Output {
    pub name: String,
    pub contents: String,
}

/// Every fabrication file of one board and the origin their coordinates are measured from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Fabrication {
    /// The origin every coordinate in these files is measured from.
    pub origin: Resolved,
    /// The files in a fixed order with unique portable names.
    pub files: Vec<Output>,
}

/// Integrity of the exact bytes of one generated artifact, not a design-state identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Artifact {
    pub name: String,
    pub size_bytes: u64,
    pub sha256: String,
}

impl Artifact {
    /// Describe bytes without normalizing their encoding, whitespace or line endings.
    fn new(name: String, bytes: &[u8]) -> Self {
        Self { name, size_bytes: bytes.len() as u64, sha256: format!("{:x}", Sha256::digest(bytes)) }
    }
}

/// Integrity records for the generated files, not the ZIP itself or the design state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Manifest {
    pub generator_version: String,
    pub files: Vec<Artifact>,
}

impl Manifest {
    /// Describe the exact uncompressed bytes that are passed to the ZIP writer.
    fn from_outputs(outputs: &[Output]) -> Self {
        Self {
            generator_version: env!("CARGO_PKG_VERSION").into(),
            files: outputs.iter().map(|output| Artifact::new(output.name.clone(), output.contents.as_bytes())).collect(),
        }
    }
}

/// Immutable ZIP bytes, including the manifest, built from captured outputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Archive {
    bytes: Vec<u8>,
}

impl Archive {
    /// Pack the captured outputs and their manifest without editing the project or writing files.
    pub fn new(outputs: &[Output]) -> Result<Self, String> {
        Ok(Self { bytes: zip(outputs)? })
    }

    /// The complete deterministic ZIP, independent of its destination filename.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// The suggested archive name, using the same portable project-name rules as its members.
pub fn archive_name(project: &str) -> String {
    format!("{}-fabrication.zip", file_part(project))
}

/// Standard apertures retain shape as well as size when sharing D-codes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Aperture {
    Circle(i64),
    Rectangle(i64, i64),
    Obround(i64, i64),
}

/// Accumulates Gerber drawing operations and the standard apertures they use.
struct Gerber {
    apertures: BTreeMap<Aperture, usize>,
    body: String,
    current: Option<usize>,
    frame: Frame,
}

impl Gerber {
    fn new(frame: Frame) -> Self {
        Self { apertures: BTreeMap::new(), body: String::new(), current: None, frame }
    }

    /// Select an aperture, assigning its D-code on first use.
    fn aperture(&mut self, aperture: Aperture) {
        let next = 10 + self.apertures.len();
        let code = *self.apertures.entry(aperture).or_insert(next);
        if self.current != Some(code) {
            let _ = writeln!(self.body, "D{code}*");
            self.current = Some(code);
        }
    }

    /// Stroke a polyline with a round aperture; a single point is flashed.
    fn stroke(&mut self, points: &[Point], width_nm: i64) {
        self.aperture(Aperture::Circle(width_nm.max(1)));
        let Some(first) = points.first() else { return };
        if points.len() == 1 {
            let _ = writeln!(self.body, "{}D03*", xy(self.frame, *first));
            return;
        }
        let _ = writeln!(self.body, "{}D02*", xy(self.frame, *first));
        for point in &points[1..] {
            let _ = writeln!(self.body, "{}D01*", xy(self.frame, *point));
        }
    }

    /// Emit a closed region path. A collapsed pad can produce a retraced, zero-area path.
    fn region(&mut self, polygon: &[Point]) {
        let Some(first) = polygon.first() else { return };
        self.body.push_str("G36*\n");
        let _ = writeln!(self.body, "{}D02*", xy(self.frame, *first));
        for point in polygon[1..].iter().chain(std::iter::once(first)) {
            let _ = writeln!(self.body, "{}D01*", xy(self.frame, *point));
        }
        self.body.push_str("G37*\n");
    }

    /// Flash R/O only for individually cardinal library and placement rotations and a resolved
    /// rectangle. R also needs an integral midpoint; O needs integral capsule endpoints to match
    /// the engine's land contour. Other R lands remain regions, even if tiny corners collapse.
    /// Other O lands retain the approximate round-ended stroke (a circle for equal sides).
    /// Rounded rectangles use regions with chorded corners only when the resolved radius is positive.
    fn pad(&mut self, pad: &PadView, cardinal: bool) {
        let Some(corners) = pad.copper else { return };
        if cardinal && let Some((width, height)) = axis_aligned_size(&corners) {
            let aperture = match pad.shape {
                PadShape::Rectangle
                    if corners[0].x_nm + corners[2].x_nm == 2 * pad.center.x_nm && corners[0].y_nm + corners[2].y_nm == 2 * pad.center.y_nm =>
                {
                    Some(Aperture::Rectangle(width, height))
                }
                PadShape::Oval if width % 2 == height % 2 => Some(Aperture::Obround(width, height)),
                _ => None,
            };
            if let Some(aperture) = aperture {
                self.aperture(aperture);
                let _ = writeln!(self.body, "{}D03*", xy(self.frame, pad.center));
                return;
            }
        }
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
        for (aperture, code) in &self.apertures {
            match *aperture {
                Aperture::Circle(diameter) => {
                    let _ = writeln!(text, "%ADD{code}C,{}*%", mm(diameter, 6));
                }
                Aperture::Rectangle(width, height) | Aperture::Obround(width, height) => {
                    let kind = if matches!(aperture, Aperture::Rectangle(..)) { 'R' } else { 'O' };
                    let _ = writeln!(text, "%ADD{code}{kind},{}X{}*%", mm(width, 6), mm(height, 6));
                }
            }
        }
        text.push_str(&self.body);
        text.push_str("M02*\n");
        text
    }
}

/// Resolve global X/Y sizes of a nondegenerate ordered axis-aligned rectangle. Opposite corners
/// share a midpoint and adjacent edges use different axes. Untransformed R/O apertures use these
/// axes with strictly positive sizes (Ucamco Gerber 2026.05 section 4.4).
fn axis_aligned_size(corners: &[Point; 4]) -> Option<(i64, i64)> {
    if !(0..4).all(|i| {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        (a.x_nm == b.x_nm) != (a.y_nm == b.y_nm)
    }) {
        return None;
    }
    if (corners[0].x_nm == corners[1].x_nm) == (corners[1].x_nm == corners[2].x_nm)
        || corners[0].x_nm + corners[2].x_nm != corners[1].x_nm + corners[3].x_nm
        || corners[0].y_nm + corners[2].y_nm != corners[1].y_nm + corners[3].y_nm
    {
        return None;
    }
    let width = corners.iter().map(|p| p.x_nm).max()? - corners.iter().map(|p| p.x_nm).min()?;
    let height = corners.iter().map(|p| p.y_nm).max()? - corners.iter().map(|p| p.y_nm).min()?;
    (width > 0 && height > 0).then_some((width, height))
}

/// A Gerber coordinate pair in integer nanometers (format 4.6 mm) in the output frame.
fn xy(frame: Frame, point: Point) -> String {
    let point = frame.to_output(point);
    format!("X{}Y{}", point.x_nm, point.y_nm)
}

/// Millimeters with a fixed number of decimals, from nanometers.
///
/// Rounds in integers, with halves going up, so the rounding does not depend on where the
/// origin is: shifting every point by a whole number of output units shifts every written
/// value by exactly that amount. Zero is written without a sign.
fn mm(nm: i64, decimals: usize) -> String {
    let decimals = decimals.min(6) as u32;
    let unit = 10_i64.pow(6 - decimals);
    let scale = 10_u64.pow(decimals);
    let units = (nm + unit / 2).div_euclid(unit);
    let sign = if units < 0 { "-" } else { "" };
    let magnitude = units.unsigned_abs();
    format!("{sign}{}.{:0width$}", magnitude / scale, magnitude % scale, width = decimals as usize)
}

/// Chord each corner for a positive resolved radius; otherwise return the four input corners.
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

/// Keep file names portable: ASCII letters, digits and `-` are kept, and every other
/// character, including spaces, dots and non-ASCII letters, becomes `_`. A name with nothing
/// left but underscores, such as an empty one, falls back to `board`.
pub(crate) fn file_part(text: &str) -> String {
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
    /// Build every fabrication file for this board, in a fixed order. See [`Self::fabrication`].
    pub fn fabrication_outputs(self) -> Result<Vec<Output>, String> {
        self.require_linear_copper()?;
        self.fabrication().map(|fabrication| fabrication.files)
    }

    /// Build every fabrication file for this board, in a fixed order, measured from the board's
    /// placement origin, and name that origin. Requires a board setup with copper layers and a
    /// closed Board Outline; returns why when either is missing or the origin cannot be resolved.
    /// Circular copper is refused before generating any straight-only output.
    /// Refuses projects with placed NPTH until their separate drill output is supported.
    /// Sanitized layer names are disambiguated against all outputs, preserving fixed file names.
    pub fn fabrication(self) -> Result<Fabrication, String> {
        self.require_linear_copper()?;
        let board = &self.project().board;
        if board.footprints().iter().any(|footprint| board.definition_for(footprint).is_some_and(|d| !d.non_plated_holes.is_empty())) {
            return Err("Fabrication output does not yet support non-plated holes; no drill data was generated.".into());
        }
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
        let origin = self.placement_origin()?;
        let frame = origin.frame();
        let base = file_part(&self.project().name);
        let pads: Vec<_> = self
            .footprints()
            .flat_map(|footprint| {
                let definition = self.placement(&footprint.id).and_then(|placed| self.project().board.definition_for(placed));
                let cardinal_pads: BTreeSet<_> = definition
                    .into_iter()
                    .flat_map(|d| &d.pads)
                    .filter(|pad| pad.rotation.is_multiple_of(90))
                    .map(|pad| pad.number.as_str())
                    .collect();
                let placed_cardinal = footprint.rotation.is_multiple_of(90);
                footprint.pads.into_iter().filter_map(move |pad| {
                    pad.copper?;
                    let cardinal = placed_cardinal && cardinal_pads.contains(pad.number.as_str());
                    Some((pad, cardinal))
                })
            })
            .collect();
        let on =
            |pad: &PadView, layer: uuid::Uuid| matches!(&pad.copper_layers, Some(CopperLayers::Resolved { layers, .. }) if layers.contains(&layer));
        let mut files = Vec::new();

        for (index, layer_id) in order.iter().enumerate() {
            let Some(layer) = copper.iter().find(|layer| layer.id == *layer_id) else { continue };
            let mut gerber = Gerber::new(frame);
            for (pad, cardinal) in pads.iter().filter(|(pad, _)| on(pad, layer.id)) {
                gerber.pad(pad, *cardinal);
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
            let mut gerber = Gerber::new(frame);
            for (pad, cardinal) in pads.iter().filter(|(pad, _)| layer.is_some_and(|layer| on(pad, layer))) {
                gerber.pad(pad, *cardinal);
            }
            board_graphics(&mut gerber, self, LayerPurpose::Mask, Some(if side == "Top" { Side::Front } else { Side::Back }));
            files.push(Output { name: format!("{base}-{label}.gbr"), contents: gerber.finish(&format!("Soldermask,{side}"), "Negative") });
        }
        for (side, label, layer) in [("Top", "Paste_Top", front), ("Bot", "Paste_Bottom", back)] {
            let mut gerber = Gerber::new(frame);
            for (pad, cardinal) in pads.iter().filter(|(pad, _)| !pad.through_hole && layer.is_some_and(|layer| on(pad, layer))) {
                gerber.pad(pad, *cardinal);
            }
            board_graphics(&mut gerber, self, LayerPurpose::Paste, Some(if side == "Top" { Side::Front } else { Side::Back }));
            files.push(Output { name: format!("{base}-{label}.gbr"), contents: gerber.finish(&format!("Paste,{side}"), "Positive") });
        }

        for (side, position, label) in [(Side::Front, "Top", "Silkscreen_Top"), (Side::Back, "Bot", "Silkscreen_Bottom")] {
            let mut silk = Gerber::new(frame);
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

        let mut profile = Gerber::new(frame);
        for drawing in &outline {
            let mut points = drawing.points.clone();
            if drawing.closed {
                points.push(points[0]);
            }
            profile.stroke(&points, drawing.stroke_nm.unwrap_or(OUTLINE_NM));
        }
        files.push(Output { name: format!("{base}-Outline.gbr"), contents: profile.finish("Profile,NP", "Positive") });

        files.push(Output { name: format!("{base}-PTH.drl"), contents: self.drill_file(frame) });
        unique_file_names(&mut files, copper_count);
        Ok(Fabrication { origin, files })
    }

    /// One Excellon file for every plated hole: vias and through-hole pads, grouped by tool.
    /// Holes are sorted in board coordinates, so moving the origin never reorders them.
    fn drill_file(self, frame: Frame) -> String {
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
            for point in points.iter().map(|point| frame.to_output(*point)) {
                let _ = writeln!(text, "X{}Y{}", mm(point.x_nm, 3), mm(point.y_nm, 3));
            }
        }
        text.push_str("M30\n");
        text
    }
}

/// Pack the outputs and their `manifest.json` with fixed timestamps and Unix file metadata.
/// The same outputs give the same bytes. A reserved member name or encoding failure is an error.
pub fn zip(outputs: &[Output]) -> Result<Vec<u8>, String> {
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};
    const MANIFEST_NAME: &str = "manifest.json";
    if outputs.iter().any(|output| output.name.eq_ignore_ascii_case(MANIFEST_NAME)) {
        return Err(format!("{MANIFEST_NAME} is reserved for fabrication integrity records"));
    }
    let manifest = serde_json::to_string_pretty(&Manifest::from_outputs(outputs)).map_err(|error| error.to_string())?;
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
    writer.start_file(MANIFEST_NAME, options).map_err(|e| e.to_string())?;
    writeln!(writer, "{manifest}").map_err(|e| e.to_string())?;
    writer.finish().map(|cursor| cursor.into_inner()).map_err(|e| e.to_string())
}

impl Session {
    /// Generate the addressed board's fabrication files without editing anything.
    pub fn export_fabrication(&self, params: &Value) -> Result<Vec<Output>, CommandError> {
        self.fabrication(params).map(|fabrication| fabrication.files)
    }

    /// Capture the addressed board's outputs and their manifest in one ZIP without editing.
    pub fn export_fabrication_archive(&self, params: &Value) -> Result<Archive, CommandError> {
        let fabrication = self.fabrication(params)?;
        Archive::new(&fabrication.files).map_err(CommandError::Unavailable)
    }

    /// Generate the addressed board's fabrication files and the origin they use, without editing.
    fn fabrication(&self, params: &Value) -> Result<Fabrication, CommandError> {
        let project = self.open_project()?;
        let target = project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let view = project.board_view(target).ok_or_else(|| CommandError::Unavailable("board unavailable".into()))?;
        view.fabrication().map_err(CommandError::Unavailable)
    }

    /// Return file text, resolved origin and the same member records as the ZIP manifest.
    pub(crate) fn fabrication_report(&self, params: &Value) -> Result<Value, CommandError> {
        let fabrication = self.fabrication(params)?;
        let manifest = Manifest::from_outputs(&fabrication.files);
        Ok(json!({"files": fabrication.files, "origin": fabrication.origin, "manifest": manifest}))
    }
}

#[cfg(test)]
mod tests;
