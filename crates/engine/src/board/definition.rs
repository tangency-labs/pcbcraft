//! Original native footprint definitions, pinned independently from symbol graphics and placements.
use crate::schematic::Point;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};

/// Stable identity of an immutable library item revision; paths and labels are only locators.
pub(crate) type Key = (String, String, u32);
pub(super) const MAX_OFFSET_NM: i64 = 100_000_000;
pub(crate) const STARTER_IDS: [&str; 7] =
    ["resistor-0402", "resistor-0603", "capacitor-0402", "capacitor-0603", "led-0402", "sot23", "header-1x02-254"];
const STARTER: [&str; 7] = [
    include_str!("../../../../libraries/starter/footprints/resistor-0402.json"),
    include_str!("../../../../libraries/starter/footprints/resistor-0603.json"),
    include_str!("../../../../libraries/starter/footprints/capacitor-0402.json"),
    include_str!("../../../../libraries/starter/footprints/capacitor-0603.json"),
    include_str!("../../../../libraries/starter/footprints/led-0402.json"),
    include_str!("../../../../libraries/starter/footprints/sot23.json"),
    include_str!("../../../../libraries/starter/footprints/header-1x02-254.json"),
];

/// A physical land relative to its footprint origin. Surface lands bind to the component
/// side; plated through-hole lands explicitly bind to all copper and carry a circular drill.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RectPad {
    pub number: String,
    pub center: Point,
    pub width_nm: i64,
    pub height_nm: i64,
    pub rotation: u16,
    pub shape: PadShape,
    pub copper: CopperBinding,
    /// Physical corner radius; omitted/zero for rectangles and circular lands.
    #[serde(default, skip_serializing_if = "zero")]
    pub corner_radius_nm: i64,
    /// Diameter of a circular plated drill; present only for all-copper through-hole lands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drill_nm: Option<i64>,
}

/// Keep the original rectangle-only JSON encoding when no physical rounding is requested.
fn zero(value: &i64) -> bool {
    *value == 0
}

/// Shapes supported by this schema. Unsupported kinds fail instead of becoming marker geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PadShape {
    Rectangle,
    RoundedRectangle,
    Circle,
    Oval,
}

/// Copper binding distinguishes a component-side surface land from a plated through-hole land.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CopperBinding {
    ComponentSide,
    AllCopper,
}

/// Technical graphics are kept separate from copper and retain their physical layer purpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphicPurpose {
    Fabrication,
    Silkscreen,
    Courtyard,
}

/// A technical graphic retains its layer role and parameterized geometry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artwork {
    pub purpose: GraphicPurpose,
    pub graphic: crate::library_graphics::Graphic,
}

/// One unfilled rectangle with physical dimensions and local orientation.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphicRect {
    pub purpose: GraphicPurpose,
    pub center: Point,
    pub width_nm: i64,
    pub height_nm: i64,
    pub rotation: u16,
}

/// A physical silkscreen centerline. Ink width and pad clearance come from board settings.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SilkPath {
    pub points: Vec<Point>,
    pub closed: bool,
}

/// Original geometry records its numeric reference without importing vendor artwork or files.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub kind: String,
    pub reference: String,
    pub locator: String,
}

/// A bounded physical footprint definition independent of any schematic component.
/// It is pinned in the project at an exact library/item/revision and opens offline.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FootprintDefinition {
    pub format: String,
    pub library: String,
    pub id: String,
    pub revision: u32,
    pub name: String,
    pub license: String,
    pub source: Source,
    pub pads: Vec<RectPad>,
    pub graphics: Vec<GraphicRect>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artwork: Vec<Artwork>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub silkscreen: Vec<SilkPath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// Optional local displacement of the default reference from the automatic anchor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_offset: Option<Point>,
}

/// Validate portable identity segments before using them in a native relative filename.
pub(crate) fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

/// Bound a single-line metadata value, rejecting empty, padded or control-bearing fields.
fn text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.trim() == value && !value.chars().any(char::is_control)
}

/// Bound local rectangle geometry so rotation and translation cannot overflow board coordinates.
fn rectangle(center: Point, width: i64, height: i64, rotation: u16) -> bool {
    (-MAX_OFFSET_NM..=MAX_OFFSET_NM).contains(&center.x_nm)
        && (-MAX_OFFSET_NM..=MAX_OFFSET_NM).contains(&center.y_nm)
        && (1..=MAX_OFFSET_NM).contains(&width)
        && (1..=MAX_OFFSET_NM).contains(&height)
        && rotation < 360
}

impl RectPad {
    /// Require a supported land contour and a smaller, positive plated drill for all-copper lands.
    fn valid(&self) -> bool {
        rectangle(self.center, self.width_nm, self.height_nm, self.rotation)
            && match self.shape {
                PadShape::Rectangle => self.corner_radius_nm == 0,
                PadShape::RoundedRectangle => (1..=self.width_nm.min(self.height_nm) / 2).contains(&self.corner_radius_nm),
                PadShape::Oval => self.corner_radius_nm == 0,
                PadShape::Circle => self.width_nm == self.height_nm && self.corner_radius_nm == 0,
            }
            && match (self.copper, self.drill_nm) {
                (CopperBinding::ComponentSide, None) => true,
                (CopperBinding::AllCopper, Some(diameter)) => diameter > 0 && diameter < self.width_nm.min(self.height_nm),
                _ => false,
            }
    }
}

