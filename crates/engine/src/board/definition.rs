//! Original native footprint definitions, pinned independently from symbol graphics and placements.
use crate::schematic::{Point, TextAlign};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};

/// Stable identity of an immutable library item revision; paths and labels are only locators.
pub(crate) type Key = (String, String, u32);
pub(crate) const MAX_OFFSET_NM: i64 = 100_000_000;
/// Most pads in one footprint: a full 32 x 32 ball grid array.
pub(crate) const MAX_PADS: usize = 1024;
/// Most parameterized artwork objects in one footprint.
pub(crate) const MAX_ARTWORK: usize = 256;
pub(crate) const LEGACY_FORMAT: &str = "pcbcraft-footprint/1";
pub(crate) const NPTH_FORMAT: &str = "pcbcraft-footprint/4";
const SLOT_FORMAT: &str = "pcbcraft-footprint/5";
/// Historical format for any supported mix of holes, without an orientation statement.
pub(crate) const MIXED_HOLES_FORMAT: &str = "pcbcraft-footprint/6";
/// Historical format 6 plus an optional assembly orientation statement.
pub(crate) const ORIENTATION_FORMAT: &str = "pcbcraft-footprint/7";
/// The sole write format: format 7 plus optional assembly label settings.
pub(crate) const CURRENT_FORMAT: &str = "pcbcraft-footprint/13";
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
/// side; plated through-hole lands bind to all copper and carry one circular or obround hole.
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
    /// Finished plated obround hole, anchored at the pad centre and independent of the land.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present_slot")]
    pub plated_slot: Option<super::slot::PlatedSlot>,
}

/// A slot field must contain actual slot geometry; omit the field for other pad types.
fn present_slot<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<super::slot::PlatedSlot>, D::Error> {
    super::slot::PlatedSlot::deserialize(deserializer).map(Some)
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

/// Optional defaults for the assembly label. The label's text is always the component's reference
/// designator; omitted settings resolve to the footprint body centre, no rotation and the default size.
/// It decodes only from an object, as does each of its records, never from a positional array.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AssemblyLabel {
    /// Local position of the centre of the lettering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Point>,
    /// Counterclockwise whole degrees, 0 through 359.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<super::reference::Size>,
}

/// The fields of an assembly label record, with the position and size records object-only.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssemblyLabelRecord {
    #[serde(default, deserialize_with = "present_object")]
    position: Option<Point>,
    #[serde(default, deserialize_with = "super::reference::present")]
    rotation: Option<u16>,
    #[serde(default, deserialize_with = "present_object")]
    size: Option<super::reference::Size>,
}

impl<'de> Deserialize<'de> for AssemblyLabel {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let AssemblyLabelRecord { position, rotation, size } = object(deserializer)?;
        Ok(Self { position, rotation, size })
    }
}

/// Decode a record only from an object: a derived decoder also reads a positional sequence.
fn object<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<T, D::Error> {
    struct Record<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Record<T> {
        type Value = T;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an object")
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    deserializer.deserialize_map(Record(std::marker::PhantomData))
}

/// A present optional record, which must be an object; `null` is refused as for other settings.
fn present_object<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> {
    object(deserializer).map(Some)
}

/// Assembly label settings with every default applied, in footprint-local coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedAssemblyLabel {
    pub position: Point,
    pub rotation: u16,
    pub size: super::reference::Size,
}

impl AssemblyLabel {
    /// The default size matches the default silkscreen reference.
    pub const DEFAULT_SIZE: super::reference::Size = super::reference::Size { height_nm: 1_000_000, stroke_width_nm: 150_000 };

