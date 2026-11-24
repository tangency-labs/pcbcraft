//! Fabrication outputs: Gerber X2 layers and Excellon drill files, written from one board
//! snapshot with deterministic content.
//!
//! Each copper layer, the top and bottom solder mask, paste and silkscreen, and the
//! Board Outline get one Gerber file in millimeters with nanometer resolution (format 4.6).
//! Plated holes (vias and through-hole pads) go in one Excellon file. Circular non-plated
//! footprint holes, when present, go in a separate non-plated Excellon file. Each drill file
//! names its content in XNC standard-comment file attributes (`Plated,1,N,PTH` or
//! `NonPlated,1,N,NPTH` over all N copper layers). Every coordinate
//! is written in the board's output frame: measured from its placement origin with Y up, seen
//! from the top.
//! Openings use the pad exporter without process expansion: mask includes pads on the outer
//! copper layer, while paste excludes through-hole pads. Rounded pad macros use circular
//! primitives; exceptional snapped contours and obround strokes retain their existing fallbacks.
//! Vias receive no automatic mask opening; authored mask artwork can still open them.
//! Mounting-side pads follow the placement side; silkscreen follows the top or bottom legend.
//! X2 aperture functions describe the emitted features without changing their geometry.
//! Copper object attributes describe intended nets and component pins.
//! Track and Board Outline strokes remain linear. Export refuses boards with copper zones.
mod object_attributes;

use self::object_attributes::{Object, RoutedNets};
use super::{
    definition::{GraphicPurpose, PadShape},
    geometry::Contour,
    origin::{Frame, Resolved},
    setup::{LayerPurpose, Side, Stratum},
    view::{BoardView, CopperLayers, PadView},
};
use crate::{CommandError, Session, schematic::Point};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    io::Write as _,
};

mod part_layers;
pub mod review;

/// Generate the board's fabrication files; returns them without editing anything.
pub const EXPORT_FABRICATION: &str = "pcb.export_fabrication";
/// Silkscreen strokes without an authored width use 0.12 mm.
const DEFAULT_SILK_NM: i64 = 120_000;
/// The outline is drawn with a thin 0.1 mm line, the conventional profile width.
const OUTLINE_NM: i64 = 100_000;
/// Segments per corner for rounded pads that cannot use a macro.
const CORNER_SEGMENTS: usize = 6;
/// Allow 1 nm of radius error when comparing the emitted and authored diameters.
const ROUNDED_DIAMETER_TOLERANCE_NM: u64 = 2;
/// Two coordinates can move toward each other by 1 µm under Gerber §4.14.2.
const OUTLINE_CLEARANCE_NM: i64 = 1_000;
/// A single dimension can vary by ±0.5 µm under Gerber §4.14.2.
const SIZE_TOLERANCE_NM: i64 = OUTLINE_CLEARANCE_NM / 2;

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
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
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

/// Shape and geometry of an aperture, independent of its use.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Aperture {
    Circle(i64),
    Rectangle(i64, i64),
    Obround(i64, i64),
    /// A four-vertex regular polygon with a vertex on +X, used as the X3 key-pin diamond.
    Diamond(i64),
    Rounded {
        diameter: i64,
        core: RoundedCore,
        fill: RoundedFill,
    },
}

/// Aperture functions supported by the board writers (Ucamco Gerber 2026.05 §5.6.10).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ApertureFunction {
    SmdPad,
    /// A plated through-hole part land, emitted as `ComponentPad` (Ucamco Gerber 2026.05 §5.6.10).
    ThroughHolePad,
    ViaPad,
    Conductor,
    Material,
    Profile,
    /// A flash marking the placed part's reference point, independent of its pads.
    PartMain,
    PartBody,
    PartCourtyard,
    PartPin,
}

const DELETE_APERTURE_FUNCTION: &str = "%TD.AperFunction*%\n";

