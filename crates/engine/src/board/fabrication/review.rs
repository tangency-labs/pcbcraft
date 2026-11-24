//! Read-only inspection of fabrication archives written by PCBcraft.
mod drill;
mod gerber;

use super::{Artifact, Manifest};
use crate::schematic::Point;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
};

/// Bound ZIP reads on desktop and in the browser.
pub const MAX_ZIP_BYTES: usize = 64 * 1024 * 1024;
const MAX_EXPANDED_BYTES: usize = 128 * 1024 * 1024;
const MAX_FILES: usize = 80;
const MAX_MARKS: usize = 1_000_000;
const MAX_COORDINATE: i64 = 9_999_999_999;

#[derive(Clone, Copy, PartialEq, Eq)]
enum CoordinateFrame {
    Legacy,
    Identified(uuid::Uuid),
}

impl CoordinateFrame {
    /// Older exports use the literal Gerber frame marker; newer ones carry a shared GUID.
    fn parse(value: &str) -> Result<Self, String> {
        if value == "Original" {
            return Ok(Self::Legacy);
        }
        uuid::Uuid::parse_str(value).map(Self::Identified).map_err(|_| "Invalid fabrication coordinate frame identifier".into())
    }
}

/// Layer roles come from the plotted file headers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    TopCopper,
    InnerCopper,
    BottomCopper,
    TopMask,
    BottomMask,
    TopPaste,
    BottomPaste,
    TopSilkscreen,
    BottomSilkscreen,
    Profile,
    PlatedDrill,
    NonPlatedDrill,
}

impl LayerRole {
    /// Whether this layer plots drill hits, which are drawn over artwork.
    pub fn is_drill(self) -> bool {
        matches!(self, Self::PlatedDrill | Self::NonPlatedDrill)
    }

    /// A display label, including the intended interpretation of negative mask artwork.
    pub fn label(self) -> &'static str {
        match self {
            Self::TopCopper => "Top copper",
            Self::InnerCopper => "Inner copper",
            Self::BottomCopper => "Bottom copper",
            Self::TopMask => "Top mask openings",
            Self::BottomMask => "Bottom mask openings",
            Self::TopPaste => "Top paste",
            Self::BottomPaste => "Bottom paste",
            Self::TopSilkscreen => "Top silkscreen",
            Self::BottomSilkscreen => "Bottom silkscreen",
            Self::Profile => "Board outline",
            Self::PlatedDrill => "Plated drill",
            Self::NonPlatedDrill => "Non-plated drill",
        }
    }
}

/// Geometry in the exported, Y-up coordinate frame, in nanometres. Equal stroke ends
/// represent a circular flash or hole. Positive primitives in a layer form a union.
#[derive(Clone, Debug)]
pub enum Mark {
    Stroke { ends: [Point; 2], diameter_nm: i64 },
    Strip { ends: [Point; 2], width_nm: i64 },
    Rectangle { center: Point, size_nm: [i64; 2] },
    Polygon(Box<[Point]>),
}

impl Mark {
    /// An enclosing rectangle, outward rounded at sub-nanometre stroke edges.
    pub fn bounds(&self) -> [Point; 2] {
        match self {
            Self::Stroke { ends: [a, b], diameter_nm } | Self::Strip { ends: [a, b], width_nm: diameter_nm } => {
                let r = (diameter_nm + 1) / 2;
                [point(a.x_nm.min(b.x_nm) - r, a.y_nm.min(b.y_nm) - r), point(a.x_nm.max(b.x_nm) + r, a.y_nm.max(b.y_nm) + r)]
            }
            Self::Rectangle { center, size_nm: [w, h] } => {
                let (x, y) = ((w + 1) / 2, (h + 1) / 2);
                [point(center.x_nm - x, center.y_nm - y), point(center.x_nm + x, center.y_nm + y)]
            }
            Self::Polygon(points) => {
                points.iter().copied().fold([point(i64::MAX, i64::MAX), point(i64::MIN, i64::MIN)], |bounds, p| merge_bounds(bounds, [p, p]))
            }
        }
    }
}