    /// Refuse an empty record, which is written by omitting the field, and out-of-range settings.
    pub fn validate(&self) -> Result<(), String> {
        if self == &Self::default() {
            return Err("Omit empty assembly label settings to use the defaults.".into());
        }
        if self.position.is_some_and(|position| !super::valid_reference_offset(position)) {
            return Err("Assembly label position must stay within the reference offset limit.".into());
        }
        if self.rotation.is_some_and(|rotation| rotation >= 360) {
            return Err("Assembly label rotation must be from 0 through 359 degrees.".into());
        }
        if self.size.is_some_and(|size| !(1..=10_000_000).contains(&size.height_nm) || !(1..=size.height_nm / 2).contains(&size.stroke_width_nm)) {
            return Err("Assembly label height must be positive and at most 10 mm, with stroke width at most half the height.".into());
        }
        Ok(())
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
    /// Optional assembly label defaults; only format 13 carries them.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "super::reference::present")]
    pub assembly_label: Option<AssemblyLabel>,
    /// Optional assembly orientation statement. Only formats 7 and 13 can carry it; earlier formats
    /// read without one keep their format and bytes.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "orientation_object")]
    pub orientation: Option<Orientation>,
}

/// How a package is meant to sit on the board for assembly. `polarized` says that turning the
/// part by a half turn changes its function, `pin_1` names the pad an assembler aligns with the
/// part's pin 1 mark, and `zero` states that the drawn rotation is the IPC-7351 zero component
/// orientation (IPC-7351, section 16).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Orientation {
    pub polarized: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "super::reference::present")]
    pub pin_1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "zero_orientation")]
    pub zero: Option<ZeroOrientation>,
}

/// Accept a present orientation only as a map. A derived struct alone also accepts positional
/// arrays in JSON; the named-field footprint contract does not.
fn orientation_object<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Orientation>, D::Error> {
    struct Object;
    impl<'de> serde::de::Visitor<'de> for Object {
        type Value = Orientation;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("an orientation object")
        }

        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<Orientation, M::Error> {
            Orientation::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    deserializer.deserialize_map(Object).map(Some)
}

/// The zero-orientation conventions a definition can state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ZeroOrientation {
    #[serde(rename = "ipc-7351")]
    Ipc7351,
}