impl ApertureFunction {
    /// Add the function to the attribute dictionary, for the next AD or region to capture.
    fn set(self, text: &mut String) {
        let value = match self {
            Self::SmdPad => "SMDPad,CuDef",
            Self::ThroughHolePad => "ComponentPad",
            Self::ViaPad => "ViaPad",
            Self::Conductor => "Conductor",
            Self::Material => "Material",
            Self::Profile => "Profile",
            Self::PartMain => "ComponentMain",
            Self::PartBody => "ComponentOutline,Body",
            Self::PartCourtyard => "ComponentOutline,Courtyard",
            Self::PartPin => "ComponentPin",
        };
        let _ = writeln!(text, "%TA.AperFunction,{value}*%");
    }
}

/// Corner centers in Gerber axes, relative to the flash position, in nanometers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RoundedCore {
    Dot(MacroPoint),
    Line([MacroPoint; 2]),
    Quad([MacroPoint; 4]),
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RoundedFill {
    StripsOnly,
    Outline,
}

type MacroPoint = (i64, i64);

impl RoundedCore {
    /// Reuse the projected land core. A strict convex quad can be filled by a Gerber outline;
    /// collinear cores use just their extremes. Leave other snapped cores to the region path.
    fn from_board(core: [Point; 4], center: Point) -> Option<Self> {
        let points = core.map(|point| (point.x_nm - center.x_nm, center.y_nm - point.y_nm));
        let cross = |a: MacroPoint, b: MacroPoint, c: MacroPoint| {
            i128::from(b.0 - a.0) * i128::from(c.1 - a.1) - i128::from(b.1 - a.1) * i128::from(c.0 - a.0)
        };
        let low = points.into_iter().min()?;
        let high = points.into_iter().max()?;
        if low == high {
            return Some(Self::Dot(low));
        }
        if points.iter().all(|&p| cross(low, high, p) == 0) {
            return Some(Self::Line([low, high]));
        }
        let turns = std::array::from_fn::<_, 4, _>(|i| cross(points[i], points[(i + 1) % 4], points[(i + 2) % 4]));
        (turns.iter().all(|&turn| turn > 0) || turns.iter().all(|&turn| turn < 0)).then_some(Self::Quad(points))
    }
}

/// Accumulates Gerber drawing operations and the apertures they use.
struct Gerber {
    apertures: BTreeMap<(Aperture, Option<ApertureFunction>), usize>,
    body: String,
    current: Option<usize>,
    frame: Frame,
}

impl Gerber {
    fn new(frame: Frame) -> Self {
        Self { apertures: BTreeMap::new(), body: String::new(), current: None, frame }
    }

    /// Select an aperture by shape, size and function. Untagged geometry does not share
    /// a D-code with tagged geometry, even when their shape and size agree.
    fn aperture(&mut self, aperture: Aperture, function: Option<ApertureFunction>) {
        let next = 10 + self.apertures.len();
        let code = *self.apertures.entry((aperture, function)).or_insert(next);
        if self.current != Some(code) {
            let _ = writeln!(self.body, "D{code}*");
            self.current = Some(code);
        }
    }

    /// Scope X2 object attributes across all primitives for one feature, including region
    /// contours, without changing the selected aperture (Ucamco Gerber 2026.05 §§5.4–5.5).
    /// Returns what `draw` returns, so a refused pad still stops the export.
    fn object<T>(&mut self, attributes: Object<'_>, draw: impl FnOnce(&mut Self) -> T) -> T {
        attributes.set(&mut self.body);
        let drawn = draw(self);
        attributes.clear(&mut self.body);
        drawn
    }

