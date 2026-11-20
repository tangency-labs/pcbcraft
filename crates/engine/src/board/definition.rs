//! Original native footprint definitions, pinned independently from symbol graphics and placements.
use crate::schematic::{Point, TextAlign};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};

/// Stable identity of an immutable library item revision; paths and labels are only locators.
pub(crate) type Key = (String, String, u32);
pub(super) const MAX_OFFSET_NM: i64 = 100_000_000;
/// Most pads in one footprint: a full 32 x 32 ball grid array.
pub(crate) const MAX_PADS: usize = 1024;
/// Most parameterized artwork objects in one footprint.
pub(crate) const MAX_ARTWORK: usize = 256;
const LEGACY_FORMAT: &str = "pcbcraft-footprint/1";
const NPTH_FORMAT: &str = "pcbcraft-footprint/4";
const MAX_MECHANICAL_HOLES: usize = 128;
pub(crate) const BASIC_COMPONENT_IDS: [&str; 18] = [
    "resistor-0402",
    "resistor-0603",
    "capacitor-0402",
    "capacitor-0603",
    "led-0402",
    "sot23",
    "header-1x02-254",
    "resistor-0805",
    "resistor-1206",
    "soic8",
    "sot563",
    "sot323",
    "sod123",
    "header-1x03-254",
    "header-1x04-254",
    "header-1x06-254",
    "header-1x08-254",
    "header-2x03-254",
];
const BASIC_COMPONENTS: [&str; 18] = [
    include_str!("../../../../libraries/basic-components/footprints/resistor-0402.json"),
    include_str!("../../../../libraries/basic-components/footprints/resistor-0603.json"),
    include_str!("../../../../libraries/basic-components/footprints/capacitor-0402.json"),
    include_str!("../../../../libraries/basic-components/footprints/capacitor-0603.json"),
    include_str!("../../../../libraries/basic-components/footprints/led-0402.json"),
    include_str!("../../../../libraries/basic-components/footprints/sot23.json"),
    include_str!("../../../../libraries/basic-components/footprints/header-1x02-254.json"),
    include_str!("../../../../libraries/basic-components/footprints/resistor-0805.json"),
    include_str!("../../../../libraries/basic-components/footprints/resistor-1206.json"),
    include_str!("../../../../libraries/basic-components/footprints/soic8.json"),
    include_str!("../../../../libraries/basic-components/footprints/sot563.json"),
    include_str!("../../../../libraries/basic-components/footprints/sot323.json"),
    include_str!("../../../../libraries/basic-components/footprints/sod123.json"),
    include_str!("../../../../libraries/basic-components/footprints/header-1x03-254.json"),
    include_str!("../../../../libraries/basic-components/footprints/header-1x04-254.json"),
    include_str!("../../../../libraries/basic-components/footprints/header-1x06-254.json"),
    include_str!("../../../../libraries/basic-components/footprints/header-1x08-254.json"),
    include_str!("../../../../libraries/basic-components/footprints/header-2x03-254.json"),
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

/// A circular, non-plated through hole in local footprint coordinates. It has no
/// copper land, pad number or terminal. Its diameter is the finished opening.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CircularNpth {
    pub center: Point,
    pub diameter_nm: i64,
}

impl CircularNpth {
    /// Bound the centre and finished diameter to the same envelope as existing footprint geometry.
    fn valid(self) -> bool {
        rectangle(self.center, self.diameter_nm, self.diameter_nm, 0)
    }
}