/// Drill tool details from the file, including tools declared with zero hits.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Tool {
    pub number: u32,
    pub diameter_nm: i64,
    pub hits: usize,
}

/// Parsed layer with its exact filename, plot primitives and drill tool table.
#[derive(Clone, Debug)]
pub struct Layer {
    pub name: String,
    pub role: LayerRole,
    pub marks: Vec<Mark>,
    pub tools: Vec<Tool>,
}

/// A package preflight finding. Coordinates, when present, are in the exported frame.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Finding {
    MissingTopCopper,
    MissingBottomCopper,
    MissingPlatedDrill,
    MissingOutline,
    EmptyOutline,
    OpenOutline { endpoints: usize, first: Point },
}

/// The verified member bytes interpreted as a set of co-registered fabrication layers.
/// The source board revision cannot be identified from this manifest.
#[derive(Clone, Debug)]
pub struct Package {
    pub generator_version: String,
    pub layers: Vec<Layer>,
    pub findings: Vec<Finding>,
    pub bounds: Option<[Point; 2]>,
}

impl Package {
    /// Read and validate the fabrication ZIP, then parse every plot and drill member.
    /// Unsupported records, integrity mismatches and conflicting coordinate frames invalidate the package.
    pub fn read(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_ZIP_BYTES {
            return Err("Fabrication ZIP exceeds 64 MiB.".into());
        }
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| format!("Invalid fabrication ZIP: {error}"))?;
        if zip.has_duplicate_names() {
            return Err("Fabrication ZIP contains duplicate member names.".into());
        }
        if zip.len() > MAX_FILES {
            return Err("Fabrication ZIP contains too many members.".into());
        }
        let mut budget = MAX_EXPANDED_BYTES;
        let mut names = BTreeSet::new();
        let mut manifest = None;
        let mut members = Vec::new();
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index).map_err(|error| error.to_string())?;
            let name = entry.name().map_err(|error| error.to_string())?.into_owned();
            if name.is_empty() || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':')) || !names.insert(name.to_ascii_lowercase()) {
                return Err(format!("Invalid or duplicate fabrication member name: {name:?}"));
            }
            if entry.size() > budget as u64 {
                return Err("Expanded fabrication files exceed 128 MiB.".into());
            }
            let mut content = Vec::new();
            (&mut entry).take(budget as u64 + 1).read_to_end(&mut content).map_err(|error| format!("{name}: {error}"))?;
            if content.len() > budget || content.len() as u64 != entry.size() {
                return Err(format!("{name}: incorrect expanded size"));
            }
            budget -= content.len();
            if name == "manifest.json" {
                manifest = Some(serde_json::from_slice::<Manifest>(&content).map_err(|error| format!("Invalid fabrication manifest: {error}"))?);
            } else {
                members.push((name, content));
            }
        }
        let manifest = manifest.ok_or("This ZIP has no PCBcraft fabrication manifest.")?;
        if manifest.files.len() != members.len() || members.is_empty() {
            return Err("Fabrication manifest and ZIP member lists differ.".into());
        }
        let mut expected: BTreeMap<_, _> = manifest.files.iter().map(|file| (file.name.as_str(), file)).collect();
        let mut layers = Vec::with_capacity(members.len());
        let mut frame = None;
        let mut budget = MAX_MARKS;
        for (name, bytes) in members {
            let record = expected.remove(name.as_str()).ok_or_else(|| format!("{name}: missing manifest record"))?;
            if Artifact::new(name.clone(), &bytes) != *record {
                return Err(format!("{name}: size or SHA-256 differs from the fabrication manifest"));
            }
            let text = std::str::from_utf8(&bytes).map_err(|_| format!("{name}: expected UTF-8 fabrication data"))?;
            let (layer, layer_frame) = match name.rsplit_once('.').map(|(_, extension)| extension) {
                Some("gbr") => gerber::read(text, &mut budget),
                Some("drl") => drill::read(text, &mut budget),
                _ => Err("Unsupported fabrication member type".into()),
            }
            .map_err(|error| format!("{name}: {error}"))?;
            if *frame.get_or_insert(layer_frame) != layer_frame {
                return Err(format!("{name}: fabrication layer coordinate frames differ"));
            }
            layers.push(Layer { name, ..layer });
        }
        let bounds = layers.iter().flat_map(|layer| layer.marks.iter().map(Mark::bounds)).reduce(merge_bounds);
        let findings = preflight(&layers);
        Ok(Self { generator_version: manifest.generator_version, layers, findings, bounds })
    }
}