    /// Stroke a polyline with an optionally tagged round aperture; a single point is flashed.
    fn stroke(&mut self, points: &[Point], width_nm: i64, function: Option<ApertureFunction>) {
        self.aperture(Aperture::Circle(width_nm.max(1)), function);
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
    /// Regions capture aperture attributes from the dictionary, not the selected D-code
    /// (Ucamco Gerber 2026.05 §5.3.1). Leave the dictionary clear between regions.
    fn region(&mut self, polygon: &[Point], function: ApertureFunction) {
        let Some(first) = polygon.first() else { return };
        function.set(&mut self.body);
        self.body.push_str("G36*\n");
        let _ = writeln!(self.body, "{}D02*", xy(self.frame, *first));
        for point in polygon[1..].iter().chain(std::iter::once(first)) {
            let _ = writeln!(self.body, "{}D01*", xy(self.frame, *point));
        }
        self.body.push_str("G37*\n");
        self.body.push_str(DELETE_APERTURE_FUNCTION);
    }

    /// Flash R/O only for individually cardinal library and placement rotations and a resolved
    /// rectangle. R also needs an integral midpoint; O needs integral capsule endpoints to match
    /// the engine's land contour. Other R lands remain regions, even if tiny corners collapse.
    /// Other O lands retain the approximate round-ended stroke (a circle for equal sides).
    /// A drawn copper obround is deliberately not identified as a fabrication pad (§§5.6.10, 6.4).
    /// Rounded rectangles flash their rounded land core when the macro can represent it and
    /// its emitted radius is within 1 nm of the authored radius; otherwise keep the resolved region.
    fn pad(&mut self, pad: &PadView, cardinal: bool, function: ApertureFunction) -> Result<(), String> {
        let Some(corners) = pad.copper else { return Ok(()) };
        if pad.shape == PadShape::RoundedRectangle {
            let contour = Contour::pad(pad.center, corners, pad.shape, pad.corner_radius_nm, pad.size_nm);
            // Snapping rotated corners can shorten a side and clamp a maximum radius. Compare
            // the emitted grid diameter, not the unrounded projection, to the authored diameter.
            let diameter = (2. * contour.radius).round() as i64;
            if pad.corner_radius_nm > 0
                && diameter > 0
                && diameter.abs_diff(2 * pad.corner_radius_nm) <= ROUNDED_DIAMETER_TOLERANCE_NM
                && let Some(core) = RoundedCore::from_board(contour.core, pad.center)
            {
                let fill = match core {
                    RoundedCore::Quad(points) => rounded_quad_fill(&points, diameter).ok_or_else(|| {
                        format!(
                            "Cannot export rounded pad at ({}, {}) mm in board coordinates: its core has no supported exact Gerber fill and its outline can self-intersect.",
                            mm(pad.center.x_nm, 6),
                            mm(pad.center.y_nm, 6),
                        )
                    })?,
                    RoundedCore::Dot(_) | RoundedCore::Line(_) => RoundedFill::StripsOnly,
                };
                self.flash(pad.center, Aperture::Rounded { diameter, core, fill }, function);
                return Ok(());
            }
        }
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
                self.flash(pad.center, aperture, function);
                return Ok(());
            }
        }
        let side = |a: Point, b: Point| (((b.x_nm - a.x_nm) as f64).hypot((b.y_nm - a.y_nm) as f64)).round() as i64;
        let (w, h) = (side(corners[0], corners[1]), side(corners[1], corners[2]));
        match pad.shape {
            PadShape::Rectangle => self.region(&corners, function),
            PadShape::RoundedRectangle => self.region(&rounded(&corners, pad.corner_radius_nm.min(w.min(h) / 2)), function),
            PadShape::Circle => self.stroke(&[pad.center], w.min(h), Some(function)),
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
                if half > 0. {
                    // The circular draw aperture describes the swept tool, not the copper pad.
                    let stroke_function = (function == ApertureFunction::Material).then_some(function);
                    self.stroke(&[end(-1.), end(1.)], short, stroke_function);
                } else {
                    self.stroke(&[pad.center], short, Some(function));
                }
            }
        }
        Ok(())
    }

    /// Select the pad's tagged aperture and flash at an absolute board point.
    fn flash(&mut self, point: Point, aperture: Aperture, function: ApertureFunction) {
        self.aperture(aperture, Some(function));
        let _ = writeln!(self.body, "{}D03*", xy(self.frame, point));
    }

    /// Finish a layer in the existing bare-board export, which writes all layers in one frame.
    fn finish(self, function: &str, polarity: &str, same_coordinates: &str) -> String {
        self.finish_in_frame(function, polarity, Some(same_coordinates))
    }

    /// Wrap one layer in a Gerber header and footer. Only a coordinated export supplies a frame
    /// identity (§5.6.5). Tagged AD commands capture the current dictionary entry, then TD clears
    /// it so later untagged apertures stay untagged (§§5.3, 5.5).
    fn finish_in_frame(self, function: &str, polarity: &str, same_coordinates: Option<&str>) -> String {
        let mut text = String::new();
        let _ = writeln!(text, "G04 PCBcraft fabrication output*");
        let _ = writeln!(text, "%TF.GenerationSoftware,PCBcraft,PCBcraft,{}*%", env!("CARGO_PKG_VERSION"));
        if let Some(identity) = same_coordinates {
            let _ = writeln!(text, "%TF.SameCoordinates,{identity}*%");
        }
        let _ = writeln!(text, "%TF.FileFunction,{function}*%");
        let _ = writeln!(text, "%TF.FilePolarity,{polarity}*%");
        text.push_str("%FSLAX46Y46*%\n%MOMM*%\n%LPD*%\nG01*\n");
        for ((aperture, aperture_function), code) in &self.apertures {
            // Define a macro template first, so the function attribute directly precedes its AD.
            if let Aperture::Rounded { diameter, core, fill } = *aperture {
                write_rounded_macro(&mut text, *code, diameter, core, fill);
            }
            if let Some(function) = aperture_function {
                function.set(&mut text);
            }
            match *aperture {
                Aperture::Circle(diameter) => {
                    let _ = writeln!(text, "%ADD{code}C,{}*%", mm(diameter, 6));
                }
                Aperture::Diamond(diameter) => {
                    let _ = writeln!(text, "%ADD{code}P,{}X4X0.0*%", mm(diameter, 6));
                }
                Aperture::Rectangle(width, height) | Aperture::Obround(width, height) => {
                    let kind = if matches!(aperture, Aperture::Rectangle(..)) { 'R' } else { 'O' };
                    let _ = writeln!(text, "%ADD{code}{kind},{}X{}*%", mm(width, 6), mm(height, 6));
                }
                Aperture::Rounded { .. } => {
                    let _ = writeln!(text, "%ADD{code}RoundRect{code}*%");
                }
            }
            if aperture_function.is_some() {
                text.push_str(DELETE_APERTURE_FUNCTION);
            }
        }
        text.push_str(&self.body);
        text.push_str("M02*\n");
        text
    }
}