/// Read the supported convention as a string, not as an externally tagged enum object.
fn zero_orientation<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<ZeroOrientation>, D::Error> {
    let name = String::deserialize(deserializer)?;
    match name.as_str() {
        "ipc-7351" => Ok(Some(ZeroOrientation::Ipc7351)),
        _ => Err(serde::de::Error::unknown_variant(&name, &["ipc-7351"])),
    }
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
    /// Require a supported land and exactly one enclosed hole on each plated through-hole pad.
    fn valid(&self) -> bool {
        rectangle(self.center, self.width_nm, self.height_nm, self.rotation)
            && match self.shape {
                PadShape::Rectangle => self.corner_radius_nm == 0,
                PadShape::RoundedRectangle => (1..=self.width_nm.min(self.height_nm) / 2).contains(&self.corner_radius_nm),
                PadShape::Oval => self.corner_radius_nm == 0,
                PadShape::Circle => self.width_nm == self.height_nm && self.corner_radius_nm == 0,
            }
            && match (self.copper, self.drill_nm, self.plated_slot) {
                (CopperBinding::ComponentSide, None, None) => true,
                (CopperBinding::AllCopper, Some(diameter), None) => diameter > 0 && diameter < self.width_nm.min(self.height_nm),
                (CopperBinding::AllCopper, None, Some(slot)) => self.encloses_slot(slot),
                _ => false,
            }
    }

    /// Copper must enclose the complete opening, not only its width. Positive but small
    /// rings are permitted here and reported by the board's annular-ring rule.
    fn encloses_slot(&self, slot: super::slot::PlatedSlot) -> bool {
        let land =
            super::geometry::Contour::pad(self.center, self.local_corners(), self.shape, self.corner_radius_nm, (self.width_nm, self.height_nm));
        slot.resolve(self.center, 0, false).is_some_and(|hole| hole.annular_ring_nm(&land) > 0.)
    }

    /// Resolve the land's bounding corners in footprint coordinates. Use on validated pads.
    pub fn local_corners(&self) -> [Point; 4] {
        let (left, top) = (-self.width_nm / 2, -self.height_nm / 2);
        let (right, bottom) = (left + self.width_nm, top + self.height_nm);
        [(left, top), (right, top), (right, bottom), (left, bottom)].map(|(x, y)| {
            let (x_nm, y_nm) = super::rotate(x, y, self.rotation);
            Point { x_nm: x_nm + self.center.x_nm, y_nm: y_nm + self.center.y_nm }
        })
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

    /// The centre of the footprint body: the bounding box of its fabrication graphics, or of its
    /// pads and holes when it has none, or the origin of an empty draft. Pads are bounded by
    /// their copper contour, so rounded ends and corners of rotated pads are taken into account.
    pub fn body_centre(&self) -> Point {
        let corners = |center: Point, width: i64, height: i64, rotation: u16| {
            let (sin, cos) = f64::from(rotation).to_radians().sin_cos();
            [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)].map(|(rx, ry)| {
                let (x, y) = (rx * width as f64, ry * height as f64);
                Point { x_nm: center.x_nm + (x * cos + y * sin).round() as i64, y_nm: center.y_nm + (y * cos - x * sin).round() as i64 }
            })
        };
        let body: Vec<Point> = self
            .graphics
            .iter()
            .filter(|graphic| graphic.purpose == GraphicPurpose::Fabrication)
            .flat_map(|graphic| corners(graphic.center, graphic.width_nm, graphic.height_nm, graphic.rotation))
            .chain(
                self.artwork
                    .iter()
                    .filter(|artwork| artwork.purpose == GraphicPurpose::Fabrication)
                    .flat_map(|artwork| artwork.graphic.paths().into_iter().flatten()),
            )
            .collect();
        let body = if body.is_empty() {
            self.pads
                .iter()
                .flat_map(|pad| {
                    let contour = super::geometry::Contour::pad(
                        pad.center,
                        pad.local_corners(),
                        pad.shape,
                        pad.corner_radius_nm,
                        (pad.width_nm, pad.height_nm),
                    );
                    let [left, top, right, bottom] = contour.bounds().map(|bound| bound.round() as i64);
                    [Point { x_nm: left, y_nm: top }, Point { x_nm: right, y_nm: bottom }]
                })
                .chain(self.non_plated_holes.iter().flat_map(|hole| corners(hole.center, hole.diameter_nm, hole.diameter_nm, 0)))
                .collect()
        } else {
            body
        };
        let (Some(left), Some(right)) = (body.iter().map(|p| p.x_nm).min(), body.iter().map(|p| p.x_nm).max()) else {
            return Point { x_nm: 0, y_nm: 0 };
        };
        let (Some(top), Some(bottom)) = (body.iter().map(|p| p.y_nm).min(), body.iter().map(|p| p.y_nm).max()) else {
            return Point { x_nm: 0, y_nm: 0 };
        };
        Point { x_nm: left + (right - left) / 2, y_nm: top + (bottom - top) / 2 }
    }

    /// Apply the defaults to the stored assembly label settings.
    pub fn assembly_label(&self) -> ResolvedAssemblyLabel {
        let label = self.assembly_label.unwrap_or_default();
        ResolvedAssemblyLabel {
            position: label.position.unwrap_or_else(|| self.body_centre()),
            rotation: label.rotation.unwrap_or(0),
            size: label.size.unwrap_or(AssemblyLabel::DEFAULT_SIZE),
        }
    }

    /// Resolve the assembly label strokes for a reference designator in local coordinates: the
    /// lettering is centred on the label position, then rotated about it. It uses the definition's
    /// reference font and is not clipped by pads.
    pub fn assembly_label_paths(&self, text: &str) -> Vec<Vec<Point>> {
        let label = self.assembly_label();
        if text.is_empty() || self.assembly_label.is_some_and(|label| label.validate().is_err()) {
            return Vec::new();
        }
        let style =
            super::silkscreen::Settings { text_height_nm: label.size.height_nm, stroke_width_nm: label.size.stroke_width_nm, pad_clearance_nm: 0 };
        let lettering =
            if self.reference.as_deref() == Some("pcbcraft-stroke-1") { super::silkscreen::legacy_lettering } else { super::silkscreen::lettering };
        let mut paths = lettering(text, 0, style);
        let points = || paths.iter().flatten();
        let (Some(left), Some(right)) = (points().map(|p| p.x_nm).min(), points().map(|p| p.x_nm).max()) else { return Vec::new() };
        let (Some(top), Some(bottom)) = (points().map(|p| p.y_nm).min(), points().map(|p| p.y_nm).max()) else { return Vec::new() };
        let (cx, cy) = (left + (right - left) / 2, top + (bottom - top) / 2);
        // Quarter turns stay exact; 90 degrees maps local (x, y) to (y, -x).
        let (sin, cos) = match label.rotation {
            0 => (0.0, 1.0),
            90 => (1.0, 0.0),
            180 => (0.0, -1.0),
            270 => (-1.0, 0.0),
            degrees => f64::from(degrees).to_radians().sin_cos(),
        };
        for point in paths.iter_mut().flatten() {
            let (x, y) = ((point.x_nm - cx) as f64, (point.y_nm - cy) as f64);
            point.x_nm = label.position.x_nm + (x * cos + y * sin).round() as i64;
            point.y_nm = label.position.y_nm + (y * cos - x * sin).round() as i64;
        }
        paths
    }

    /// Return the exact identity used by placements and pinned project copies.
    pub(crate) fn key(&self) -> Key {
        (self.library.clone(), self.id.clone(), self.revision)
    }

    /// Report actual slot geometry for fabrication and clipboard feature checks.
    pub(crate) fn has_plated_slots(&self) -> bool {
        self.pads.iter().any(|pad| pad.plated_slot.is_some())
    }

    /// Keep historical feature requirements; formats 6, 7 and 13 allow any supported hole mix.
    /// Only formats 7 and 13 can carry orientation, and only format 13 carries label settings.
    fn validate_format(&self) -> Result<(), String> {
        if self.assembly_label.is_some()
            && matches!(self.format.as_str(), LEGACY_FORMAT | NPTH_FORMAT | SLOT_FORMAT | MIXED_HOLES_FORMAT | ORIENTATION_FORMAT)
        {
            return Err("assembly label settings require footprint format 13".into());
        }
        let matches_content = match self.format.as_str() {
            LEGACY_FORMAT | NPTH_FORMAT | SLOT_FORMAT | MIXED_HOLES_FORMAT if self.orientation.is_some() => {
                return Err(format!(
                    "footprint format must be {ORIENTATION_FORMAT} or {CURRENT_FORMAT} for a definition with an orientation statement"
                ));
            }
            MIXED_HOLES_FORMAT | ORIENTATION_FORMAT | CURRENT_FORMAT => true,
            LEGACY_FORMAT => self.non_plated_holes.is_empty() && !self.has_plated_slots(),
            NPTH_FORMAT => !self.non_plated_holes.is_empty() && !self.has_plated_slots(),
            SLOT_FORMAT => self.has_plated_slots(),
            _ => return Err(format!("unsupported footprint format {}", self.format)),
        };
        if !matches_content {
            return Err("footprint format does not match its authored hole content".into());
        }
        Ok(())
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
    /// A format that does not match the authored features and a `pin_1` naming no pad are
    /// also refused with their own reasons.
    fn validate_mode(&self, draft: bool) -> Result<(), String> {
        if self.pads.len() > MAX_PADS {
            return Err(format!("a footprint has at most {MAX_PADS} pads, not {}", self.pads.len()));
        }
        if self.artwork.len() > MAX_ARTWORK {
            return Err(format!("a footprint has at most {MAX_ARTWORK} artwork objects, not {}", self.artwork.len()));
        }
        self.validate_format()?;
        if let Some(label) = &self.assembly_label {
            label.validate()?;
        }
        if self.non_plated_holes.len() > MAX_MECHANICAL_HOLES || self.non_plated_holes.iter().any(|hole| !hole.valid()) {
            return Err("invalid circular non-plated hole count, centre or finished diameter".into());
        }
        let numbers: BTreeSet<_> = self.pads.iter().map(|pad| pad.number.as_str()).collect();
        if self.orientation.as_ref().and_then(|o| o.pin_1.as_deref()).is_some_and(|pin| !numbers.contains(pin)) {
            return Err("orientation pin_1 names no pad of this footprint".into());
        }
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