/// Check the presence of essential layers and closure of exported outline centerlines.
/// Coincident strokes contribute only one edge, independent of plot direction.
fn preflight(layers: &[Layer]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (role, finding) in [
        (LayerRole::TopCopper, Finding::MissingTopCopper),
        (LayerRole::BottomCopper, Finding::MissingBottomCopper),
        (LayerRole::PlatedDrill, Finding::MissingPlatedDrill),
    ] {
        if !layers.iter().any(|layer| layer.role == role) {
            findings.push(finding);
        }
    }
    let profiles: Vec<_> = layers.iter().filter(|layer| layer.role == LayerRole::Profile).collect();
    let mut degrees = BTreeMap::<_, usize>::new();
    let mut edges = BTreeSet::new();
    for mark in profiles.iter().flat_map(|layer| &layer.marks) {
        let Mark::Stroke { ends, .. } = mark else { continue };
        let mut edge = ends.map(|p| (p.x_nm, p.y_nm));
        edge.sort_unstable();
        if edge[0] != edge[1] && edges.insert(edge) {
            for endpoint in edge {
                *degrees.entry(endpoint).or_default() += 1;
            }
        }
    }
    if profiles.is_empty() {
        findings.push(Finding::MissingOutline);
    } else if degrees.is_empty() {
        findings.push(Finding::EmptyOutline);
    } else {
        let mut open = degrees.into_iter().filter(|(_, degree)| degree % 2 == 1);
        if let Some(((x, y), _)) = open.next() {
            findings.push(Finding::OpenOutline { endpoints: 1 + open.count(), first: point(x, y) });
        }
    }
    findings
}

fn point(x_nm: i64, y_nm: i64) -> Point {
    Point { x_nm, y_nm }
}

fn merge_bounds([a, b]: [Point; 2], [c, d]: [Point; 2]) -> [Point; 2] {
    [point(a.x_nm.min(c.x_nm), a.y_nm.min(c.y_nm)), point(b.x_nm.max(d.x_nm), b.y_nm.max(d.y_nm))]
}

/// Require bounded integral nanometres in a fixed-format Gerber coordinate.
fn coordinate(value: &str) -> Result<i64, String> {
    value.parse::<i64>().ok().filter(|value| value.unsigned_abs() <= MAX_COORDINATE as u64).ok_or_else(|| "Invalid or out-of-range coordinate".into())
}

/// Convert plain decimal millimetres to integral nanometres without binary rounding.
fn decimal(value: &str) -> Result<i64, String> {
    let negative = value.starts_with('-');
    let value = value.strip_prefix('-').unwrap_or(value);
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty() || fraction.len() > 6 || !whole.bytes().chain(fraction.bytes()).all(|byte| byte.is_ascii_digit()) {
        return Err("Expected decimal millimetres".into());
    }
    let whole = coordinate(whole)?;
    let fraction = if fraction.is_empty() { 0 } else { coordinate(fraction)? * 10_i64.pow(6 - fraction.len() as u32) };
    coordinate(&((whole * 1_000_000 + fraction) * if negative { -1 } else { 1 }).to_string())
}

/// Read an absolute pair in the writer's X…Y… form.
fn xy(value: &str, parse: fn(&str) -> Result<i64, String>) -> Result<Point, String> {
    let (x, y) = value.strip_prefix('X').and_then(|value| value.split_once('Y')).ok_or("Expected X and Y coordinates")?;
    Ok(point(parse(x)?, parse(y)?))
}

/// Charge every materialized mark against a shared package budget.
fn add(marks: &mut Vec<Mark>, mark: Mark, budget: &mut usize) -> Result<(), String> {
    *budget = budget.checked_sub(1).ok_or("Fabrication geometry exceeds the review limit")?;
    marks.push(mark);
    Ok(())
}

#[cfg(test)]
mod tests;