/// Add a strip along each edge and a circle at each vertex, and fill the inner polygon unless
/// the strips already cover it safely.
/// Their union is the core expanded by half the emitted diameter, without chord approximations.
/// Ucamco Gerber 2026.05 §§4.5.1.3, 4.5.1.4 and 4.5.1.6 define these primitives. Their
/// rotation is about the macro origin, not each primitive's center: use zero because
/// the core already includes pad rotation, placement mirroring/rotation and the Y flip.
fn write_rounded_macro(text: &mut String, code: usize, diameter: i64, core: RoundedCore, fill: RoundedFill) {
    let _ = writeln!(text, "%AMRoundRect{code}*");
    let diameter_mm = mm(diameter, 6);
    let pair = |(x, y)| format!("{},{}", mm(x, 6), mm(y, 6));
    let edge = |text: &mut String, a, b| {
        let _ = writeln!(text, "20,1,{diameter_mm},{},{},0*", pair(a), pair(b));
    };
    let vertices = match &core {
        RoundedCore::Dot(point) => std::slice::from_ref(point),
        RoundedCore::Line(points) => {
            edge(text, points[0], points[1]);
            points.as_slice()
        }
        RoundedCore::Quad(points) => {
            for (a, b) in points.iter().copied().zip(points.iter().copied().cycle().skip(1)) {
                edge(text, a, b);
            }
            if fill == RoundedFill::Outline {
                text.push_str("4,1,4");
                for &point in points.iter().chain(&points[..1]) {
                    let _ = write!(text, ",{}", pair(point));
                }
                text.push_str(",0*\n");
            }
            points.as_slice()
        }
    };
    for &point in vertices {
        let _ = writeln!(text, "1,1,{diameter_mm},{},0*", pair(point));
    }
    text.push_str("%\n");
}