/// Absence is the only encoding for an empty list. In particular, older formats
/// must not accept a `non_plated_holes` field disguised as an empty array.
fn nonempty_mechanical_holes<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<CircularNpth>, D::Error> {
    let holes = Vec::<CircularNpth>::deserialize(deserializer)?;
    if !(1..=MAX_MECHANICAL_HOLES).contains(&holes.len()) {
        return Err(serde::de::Error::custom("expected 1–128 non-plated holes"));
    }
    Ok(holes)
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
    /// Authored mechanical openings, separate from the electrical pad namespace.
    #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "nonempty_mechanical_holes")]
    pub non_plated_holes: Vec<CircularNpth>,
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
        self.reference_paths_with(text, style, offset, None)
    }

    /// Apply local print overrides at the reference anchor before footprint mirroring and rotation.
    /// Visibility affects only generated ink; component identity and the physical pads remain separate.
    pub(crate) fn reference_paths_with(
        &self,
        text: &str,
        style: super::silkscreen::Settings,
        offset: Point,
        appearance: Option<&super::reference::Text>,
    ) -> Vec<Vec<Point>> {
        let appearance = super::reference::Resolved::new(Some(self), text, style, appearance);
        if !appearance.visible {
            return Vec::new();
        }
        let outline_stroke_nm = style.stroke_width_nm;
        let style =
            super::silkscreen::Settings { text_height_nm: appearance.size.height_nm, stroke_width_nm: appearance.size.stroke_width_nm, ..style };
        if !super::valid_reference_offset(offset) {
            return Vec::new();
        }
        let default = self.reference_offset.unwrap_or(super::DEFAULT_REFERENCE_OFFSET);
        if !super::valid_reference_offset(default) {
            return Vec::new();
        }
        let top_pad = self.pads.iter().map(|p| p.center.y_nm - p.width_nm.max(p.height_nm) / 2).min().unwrap_or(0);
        let top_line = self.silkscreen.iter().flat_map(|path| &path.points).map(|p| p.y_nm - outline_stroke_nm / 2).min().unwrap_or(top_pad);
        let lettering = if appearance.font == "pcbcraft-technical-1" { super::silkscreen::lettering } else { super::silkscreen::legacy_lettering };
        let mut paths = lettering(&appearance.text, 0, style);
        let bottom = paths.iter().flatten().map(|p| p.y_nm).max().unwrap_or(0) + (style.stroke_width_nm + 1) / 2;
        let align = match appearance.align {
            TextAlign::Start => paths.iter().flatten().map(|p| p.x_nm).min().unwrap_or(0) - style.stroke_width_nm / 2,
            TextAlign::Middle => 0,
            TextAlign::End => paths.iter().flatten().map(|p| p.x_nm).max().unwrap_or(0) + (style.stroke_width_nm + 1) / 2,
        };
        let y = top_pad.min(top_line) - style.pad_clearance_nm;
        for point in paths.iter_mut().flatten() {
            let (x, local_y) = super::rotate(point.x_nm - align, point.y_nm - bottom, appearance.rotation);
            point.x_nm = x + default.x_nm + offset.x_nm;
            point.y_nm = local_y + default.y_nm + y + offset.y_nm;
        }
        super::silkscreen::clipped(paths, &self.pads, style)
    }

    /// Return the exact identity used by placements and pinned project copies.
    pub(crate) fn key(&self) -> Key {
        (self.library.clone(), self.id.clone(), self.revision)
    }

    /// Select the native format from authored features, leaving untouched legacy footprints unchanged.
    pub(crate) fn format_for(has_npth: bool) -> &'static str {
        if has_npth { NPTH_FORMAT } else { LEGACY_FORMAT }
    }

    /// Refuse unknown versions and mixed feature/version pairs before writing or accepting a definition.
    fn validate_format(&self) -> Result<(), String> {
        match self.format.as_str() {
            LEGACY_FORMAT | NPTH_FORMAT if self.format == Self::format_for(!self.non_plated_holes.is_empty()) => Ok(()),
            LEGACY_FORMAT | NPTH_FORMAT => Err("footprint format does not match its non-plated hole content".into()),
            _ => Err(format!("unsupported footprint format {}", self.format)),
        }
    }

    /// Validate the schema, provenance, bounded geometry and distinct electrical pad identifiers.
    /// Land-less circular NPTH never takes part in the electrical pad checks.
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.validate_mode(false)
    }
    /// A draft can temporarily have neither pads nor mechanical holes.
    pub(crate) fn validate_draft(&self) -> Result<(), String> {
        self.validate_mode(true)
    }
    /// Check shared constraints before accepting either an editable draft or a published package.
    /// Exceeding the pad or artwork limit is refused with a message that names the limit and the count.
    fn validate_mode(&self, draft: bool) -> Result<(), String> {
        if self.pads.len() > MAX_PADS {
            return Err(format!("a footprint has at most {MAX_PADS} pads, not {}", self.pads.len()));
        }
        if self.artwork.len() > MAX_ARTWORK {
            return Err(format!("a footprint has at most {MAX_ARTWORK} artwork objects, not {}", self.artwork.len()));
        }
        self.validate_format()?;
        if self.non_plated_holes.len() > MAX_MECHANICAL_HOLES || self.non_plated_holes.iter().any(|hole| !hole.valid()) {
            return Err("invalid circular non-plated hole count, centre or finished diameter".into());
        }
        let numbers: BTreeSet<_> = self.pads.iter().map(|pad| pad.number.as_str()).collect();
        if !valid_id(&self.library)
            || !valid_id(&self.id)
            || self.revision == 0
            || !text(&self.name, 200)
            || !text(&self.license, 200)
            || !text(&self.source.kind, 64)
            || !text(&self.source.reference, 2048)
            || !text(&self.source.locator, 1024)
            || (!draft && self.pads.is_empty() && self.non_plated_holes.is_empty())
            || numbers.len() != self.pads.len()
            || self.pads.iter().any(|p| crate::schematic::PinNumber::new(&p.number).is_none())
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

/// Emit the shared native bytes for a validated footprint dependency, checking the
/// version/feature pair without rerunning geometric and electrical validation on save.
pub(crate) fn write_saved(definition: &FootprintDefinition) -> Result<Vec<u8>, String> {
    definition.validate_format()?;
    let mut bytes = serde_json::to_vec_pretty(definition).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Resolve an original built-in definition. A project pins this copy once, rather than resolving
/// its placement geometry from the installed catalog on every load.
pub(crate) fn basic_components(id: &str) -> Option<Arc<FootprintDefinition>> {
    static CATALOG: std::sync::LazyLock<Vec<Option<Arc<FootprintDefinition>>>> =
        std::sync::LazyLock::new(|| BASIC_COMPONENTS.iter().map(|text| read_saved(text.as_bytes()).ok()).collect());
    BASIC_COMPONENT_IDS.iter().position(|name| *name == id).and_then(|index| CATALOG[index].clone())
}

#[cfg(test)]
mod tests;