impl FootprintDefinition {
    /// Resolve generated reference strokes in local coordinates for editing and fabrication.
    /// Apply both the library default and the placement displacement before clipping to copper.
    pub fn reference_paths(&self, text: &str, style: super::silkscreen::Settings, offset: Point) -> Vec<Vec<Point>> {
        let Some(font) = self.reference.as_deref() else { return Vec::new() };
        if !super::valid_reference_offset(offset) {
            return Vec::new();
        }
        let default = self.reference_offset.unwrap_or(super::DEFAULT_REFERENCE_OFFSET);
        if !super::valid_reference_offset(default) {
            return Vec::new();
        }
        let top_pad = self.pads.iter().map(|p| p.center.y_nm - p.width_nm.max(p.height_nm) / 2).min().unwrap_or(0);
        let top_line = self.silkscreen.iter().flat_map(|path| &path.points).map(|p| p.y_nm - style.stroke_width_nm / 2).min().unwrap_or(top_pad);
        let lettering = if font == "pcbcraft-technical-1" { super::silkscreen::lettering } else { super::silkscreen::legacy_lettering };
        let mut paths = lettering(text, 0, style);
        let bottom = paths.iter().flatten().map(|p| p.y_nm).max().unwrap_or(0) + (style.stroke_width_nm + 1) / 2;
        let y = top_pad.min(top_line) - style.pad_clearance_nm - bottom;
        for point in paths.iter_mut().flatten() {
            point.x_nm += default.x_nm + offset.x_nm;
            point.y_nm += default.y_nm + y + offset.y_nm;
        }
        super::silkscreen::clipped(paths, &self.pads, style)
    }

    /// Return the exact identity used by placements and pinned project copies.
    pub(crate) fn key(&self) -> Key {
        (self.library.clone(), self.id.clone(), self.revision)
    }

    /// Validate the schema, provenance, bounded graphics and distinct electrical pad identifiers.
    /// Arbitrary pad stacks, slots and non-plated drills remain unsupported.
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.validate_mode(false)
    }
    /// Empty pad sets are permitted only in editable drafts, never silently published.
    pub(crate) fn validate_draft(&self) -> Result<(), String> {
        self.validate_mode(true)
    }
    /// Check shared constraints before accepting either an editable draft or a published package.
    fn validate_mode(&self, draft: bool) -> Result<(), String> {
        let numbers: BTreeSet<_> = self.pads.iter().map(|pad| pad.number.as_str()).collect();
        if self.format != "pcbcraft-footprint/1"
            || !valid_id(&self.library)
            || !valid_id(&self.id)
            || self.revision == 0
            || !text(&self.name, 200)
            || !text(&self.license, 200)
            || !text(&self.source.kind, 64)
            || !text(&self.source.reference, 2048)
            || !text(&self.source.locator, 1024)
            || (!draft && self.pads.is_empty())
            || self.pads.len() > 128
            || numbers.len() != self.pads.len()
            || self.pads.iter().any(|p| crate::schematic::PinNumber::new(&p.number).is_none())
            || self.artwork.len() > 128
            || self.artwork.iter().any(|a| a.graphic.fill != crate::library_graphics::Fill::None || a.graphic.validate(MAX_OFFSET_NM).is_err())
            || self.graphics.len() > 64
            || self.silkscreen.len() > 64
            || self.reference.as_deref().is_some_and(|font| !matches!(font, "pcbcraft-stroke-1" | "pcbcraft-technical-1"))
            || self
                .reference_offset
                .is_some_and(|offset| self.reference.is_none() || offset == super::DEFAULT_REFERENCE_OFFSET || !super::valid_reference_offset(offset))
            || self.silkscreen.iter().any(|path| {
                !(2..=64).contains(&path.points.len())
                    || (path.closed && path.points.len() < 3)
                    || path.points.iter().any(|p| !rectangle(*p, 1, 1, 0))
            })
            || self.pads.iter().any(|p| !p.valid())
            || self.graphics.iter().any(|g| !rectangle(g.center, g.width_nm, g.height_nm, g.rotation))
        {
            return Err("invalid footprint identity, metadata, pad identities or rectangle geometry".into());
        }
        Ok(())
    }
}

/// Parse and validate a project copy without consulting built-in sources or upgrading its revision.
pub(crate) fn read_saved(bytes: &[u8]) -> Result<Arc<FootprintDefinition>, String> {
    let definition: FootprintDefinition = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    definition.validate()?;
    Ok(Arc::new(definition))
}

/// Resolve an original built-in definition. A project pins this copy once, rather than resolving
/// its placement geometry from the installed catalog on every load.
pub(crate) fn starter(id: &str) -> Option<Arc<FootprintDefinition>> {
    static CATALOG: std::sync::LazyLock<Vec<Option<Arc<FootprintDefinition>>>> =
        std::sync::LazyLock::new(|| STARTER.iter().map(|text| read_saved(text.as_bytes()).ok()).collect());
    STARTER_IDS.iter().position(|name| *name == id).and_then(|index| CATALOG[index].clone())
}

#[cfg(test)]
mod tests;