/// Choose a fill before emitting any pad artwork, keeping every primitive overlap and
/// outline separation safe under Gerber §4.14.2 coordinate and size perturbations.
/// Every accepted macro anchors each corner circle on the exact vertex shared with its
/// adjacent edge strips (and outline when present), so coincident coordinates move together.
/// Center-line rectangles cannot guarantee corner-circle connectivity in the remaining narrow
/// band (min core span <= 1 µm and diameter <= min core span + 1.5 µm <= 2.5 µm) under §4.14.2,
/// so those narrow cores are refused.
fn rounded_quad_fill(points: &[MacroPoint; 4], diameter: i64) -> Option<RoundedFill> {
    if strips_cover_core(points, diameter) {
        return Some(RoundedFill::StripsOnly);
    }
    let [a, b, c, d] = *points;
    (perturbed_edges_separate(a, b, c, d) && perturbed_edges_separate(b, c, d, a)).then_some(RoundedFill::Outline)
}

/// Endpoints can move within ±0.5 µm boxes (§4.14.2). The swept segment is the segment
/// plus that box; separating axes are X, Y, and the normals to the two original segments.
/// Require a gap larger than the combined projection of both boxes on one of these axes.
fn perturbed_edges_separate(a: MacroPoint, b: MacroPoint, c: MacroPoint, d: MacroPoint) -> bool {
    [(1, 0), (0, 1), (a.1 - b.1, b.0 - a.0), (c.1 - d.1, d.0 - c.0)].into_iter().any(|(nx, ny)| {
        let project = |(x, y)| i128::from(nx) * i128::from(x) + i128::from(ny) * i128::from(y);
        let (a, b, c, d) = (project(a), project(b), project(c), project(d));
        let gap = (a.min(b) - c.max(d)).max(c.min(d) - a.max(b));
        gap > i128::from(OUTLINE_CLEARANCE_NM) * (i128::from(nx.abs()) + i128::from(ny.abs()))
    })
}

/// Whether the diameter-wide strips along a strictly convex core's edges still cover all of it
/// under Gerber §4.14.2 coordinate (±0.5 µm per coordinate) and size (±0.5 µm diameter) tolerances.
/// They do when every vertex stays strictly within the reduced diameter of some edge's line even
/// after opposite edges move apart by the full perturbation box projection: each interior point is
/// then still within half a reduced diameter of its nearest edge, with no gap between opposite strips.
fn strips_cover_core(points: &[MacroPoint; 4], diameter: i64) -> bool {
    let Some(effective_diameter) = diameter.checked_sub(SIZE_TOLERANCE_NM) else {
        return false;
    };
    effective_diameter > OUTLINE_CLEARANCE_NM
        && (0..4).any(|i| {
            let (a, b) = (points[i], points[(i + 1) % 4]);
            let (dx, dy) = (i128::from(b.0 - a.0), i128::from(b.1 - a.1));
            let margin = i128::from(OUTLINE_CLEARANCE_NM) * (dx.abs() + dy.abs());
            let limit = i128::from(effective_diameter).pow(2) * (dx * dx + dy * dy);
            points.iter().all(|&p| {
                let cross = (dx * i128::from(p.1 - a.1) - dy * i128::from(p.0 - a.0)).abs();
                (cross + margin).pow(2) < limit
            })
        })
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
        gerber.stroke(&points, drawing.stroke_nm.unwrap_or(OUTLINE_NM), Some(ApertureFunction::Material));
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
    /// Refuses circular copper and copper zones before generating any output files.
    /// Outputs include an independent non-plated drill file only when the board has such holes.
    /// Returns an error if a plated or non-plated drill diameter rounds to zero at the output resolution.
    /// Sanitized layer names are disambiguated against all outputs, preserving fixed file names.
    /// X2 attributes describe pad, track and via functions and intended copper membership.
    /// Ambiguous routed copper does not acquire an arbitrary net name.
    /// A placed slot is refused until the plated machining output can preserve its geometry.
    pub fn fabrication(self) -> Result<Fabrication, String> {
        self.require_linear_copper()?;
        if !self.zones().is_empty() {
            return Err("Fabrication export is unavailable on boards with copper zones.".into());
        }
        let board = &self.project().board;
        if board.footprints().iter().filter_map(|footprint| board.definition_for(footprint)).any(|definition| definition.has_plated_slots()) {
            return Err("Fabrication output cannot yet machine plated slots; no files were generated.".into());
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
        let plated = self.plated_drill_hits()?;
        let frame_id = same_coordinates_id(origin);
        let non_plated = self.non_plated_drill_hits()?;
        let base = file_part(&self.project().name);
        let routed_nets = RoutedNets::new(self);
        let nets = &routed_nets;
        let pads: Vec<_> = self
            .footprints()
            .flat_map(|footprint| {
                let placement = self.placement(&footprint.id);
                let definition = placement.and_then(|placed| self.project().board.definition_for(placed));
                let unconnected: BTreeSet<_> = placement.into_iter().flat_map(|placed| &placed.unconnected_pads).map(String::as_str).collect();
                let reference = footprint.reference;
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
                    let has_pin = !unconnected.contains(pad.number.as_str());
                    let net = nets.pad_name(&footprint.id, &pad.number);
                    Some((pad, cardinal, reference.clone(), has_pin, net))
                })
            })
            .collect();
        let on =
            |pad: &PadView, layer: uuid::Uuid| matches!(&pad.copper_layers, Some(CopperLayers::Resolved { layers, .. }) if layers.contains(&layer));
        let mut files = Vec::new();

        for (index, layer_id) in order.iter().enumerate() {
            let Some(layer) = copper.iter().find(|layer| layer.id == *layer_id) else { continue };
            let mut gerber = Gerber::new(frame);
            let outer = index == 0 || index + 1 == order.len();
            for (pad, cardinal, reference, has_pin, net) in pads.iter().filter(|(pad, ..)| on(pad, layer.id)) {
                // Plated through-hole lands keep their function on inner copper, but .P describes
                // part pins on outer copper, not inner plated lands (§5.6.14). The pad-derived
                // mask opening does not shrink the mounting-side copper land (CuDef).
                let function = if pad.through_hole { ApertureFunction::ThroughHolePad } else { ApertureFunction::SmdPad };
                let pin = outer.then_some(if *has_pin { pad.number.as_str() } else { "" });
                gerber.object(Object { net: Some(net), component: Some((reference, pin)) }, |gerber| gerber.pad(pad, *cardinal, function))?;
            }
            let mut tracks: Vec<_> = self.tracks().iter().filter(|track| track.layer == layer.id).collect();
            tracks.sort_by_key(|track| (track.width_nm, track.id));
            for track in tracks {
                gerber.object(routed_nets.object(track.id), |gerber| {
                    gerber.stroke(&[track.start, track.end], track.width_nm, Some(ApertureFunction::Conductor));
                });
            }
            for via in self.vias() {
                gerber.object(routed_nets.object(via.id), |gerber| gerber.stroke(&[via.at], via.diameter_nm, Some(ApertureFunction::ViaPad)));
            }
            let position = match (index, layer.side) {
                (_, Some(Side::Front)) => "Top",
                (_, Some(Side::Back)) => "Bot",
                _ => "Inr",
            };
            files.push(Output {
                name: format!("{base}-{}.gbr", file_part(&layer.name)),
                contents: gerber.finish(&format!("Copper,L{},{position}", index + 1), "Positive", &frame_id),
            });
        }

        let copper_count = files.len();
        let front = order.first().copied();
        let back = order.last().copied().filter(|_| order.len() > 1);
        for (side, label, layer) in [("Top", "Mask_Top", front), ("Bot", "Mask_Bottom", back)] {
            let mut gerber = Gerber::new(frame);
            for (pad, cardinal, ..) in pads.iter().filter(|(pad, ..)| layer.is_some_and(|layer| on(pad, layer))) {
                gerber.pad(pad, *cardinal, ApertureFunction::Material)?;
            }
            board_graphics(&mut gerber, self, LayerPurpose::Mask, Some(if side == "Top" { Side::Front } else { Side::Back }));
            files.push(Output { name: format!("{base}-{label}.gbr"), contents: gerber.finish(&format!("Soldermask,{side}"), "Negative", &frame_id) });
        }
        for (side, label, layer) in [("Top", "Paste_Top", front), ("Bot", "Paste_Bottom", back)] {
            let mut gerber = Gerber::new(frame);
            for (pad, cardinal, ..) in pads.iter().filter(|(pad, ..)| !pad.through_hole && layer.is_some_and(|layer| on(pad, layer))) {
                gerber.pad(pad, *cardinal, ApertureFunction::Material)?;
            }
            board_graphics(&mut gerber, self, LayerPurpose::Paste, Some(if side == "Top" { Side::Front } else { Side::Back }));
            files.push(Output { name: format!("{base}-{label}.gbr"), contents: gerber.finish(&format!("Paste,{side}"), "Positive", &frame_id) });
        }

        for (side, position, label) in [(Side::Front, "Top", "Silkscreen_Top"), (Side::Back, "Bot", "Silkscreen_Bottom")] {
            let mut silk = Gerber::new(frame);
            for footprint in self.footprints() {
                for graphic in footprint.graphics.iter().filter(|graphic| graphic.purpose == GraphicPurpose::Silkscreen && graphic.side == side) {
                    let mut points = graphic.points.clone();
                    if graphic.closed && points.len() > 2 {
                        points.push(points[0]);
                    }
                    silk.stroke(&points, graphic.stroke_nm.unwrap_or(DEFAULT_SILK_NM), Some(ApertureFunction::Material));
                }
            }
            board_graphics(&mut silk, self, LayerPurpose::Silkscreen, Some(side));
            files.push(Output { name: format!("{base}-{label}.gbr"), contents: silk.finish(&format!("Legend,{position}"), "Positive", &frame_id) });
        }

        let mut profile = Gerber::new(frame);
        for drawing in &outline {
            let mut points = drawing.points.clone();
            if drawing.closed {
                points.push(points[0]);
            }
            // Keep legacy artwork in the output, but do not identify an open drawing as
            // the board's physical edge (§§5.6.10, 6.5).
            let function = drawing.closed.then_some(ApertureFunction::Profile);
            profile.stroke(&points, drawing.stroke_nm.unwrap_or(OUTLINE_NM), function);
        }
        files.push(Output { name: format!("{base}-Outline.gbr"), contents: profile.finish("Profile,NP", "Positive", &frame_id) });

        let span = order.len();
        files.push(Output { name: format!("{base}-PTH.drl"), contents: drill_file(frame, &frame_id, DrillPlating::Plated, span, plated) });
        if !non_plated.is_empty() {
            files
                .push(Output { name: format!("{base}-NPTH.drl"), contents: drill_file(frame, &frame_id, DrillPlating::NonPlated, span, non_plated) });
        }
        unique_file_names(&mut files, copper_count);
        Ok(Fabrication { origin, files })
    }

    /// Group via and plated pad drill hits by diameter.
    /// A plated pad drill below half the output unit returns an error because its tool size would round to zero.
    fn plated_drill_hits(self) -> Result<BTreeMap<i64, Vec<Point>>, String> {
        let mut holes: BTreeMap<i64, Vec<Point>> = BTreeMap::new();
        for via in self.vias() {
            holes.entry(via.drill_nm).or_default().push(via.at);
        }
        for pad in self.footprints().flat_map(|footprint| footprint.pads) {
            if let Some(drill) = pad.drill_nm {
                if drill < DRILL_UNIT_NM / 2 {
                    return Err(format!("Plated drill diameter {drill} nm rounds to zero at the 0.001 mm output resolution."));
                }
                holes.entry(drill).or_default().push(pad.center);
            }
        }
        Ok(holes)
    }

    /// Resolve mechanical openings from the pinned footprint with no copper or pad projection.
    /// A diameter below half the output unit would write an unusable zero tool size.
    fn non_plated_drill_hits(self) -> Result<BTreeMap<i64, Vec<Point>>, String> {
        let board = &self.project().board;
        let mut holes: BTreeMap<i64, Vec<Point>> = BTreeMap::new();
        for footprint in board.footprints() {
            let Some(definition) = board.definition_for(footprint) else { continue };
            for hole in footprint.non_plated_holes(definition) {
                if hole.diameter_nm < DRILL_UNIT_NM / 2 {
                    return Err(format!(
                        "Non-plated drill diameter {} nm in footprint-{} rounds to zero at the 0.001 mm output resolution.",
                        hole.diameter_nm, footprint.id
                    ));
                }
                holes.entry(hole.diameter_nm).or_default().push(hole.center);
            }
        }
        Ok(holes)
    }
}

const DRILL_DECIMALS: usize = 3;
const DRILL_UNIT_NM: i64 = 10_i64.pow(6 - DRILL_DECIMALS as u32);

enum DrillPlating {
    Plated,
    NonPlated,
}

/// Deterministic Gerber X2 `.SameCoordinates` GUID (§5.6.5) for one resolved export frame.
/// Derived from the resolved origin coordinates, axis convention and output unit so every
/// Gerber, PTH and NPTH file in an export shares one identifier, repeated exports in the same
/// frame stay identical, and moving the origin produces a distinct identifier.
fn same_coordinates_id(origin: Resolved) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"pcbcraft-same-coordinates/1\n");
    hasher.update(origin.axes.as_bytes());
    hasher.update(b"\nmm\n");
    hasher.update(origin.at.x_nm.to_le_bytes());
    hasher.update(origin.at.y_nm.to_le_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // Set RFC 9562 version 8 (custom deterministic) and RFC 4122 variant bits.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).hyphenated().to_string()
}

/// Write one Excellon file from resolved through hits using absolute metric coordinates.
/// Tools are sorted by diameter and hits in board coordinates, independent of the output origin.
/// The header carries X2 file attributes as XNC standard comments (`; #@! `, XNC 2021.11 §4.1):
/// the generating software, the shared coordinate frame of this export's Gerber layers (Gerber
/// §5.6.5) and the file function spanning copper layers 1 to `copper_layers` (§5.6.3).
fn drill_file(frame: Frame, same_coordinates: &str, plating: DrillPlating, copper_layers: usize, mut holes: BTreeMap<i64, Vec<Point>>) -> String {
    let (description, kind, function) = match plating {
        DrillPlating::Plated => ("plated", "PLATED", format!("Plated,1,{copper_layers},PTH")),
        DrillPlating::NonPlated => ("non-plated", "NON_PLATED", format!("NonPlated,1,{copper_layers},NPTH")),
    };
    let mut text = format!("M48\n; PCBcraft {description} drill file\n; TYPE={kind}\n");
    let _ = writeln!(text, "; #@! TF.GenerationSoftware,PCBcraft,PCBcraft,{}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(text, "; #@! TF.SameCoordinates,{same_coordinates}");
    let _ = writeln!(text, "; #@! TF.FileFunction,{function}");
    text.push_str("FMAT,2\nMETRIC\n");
    for (index, diameter) in holes.keys().enumerate() {
        let _ = writeln!(text, "T{}C{}", index + 1, mm(*diameter, DRILL_DECIMALS));
    }
    text.push_str("%\nG90\nG05\n");
    for (index, points) in holes.values_mut().enumerate() {
        points.sort_by_key(|point| (point.x_nm, point.y_nm));
        let _ = writeln!(text, "T{}", index + 1);
        for point in points.iter().map(|point| frame.to_output(*point)) {
            let _ = writeln!(text, "X{}Y{}", mm(point.x_nm, DRILL_DECIMALS), mm(point.y_nm, DRILL_DECIMALS));
        }
    }
    text.push_str("M30\n");
    text
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
