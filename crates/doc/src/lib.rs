//! Canonical project data. No UI or protocol dependencies.
use pcbcraft_geom::{Point, positive};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub type Id = u64;
pub const MAX_ITEMS: usize = 2048;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Pin {
    pub number: String,
    pub name: String,
    pub offset: Point,
    pub net: Option<Id>,
    #[serde(default = "passive")]
    pub electrical_type: String,
    #[serde(default)]
    pub direction: Option<f64>,
    #[serde(default = "first_unit")]
    pub unit: u32,
}
fn first_unit() -> u32 {
    1
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SymbolUnitPlacement {
    pub unit: u32,
    pub position: Point,
    pub rotation: f64,
}
fn passive() -> String {
    "passive".into()
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LibrarySymbol {
    pub library: String,
    pub unit: u32,
    pub definition: pcbcraft_library::SymbolDefinition,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LibraryFootprint {
    pub library: String,
    pub definition: pcbcraft_library::FootprintDefinition,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Component {
    pub id: Id,
    pub reference: String,
    pub value: String,
    pub kind: String,
    pub position: Point,
    pub rotation: f64,
    pub pins: Vec<Pin>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<SymbolUnitPlacement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_symbol: Option<LibrarySymbol>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_footprint: Option<LibraryFootprint>,
}
impl Component {
    pub fn unit_placement(&self, unit: u32) -> Option<(Point, f64)> {
        if unit == self.library_symbol.as_ref().map(|s| s.unit).unwrap_or(1) {
            Some((self.position, self.rotation))
        } else {
            self.units
                .iter()
                .find(|u| u.unit == unit)
                .map(|u| (u.position, u.rotation))
        }
    }
    /// Rendering views share the package ID and physical pin/net identity.
    pub fn schematic_views(&self) -> Vec<Component> {
        let mut views = vec![];
        for unit in std::iter::once(self.library_symbol.as_ref().map(|s| s.unit).unwrap_or(1))
            .chain(self.units.iter().map(|u| u.unit))
        {
            if let Some((position, rotation)) = self.unit_placement(unit) {
                let mut view = self.clone();
                view.position = position;
                view.rotation = rotation;
                view.pins.retain(|p| p.unit == unit);
                view.units.clear();
                if let Some(s) = &mut view.library_symbol {
                    s.unit = unit;
                }
                if !self.units.is_empty() {
                    let suffix = if unit <= 26 {
                        char::from_u32(64 + unit)
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| unit.to_string())
                    } else {
                        unit.to_string()
                    };
                    view.reference = format!("{}{suffix}", self.reference);
                }
                views.push(view);
            }
        }
        views
    }
    /// Local drawing extents used for selection and labels, independent of the renderer.
    pub fn drawing_points(&self) -> Vec<Point> {
        let unit = self.library_symbol.as_ref().map(|s| s.unit).unwrap_or(1);
        let mut points: Vec<Point> = self
            .pins
            .iter()
            .filter(|p| p.unit == unit)
            .map(|p| p.offset)
            .collect();
        if let Some(library) = &self.library_symbol {
            for graphic in library
                .definition
                .graphics
                .iter()
                .filter(|g| (g.unit == 0 || g.unit == unit) && g.body_style <= 1)
            {
                points.extend(graphic.primitive.path_points());
            }
        } else {
            points.extend([Point::new(-3.0, -2.0), Point::new(3.0, 2.0)]);
        }
        points
    }
    pub fn pin_position(&self, number: &str) -> Option<Point> {
        self.pins.iter().find(|p| p.number == number).and_then(|p| {
            self.unit_placement(p.unit)
                .map(|(position, rotation)| position.offset(p.offset.rotate(-rotation)))
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Net {
    pub id: Id,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PinRef {
    pub component: Id,
    pub pin: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Wire {
    pub id: Id,
    pub net: Id,
    pub from: PinRef,
    pub to: PinRef,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Pad {
    pub number: String,
    pub offset: Point,
    pub diameter: f64,
    pub net: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<pcbcraft_library::FootprintPad>,
}
impl Pad {
    pub fn drill_outline_points(&self) -> Vec<Point> {
        let Some(g) = &self.geometry else {
            return vec![];
        };
        let Some(drill) = &g.drill else { return vec![] };
        let mut hole = g.clone();
        hole.size = drill.size;
        hole.shape = "oval".into();
        hole.roundrect_ratio = None;
        hole.position = self.offset.offset(drill.offset.rotate(-g.rotation));
        Pad {
            number: String::new(),
            offset: hole.position,
            diameter: drill.size.x.min(drill.size.y),
            net: None,
            geometry: Some(hole),
        }
        .outline_points()
    }
    /// Maximum chord approximation error of the rounded copper outline.
    pub fn outline_error(&self) -> f64 {
        self.geometry
            .as_ref()
            .map(|p| {
                let radius = match p.shape.as_str() {
                    "oval" => p.size.x.min(p.size.y) / 2.0,
                    "roundrect" => p.size.x.min(p.size.y) * p.roundrect_ratio.unwrap_or(0.0),
                    _ => 0.0,
                };
                radius * (1.0 - (std::f64::consts::PI / 64.0).cos())
            })
            .unwrap_or(0.0)
    }
    /// Closed convex copper outline in footprint-local coordinates.
    pub fn outline_points(&self) -> Vec<Point> {
        let (size, rotation, shape, ratio) = self
            .geometry
            .as_ref()
            .map(|p| {
                (
                    p.size,
                    p.rotation,
                    p.shape.as_str(),
                    p.roundrect_ratio.unwrap_or(0.0),
                )
            })
            .unwrap_or((Point::new(self.diameter, self.diameter), 0.0, "circle", 0.0));
        let hx = size.x / 2.0;
        let hy = size.y / 2.0;
        let radius = match shape {
            "circle" | "oval" => hx.min(hy),
            "roundrect" => size.x.min(size.y) * ratio,
            _ => 0.0,
        };
        let mut points = vec![];
        if radius <= 1e-9 {
            points.extend([
                Point::new(-hx, -hy),
                Point::new(hx, -hy),
                Point::new(hx, hy),
                Point::new(-hx, hy),
            ]);
        } else {
            for (x, y, start) in [
                (hx - radius, hy - radius, 0.0),
                (-hx + radius, hy - radius, 90.0),
                (-hx + radius, -hy + radius, 180.0),
                (hx - radius, -hy + radius, 270.0),
            ] {
                for i in 0..=16 {
                    let angle = (start + i as f64 * 90.0 / 16.0).to_radians();
                    points.push(Point::new(
                        x + radius * angle.cos(),
                        y + radius * angle.sin(),
                    ));
                }
            }
        }
        if let Some(first) = points.first().copied() {
            points.push(first);
        }
        points
            .into_iter()
            .map(|p| self.offset.offset(p.rotate(-rotation)))
            .collect()
    }
    pub fn bounding_radius(&self) -> f64 {
        self.geometry
            .as_ref()
            .map(|p| {
                if p.shape == "circle" {
                    p.size.x / 2.0
                } else {
                    p.size.x.hypot(p.size.y) / 2.0
                }
            })
            .unwrap_or(self.diameter / 2.0)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Footprint {
    pub id: Id,
    pub component: Id,
    pub position: Point,
    pub rotation: f64,
    pub pads: Vec<Pad>,
}
impl Footprint {
    pub fn drawing_points(&self, project: &Project) -> Vec<Point> {
        let mut points: Vec<Point> = self.pads.iter().flat_map(|p| p.outline_points()).collect();
        if let Some(library) = project
            .component(self.component)
            .and_then(|c| c.library_footprint.as_ref())
        {
            for graphic in &library.definition.graphics {
                points.extend(graphic.path_points());
            }
        } else {
            points.extend([Point::new(-3.75, -1.5), Point::new(3.75, 1.5)]);
        }
        points
    }
    pub fn pad_position(&self, number: &str) -> Option<Point> {
        self.pads
            .iter()
            .find(|p| p.number == number)
            .map(|p| self.position.offset(p.offset.rotate(-self.rotation)))
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Layer {
    #[serde(rename = "F.Cu")]
    Front,
    #[serde(rename = "B.Cu")]
    Back,
}
impl Layer {
    pub fn name(self) -> &'static str {
        match self {
            Self::Front => "F.Cu",
            Self::Back => "B.Cu",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Track {
    pub id: Id,
    pub net: Id,
    pub start: Point,
    pub end: Point,
    pub width: f64,
    pub layer: Layer,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Cutout {
    pub id: Id,
    pub center: Point,
    pub radius: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Board {
    pub width: f64,
    pub height: f64,
    pub thickness: f64,
    pub clearance: f64,
    pub footprints: Vec<Footprint>,
    pub tracks: Vec<Track>,
    pub cutouts: Vec<Cutout>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub components: Vec<Component>,
    pub nets: Vec<Net>,
    pub wires: Vec<Wire>,
    pub board: Board,
    #[serde(default)]
    pub library: ProjectLibrary,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProjectLibrary {
    #[serde(default)]
    pub notices: Vec<String>,
    pub symbols: Vec<pcbcraft_library::SymbolDefinition>,
    pub footprints: Vec<pcbcraft_library::FootprintDefinition>,
}
impl ProjectLibrary {
    pub fn validate(&self) -> Result<(), String> {
        if self.notices.len() > 256 || self.notices.iter().any(|s| s.len() > 128 * 1024) {
            return Err("library provenance notice limit exceeded".into());
        }
        if self.symbols.len() > 256 || self.footprints.len() > 256 {
            return Err("project library supports at most 256 symbols and 256 footprints".into());
        }
        let mut names = BTreeSet::new();
        for s in &self.symbols {
            s.validate().map_err(|e| e.to_string())?;
            if !names.insert(&s.name) {
                return Err("duplicate library symbol name".into());
            }
        }
        names.clear();
        for f in &self.footprints {
            f.validate().map_err(|e| e.to_string())?;
            if !names.insert(&f.name) {
                return Err("duplicate library footprint name".into());
            }
        }
        Ok(())
    }
}
impl Default for Project {
    fn default() -> Self {
        Self {
            library: ProjectLibrary::default(),
            format: "pcbcraft".into(),
            version: 1,
            name: "Untitled circuit".into(),
            components: vec![],
            nets: vec![],
            wires: vec![],
            board: Board {
                width: 60.0,
                height: 40.0,
                thickness: 1.6,
                clearance: 0.2,
                footprints: vec![],
                tracks: vec![],
                cutouts: vec![],
            },
        }
    }
}

pub struct Template {
    pub kind: &'static str,
    pub label: &'static str,
    pub prefix: &'static str,
    pub value: &'static str,
}
pub const TEMPLATES: &[Template] = &[
    Template {
        kind: "resistor",
        label: "Resistor",
        prefix: "R",
        value: "1k",
    },
    Template {
        kind: "led",
        label: "LED",
        prefix: "D",
        value: "LED",
    },
    Template {
        kind: "capacitor",
        label: "Capacitor",
        prefix: "C",
        value: "100nF",
    },
    Template {
        kind: "connector",
        label: "2-pin connector",
        prefix: "J",
        value: "Power",
    },
];
pub fn component_template(
    kind: &str,
    id: Id,
    reference: String,
    position: Point,
) -> Option<Component> {
    let t = TEMPLATES.iter().find(|t| t.kind == kind)?;
    let names = if kind == "led" {
        ["A", "K"]
    } else if kind == "connector" {
        ["+", "−"]
    } else {
        ["1", "2"]
    };
    Some(Component {
        units: vec![],
        id,
        reference,
        value: t.value.into(),
        kind: kind.into(),
        position,
        rotation: 0.0,
        library_symbol: None,
        library_footprint: None,
        pins: names
            .iter()
            .enumerate()
            .map(|(i, n)| Pin {
                number: (i + 1).to_string(),
                name: (*n).into(),
                offset: Point::new(if i == 0 { -5.0 } else { 5.0 }, 0.0),
                net: None,
                electrical_type: passive(),
                direction: None,
                unit: 1,
            })
            .collect(),
    })
}
impl Project {
    pub fn component(&self, id: Id) -> Option<&Component> {
        self.components.iter().find(|c| c.id == id)
    }
    pub fn footprint(&self, id: Id) -> Option<&Footprint> {
        self.board.footprints.iter().find(|f| f.id == id)
    }
    pub fn pin(&self, r: &PinRef) -> Option<&Pin> {
        self.component(r.component)?
            .pins
            .iter()
            .find(|p| p.number == r.pin)
    }
    pub fn pin_position(&self, r: &PinRef) -> Option<Point> {
        self.component(r.component)?.pin_position(&r.pin)
    }
    /// Presentation-only orthogonal path that leaves pins in their outward direction.
    /// Wire crossings never implicitly connect nets; logical connectivity is explicit.
    pub fn wire_path(&self, w: &Wire) -> Option<Vec<Point>> {
        let a = self.pin_position(&w.from)?;
        let b = self.pin_position(&w.to)?;
        let direction = |r: &PinRef| -> Option<Point> {
            let pin = self.pin(r)?;
            let local = if let Some(angle) = pin.direction {
                Point::new(-angle.to_radians().cos(), angle.to_radians().sin())
            } else {
                pin.offset
            };
            Some(local.rotate(-self.component(r.component)?.unit_placement(pin.unit)?.1))
        };
        let da = direction(&w.from)?;
        let db = direction(&w.to)?;
        Some(orthogonal_path(a, b, da, db))
    }
    pub fn net_name(&self, id: Id) -> &str {
        self.nets
            .iter()
            .find(|n| n.id == id)
            .map(|n| n.name.as_str())
            .unwrap_or("unassigned")
    }
    pub fn ids(&self) -> impl Iterator<Item = Id> + '_ {
        self.components
            .iter()
            .map(|x| x.id)
            .chain(self.nets.iter().map(|x| x.id))
            .chain(self.wires.iter().map(|x| x.id))
            .chain(self.board.footprints.iter().map(|x| x.id))
            .chain(self.board.tracks.iter().map(|x| x.id))
            .chain(self.board.cutouts.iter().map(|x| x.id))
    }
    pub fn validate(&self) -> Result<(), String> {
        self.library.validate()?;
        if self
            .components
            .iter()
            .map(|c| 1 + c.units.len())
            .sum::<usize>()
            > MAX_ITEMS
        {
            return Err("schematic unit limit reached (2048)".into());
        }
        if self.format != "pcbcraft" || self.version != 1 {
            return Err("unsupported project format/version".into());
        }
        if self.name.is_empty() || self.name.len() > 256 {
            return Err("project name must be 1–256 bytes".into());
        }
        if self.ids().count() > MAX_ITEMS {
            return Err("project object limit reached (2048)".into());
        }
        let mut ids = BTreeSet::new();
        for id in self.ids() {
            if id == 0 || id > u32::MAX as u64 || !ids.insert(id) {
                return Err("duplicate or out-of-range object ID".into());
            }
        }
        if self.board.tracks.len()
            + self
                .board
                .footprints
                .iter()
                .map(|f| f.pads.len())
                .sum::<usize>()
            > 4096
        {
            return Err("copper item limit reached (4096)".into());
        }
        let nets: BTreeSet<_> = self.nets.iter().map(|n| n.id).collect();
        let mut names = BTreeSet::new();
        for n in &self.nets {
            if n.name.is_empty() || n.name.len() > 128 || !names.insert(&n.name) {
                return Err("invalid or duplicate net name".into());
            }
        }
        let mut refs = BTreeSet::new();
        for c in &self.components {
            if !c.position.valid()
                || !c.rotation.is_finite()
                || c.rotation.abs() > 360_000.0
                || !refs.insert(&c.reference)
                || c.reference.is_empty()
                || c.reference.len() > 64
                || c.value.len() > 256
                || (c.library_symbol.is_none() && !TEMPLATES.iter().any(|t| t.kind == c.kind))
            {
                return Err("invalid component".into());
            }
            if let Some(symbol) = &c.library_symbol {
                symbol.definition.validate().map_err(|e| e.to_string())?;
                if symbol.unit != 1
                    || c.kind != "library"
                    || symbol.library.len() > 256
                    || !symbol.definition.unsupported.is_empty()
                {
                    return Err("invalid or unsupported embedded library symbol".into());
                }
                let pins: Vec<_> = symbol
                    .definition
                    .pins
                    .iter()
                    .filter(|p| p.body_style <= 1)
                    .collect();
                if pins.len() != c.pins.len()
                    || pins.iter().any(|p| {
                        !c.pins.iter().any(|pin| {
                            pin.number == p.number
                                && pin.name == p.name
                                && pin.offset == p.position
                                && pin.direction == Some(p.angle)
                                && pin.electrical_type == p.electrical_type
                                && pin.unit == p.unit.max(1)
                        })
                    })
                {
                    return Err("component pins do not match embedded symbol definition".into());
                }
                let expected = symbol_units(&symbol.definition)?;
                let actual: BTreeSet<_> = std::iter::once(1)
                    .chain(c.units.iter().map(|u| u.unit))
                    .collect();
                if expected != actual
                    || c.units.len() + 1 != actual.len()
                    || c.units.iter().any(|u| {
                        u.unit <= 1
                            || !u.position.valid()
                            || !u.rotation.is_finite()
                            || u.rotation.abs() > 360_000.0
                    })
                {
                    return Err("invalid schematic unit placements".into());
                }
            } else if !c.units.is_empty() || c.pins.iter().any(|p| p.unit != 1) {
                return Err("starter symbols have one unit".into());
            }
            if let Some(footprint) = &c.library_footprint {
                footprint.definition.validate().map_err(|e| e.to_string())?;
                validate_footprint_support(&footprint.definition)?;
                if footprint.library.len() > 256 {
                    return Err("invalid footprint library name".into());
                }
                let pins: BTreeSet<_> = c.pins.iter().map(|p| p.number.as_str()).collect();
                let pads: BTreeSet<_> = footprint
                    .definition
                    .pads
                    .iter()
                    .map(|p| p.number.as_str())
                    .collect();
                if pins != pads {
                    return Err("embedded footprint pad numbers do not match symbol pins".into());
                }
            }
            if c.pins.is_empty() || c.pins.len() > 64 {
                return Err("invalid pin count".into());
            }
            let mut numbers = BTreeSet::new();
            for p in &c.pins {
                if p.number.is_empty()
                    || p.number.len() > 16
                    || p.name.len() > 64
                    || !numbers.insert(&p.number)
                    || !p.offset.valid()
                    || p.direction
                        .is_some_and(|angle| !angle.is_finite() || angle.abs() > 360_000.0)
                    || p.electrical_type.len() > 64
                    || p.net.is_some_and(|n| !nets.contains(&n))
                {
                    return Err("invalid pin or net reference".into());
                }
            }
        }
        for w in &self.wires {
            if !nets.contains(&w.net)
                || self.pin(&w.from).is_none_or(|p| p.net != Some(w.net))
                || self.pin(&w.to).is_none_or(|p| p.net != Some(w.net))
            {
                return Err("wire has inconsistent pin/net references".into());
            }
        }
        let b = &self.board;
        if !positive(b.width)
            || !positive(b.height)
            || !positive(b.thickness)
            || !positive(b.clearance)
        {
            return Err("invalid board dimensions/rules".into());
        }
        let mut placed = BTreeSet::new();
        for f in &b.footprints {
            if self.component(f.component).is_none()
                || !placed.insert(f.component)
                || !f.position.valid()
                || !f.rotation.is_finite()
                || f.rotation.abs() > 360_000.0
                || f.pads.is_empty()
                || f.pads.len() > 64
            {
                return Err("invalid footprint".into());
            }
            let mut numbers = BTreeSet::new();
            if let Some(library) = self
                .component(f.component)
                .and_then(|c| c.library_footprint.as_ref())
            {
                if f.pads.len() != library.definition.pads.len()
                    || f.pads.iter().any(|p| {
                        p.geometry.is_none()
                            || (p.diameter
                                - p.geometry
                                    .as_ref()
                                    .map(|g| g.size.x.min(g.size.y))
                                    .unwrap_or(0.0))
                            .abs()
                                > 1e-6
                    })
                {
                    return Err("placed footprint pads must retain embedded geometry".into());
                }
            } else if self
                .component(f.component)
                .is_some_and(|c| c.library_symbol.is_some())
            {
                return Err(
                    "library symbol requires an assigned footprint before PCB placement".into(),
                );
            }
            for p in &f.pads {
                if !numbers.insert(&p.number)
                    || !p.offset.valid()
                    || !positive(p.diameter)
                    || p.net.is_some_and(|n| !nets.contains(&n))
                    || self.component(f.component).is_none_or(|c| {
                        !c.pins
                            .iter()
                            .any(|pin| pin.number == p.number && pin.net == p.net)
                    })
                {
                    return Err("invalid pad or stale pad net; sync the PCB".into());
                }
                if let Some(geometry) = &p.geometry {
                    if geometry.position != p.offset
                        || geometry.number != p.number
                        || !positive(geometry.size.x)
                        || !positive(geometry.size.y)
                    {
                        return Err("pad geometry does not match imported footprint".into());
                    }
                    let definition = self
                        .component(f.component)
                        .and_then(|c| c.library_footprint.as_ref())
                        .ok_or("pad geometry requires an embedded footprint definition")?;
                    if !definition
                        .definition
                        .pads
                        .iter()
                        .any(|source| source == geometry)
                    {
                        return Err("pad geometry is not present in embedded definition".into());
                    }
                }
            }
        }
        for t in &b.tracks {
            if !nets.contains(&t.net)
                || !t.start.valid()
                || !t.end.valid()
                || !positive(t.width)
                || t.start.distance(t.end) < 1e-6
            {
                return Err("invalid track geometry or net".into());
            }
        }
        for c in &b.cutouts {
            if !c.center.valid()
                || !positive(c.radius)
                || c.center.x - c.radius <= 0.0
                || c.center.y - c.radius <= 0.0
                || c.center.x + c.radius >= b.width
                || c.center.y + c.radius >= b.height
            {
                return Err("cutout must be strictly inside the board".into());
            }
        }
        Ok(())
    }
}

pub fn symbol_units(def: &pcbcraft_library::SymbolDefinition) -> Result<BTreeSet<u32>, String> {
    let units: BTreeSet<_> = def
        .pins
        .iter()
        .filter(|p| p.body_style <= 1)
        .map(|p| p.unit.max(1))
        .chain(
            def.graphics
                .iter()
                .filter(|g| g.body_style <= 1 && g.unit > 0)
                .map(|g| g.unit),
        )
        .collect();
    if !units.contains(&1) || units.len() > 32 || units.iter().any(|u| *u > 32) {
        return Err("placement supports 1–32 schematic units, including unit 1".into());
    }
    Ok(units)
}
pub fn library_component(
    definition: pcbcraft_library::SymbolDefinition,
    library: String,
    id: Id,
    reference: String,
    position: Point,
    value: Option<String>,
) -> Result<Component, String> {
    definition.validate().map_err(|e| e.to_string())?;
    if !definition.unsupported.is_empty() {
        return Err(format!(
            "unsupported symbol features: {}",
            definition.unsupported.join(", ")
        ));
    }
    let unit_ids = symbol_units(&definition)?;
    let mut numbers = BTreeSet::new();
    let pins: Vec<_> = definition
        .pins
        .iter()
        .filter(|p| p.body_style <= 1)
        .map(|p| Pin {
            number: p.number.clone(),
            name: p.name.clone(),
            offset: p.position,
            net: None,
            electrical_type: p.electrical_type.clone(),
            direction: Some(p.angle),
            unit: p.unit.max(1),
        })
        .collect();
    if pins.is_empty() || pins.len() > 64 || pins.iter().any(|p| !numbers.insert(&p.number)) {
        return Err("symbol placement requires 1–64 unique physical pin numbers across units; repeated common pins are not supported yet".into());
    }
    let extent = pins
        .iter()
        .map(|p| p.offset)
        .chain(
            definition
                .graphics
                .iter()
                .flat_map(|g| g.primitive.path_points()),
        )
        .fold(Point::new(7.0, 5.0), |m, p| {
            Point::new(m.x.max(p.x.abs()), m.y.max(p.y.abs()))
        });
    let units = unit_ids
        .into_iter()
        .filter(|u| *u > 1)
        .enumerate()
        .map(|(i, unit)| {
            let index = i + 1;
            SymbolUnitPlacement {
                unit,
                position: position.offset(Point::new(
                    (index % 3) as f64 * (extent.x * 2.0 + 10.0),
                    (index / 3) as f64 * (extent.y * 2.0 + 12.0),
                )),
                rotation: 0.0,
            }
        })
        .collect();
    Ok(Component {
        id,
        reference,
        value: value.unwrap_or_else(|| definition.value.clone()),
        kind: "library".into(),
        position,
        rotation: 0.0,
        pins,
        units,
        library_symbol: Some(LibrarySymbol {
            library,
            unit: 1,
            definition,
        }),
        library_footprint: None,
    })
}
pub fn validate_footprint_support(
    definition: &pcbcraft_library::FootprintDefinition,
) -> Result<(), String> {
    definition.validate().map_err(|e| e.to_string())?;
    if definition.graphics.iter().any(|g| match g {
        pcbcraft_library::Graphic::Line { layer, .. }
        | pcbcraft_library::Graphic::Rectangle { layer, .. }
        | pcbcraft_library::Graphic::Circle { layer, .. }
        | pcbcraft_library::Graphic::Arc { layer, .. }
        | pcbcraft_library::Graphic::Polyline { layer, .. }
        | pcbcraft_library::Graphic::Text { layer, .. } => layer == "Edge.Cuts",
    }) {
        return Err("footprint-contained Edge.Cuts require mechanical cut geometry support, which is planned".into());
    }
    if !definition.unsupported.is_empty() {
        return Err(format!(
            "footprint has unsupported primitives: {}",
            definition.unsupported.join(", ")
        ));
    }
    if definition.graphics.iter().any(|g| match g {
        pcbcraft_library::Graphic::Line { layer, .. }
        | pcbcraft_library::Graphic::Rectangle { layer, .. }
        | pcbcraft_library::Graphic::Circle { layer, .. }
        | pcbcraft_library::Graphic::Arc { layer, .. }
        | pcbcraft_library::Graphic::Polyline { layer, .. }
        | pcbcraft_library::Graphic::Text { layer, .. } => layer.ends_with(".Cu"),
    }) {
        return Err("footprint copper graphics require conductive geometry support; use numbered pads instead".into());
    }
    let mut numbers = BTreeSet::new();
    for p in &definition.pads {
        let copper_ok = if p.pad_type == "thru_hole" {
            p.layers.iter().any(|l| l == "*.Cu")
                || (p.layers.iter().any(|l| l == "F.Cu") && p.layers.iter().any(|l| l == "B.Cu"))
        } else {
            p.layers.iter().any(|l| l == "F.Cu")
                && !p.layers.iter().any(|l| l == "B.Cu" || l == "*.Cu")
        };
        let drill_ok = if p.pad_type == "thru_hole" {
            drill_inside_pad(p)
        } else {
            p.drill.is_none()
        };
        if !["smd", "thru_hole"].contains(&p.pad_type.as_str())
            || !["circle", "rect", "roundrect", "oval"].contains(&p.shape.as_str())
            || (p.shape == "circle" && (p.size.x - p.size.y).abs() > 1e-6)
            || !copper_ok
            || !drill_ok
            || p.number.is_empty()
            || !numbers.insert(&p.number)
        {
            return Err("placement supports unique numbered front SMD or plated through-hole circle/rect/roundrect/oval pads with a drill strictly inside the pad; custom pads, unnumbered mechanical holes and back SMD placement are planned".into());
        }
    }
    if definition.pads.is_empty() || definition.pads.len() > 64 {
        return Err("imported footprint must have 1–64 pads".into());
    }
    Ok(())
}

fn drill_inside_pad(p: &pcbcraft_library::FootprintPad) -> bool {
    let Some(drill) = &p.drill else { return false };
    // Standard circular/oval holes are capsules. A convex pad contains the
    // whole capsule iff it contains both endpoint discs of that capsule.
    let radius = drill.size.x.min(drill.size.y) / 2.0;
    let half_axis = Point::new(
        (drill.size.x / 2.0 - radius).max(0.0),
        (drill.size.y / 2.0 - radius).max(0.0),
    );
    let corner = match p.shape.as_str() {
        "circle" | "oval" => p.size.x.min(p.size.y) / 2.0,
        "roundrect" => p.size.x.min(p.size.y) * p.roundrect_ratio.unwrap_or(0.0),
        "rect" => 0.0,
        _ => return false,
    };
    let clearance = |point: Point| {
        let dx = point.x.abs() - (p.size.x / 2.0 - corner);
        let dy = point.y.abs() - (p.size.y / 2.0 - corner);
        -(dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - corner)
    };
    radius > 0.0
        && [
            drill.offset.offset(half_axis),
            drill.offset.offset(Point::new(-half_axis.x, -half_axis.y)),
        ]
        .into_iter()
        .all(|end| clearance(end) > radius + 1e-6)
}

fn orthogonal_path(a: Point, b: Point, da: Point, db: Point) -> Vec<Point> {
    let vertical_a = da.y.abs() > da.x.abs();
    let vertical_b = db.y.abs() > db.x.abs();
    if vertical_a != vertical_b {
        let outward = |direction: Point, vertical: bool| {
            if vertical {
                Point::new(0.0, if direction.y >= 0.0 { 3.0 } else { -3.0 })
            } else {
                Point::new(if direction.x >= 0.0 { 3.0 } else { -3.0 }, 0.0)
            }
        };
        let start = a.offset(outward(da, vertical_a));
        let end = b.offset(outward(db, vertical_b));
        let bend = if vertical_a {
            Point::new(end.x, start.y)
        } else {
            Point::new(start.x, end.y)
        };
        let mut points = vec![a, start, bend, end, b];
        points.dedup_by(|a, b| a.distance(*b) < 1e-9);
        return points;
    }
    if da.y.abs() > da.x.abs() && db.y.abs() > db.x.abs() {
        return orthogonal_path(
            Point::new(a.y, a.x),
            Point::new(b.y, b.x),
            Point::new(da.y, da.x),
            Point::new(db.y, db.x),
        )
        .into_iter()
        .map(|p| Point::new(p.y, p.x))
        .collect();
    }
    let sa = if da.x >= 0.0 { 1.0 } else { -1.0 };
    let sb = if db.x >= 0.0 { 1.0 } else { -1.0 };
    if sa == sb && (a.y - b.y).abs() > 1e-6 {
        let x = if sa < 0.0 {
            a.x.min(b.x) - 3.0
        } else {
            a.x.max(b.x) + 3.0
        };
        return vec![a, Point::new(x, a.y), Point::new(x, b.y), b];
    }
    if sa != sb && (b.x - a.x) * sa > 0.0 {
        let x = (a.x + b.x) / 2.0;
        return vec![a, Point::new(x, a.y), Point::new(x, b.y), b];
    }
    let start = Point::new(a.x + sa * 3.0, a.y);
    let end = Point::new(b.x + sb * 3.0, b.y);
    let y = a.y.max(b.y) + 8.0;
    vec![
        a,
        start,
        Point::new(start.x, y),
        Point::new(end.x, y),
        end,
        b,
    ]
}

#[cfg(test)]
mod drill_tests {
    use super::*;
    #[test]
    fn native_library_unknown_fields_are_rejected_instead_of_discarded() -> Result<(), String> {
        assert!(serde_json::from_value::<ProjectLibrary>(serde_json::json!({"symbols":[],"footprints":[],"license_metadata":"must not disappear"})).is_err());
        let definition = dip()?;
        let mut value = serde_json::to_value(&definition).map_err(|e| e.to_string())?;
        value["license_metadata"] = serde_json::json!("must not disappear");
        assert!(serde_json::from_value::<pcbcraft_library::FootprintDefinition>(value).is_err());
        for field in ["pad", "drill"] {
            let mut value = serde_json::to_value(&definition).map_err(|e| e.to_string())?;
            if field == "pad" {
                value["pads"][0]["unsupported_future_field"] = serde_json::json!(true);
            } else {
                value["pads"][0]["drill"]["unsupported_future_field"] = serde_json::json!(true);
            }
            assert!(
                serde_json::from_value::<pcbcraft_library::FootprintDefinition>(value).is_err()
            );
        }
        let symbol = pcbcraft_library::read_symbols(include_str!(
            "../../library/tests/fixtures/device.kicad_sym"
        ))
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.name == "R")
        .ok_or("missing resistor")?;
        for path in ["root", "pin", "graphic", "graphic-primitive", "point"] {
            let mut value = serde_json::to_value(&symbol).map_err(|e| e.to_string())?;
            match path {
                "root" => value["unsupported_future_field"] = serde_json::json!(true),
                "pin" => value["pins"][0]["unsupported_future_field"] = serde_json::json!(true),
                "point" => {
                    value["pins"][0]["position"]["unsupported_future_field"] =
                        serde_json::json!(true)
                }
                "graphic-primitive" => {
                    value["graphics"][0]["primitive"]["Rectangle"]["unsupported_future_field"] =
                        serde_json::json!(true)
                }
                _ => value["graphics"][0]["unsupported_future_field"] = serde_json::json!(true),
            };
            assert!(serde_json::from_value::<pcbcraft_library::SymbolDefinition>(value).is_err());
        }
        Ok(())
    }
    #[test]
    fn copper_graphics_cannot_bypass_pad_connectivity_checks() -> Result<(), String> {
        let mut definition = dip()?;
        validate_footprint_support(&definition)?;
        definition.graphics.push(pcbcraft_library::Graphic::Line {
            start: Point::default(),
            end: Point::new(7.62, 0.0),
            width: 0.3,
            layer: "F.Cu".into(),
        });
        assert!(
            validate_footprint_support(&definition).is_err_and(|e| e.contains("copper graphics"))
        );
        Ok(())
    }
    fn assert_outward(path: &[Point], a: Point, b: Point, da: Point, db: Point) {
        assert!(path.len() >= 3);
        assert_eq!(path.first(), Some(&a));
        assert_eq!(path.last(), Some(&b));
        let first = Point::new(path[1].x - a.x, path[1].y - a.y);
        let last = Point::new(path[path.len() - 2].x - b.x, path[path.len() - 2].y - b.y);
        assert!(
            first.x * da.x + first.y * da.y > 0.0,
            "first leg must leave its pin outward"
        );
        assert!(
            last.x * db.x + last.y * db.y > 0.0,
            "last leg must approach from the outward side"
        );
        assert!(path.windows(2).all(
            |pair| (pair[0].x - pair[1].x).abs() < 1e-8 || (pair[0].y - pair[1].y).abs() < 1e-8
        ));
    }
    #[test]
    fn mixed_pin_orientations_have_outward_stubs_at_both_ends() {
        let a = Point::default();
        let b = Point::new(10.0, 10.0);
        for vertical in [Point::new(0.0, -1.0), Point::new(0.0, 1.0)] {
            for horizontal in [Point::new(-1.0, 0.0), Point::new(1.0, 0.0)] {
                for (da, db) in [(vertical, horizontal), (horizontal, vertical)] {
                    assert_outward(&orthogonal_path(a, b, da, db), a, b, da, db);
                }
            }
        }
    }
    #[test]
    fn wire_path_uses_rotated_secondary_unit_pin_endpoints_and_directions() -> Result<(), String> {
        let definition = pcbcraft_library::read_symbols(include_str!(
            "../../library/tests/fixtures/lm358.kicad_sym"
        ))
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.name == "LM358")
        .ok_or("missing LM358")?;
        let mut c = library_component(
            definition,
            "Amplifier_Operational".into(),
            1,
            "U1".into(),
            Point::new(20.0, 20.0),
            None,
        )?;
        c.units
            .iter_mut()
            .find(|u| u.unit == 2)
            .ok_or("missing unit2")?
            .rotation = 90.0;
        for pin in &mut c.pins {
            if ["5", "1"].contains(&pin.number.as_str()) {
                pin.net = Some(2);
            }
        }
        let mut project = Project::default();
        project.components.push(c);
        project.nets.push(Net {
            id: 2,
            name: "signal".into(),
        });
        let wire = Wire {
            id: 3,
            net: 2,
            from: PinRef {
                component: 1,
                pin: "5".into(),
            },
            to: PinRef {
                component: 1,
                pin: "1".into(),
            },
        };
        project.wires.push(wire.clone());
        project.validate()?;
        let a = project
            .pin_position(&wire.from)
            .ok_or("missing first point")?;
        let b = project
            .pin_position(&wire.to)
            .ok_or("missing second point")?;
        assert_outward(
            &project.wire_path(&wire).ok_or("missing path")?,
            a,
            b,
            Point::new(0.0, 1.0),
            Point::new(1.0, 0.0),
        );
        Ok(())
    }
    fn dip() -> Result<pcbcraft_library::FootprintDefinition, String> {
        pcbcraft_library::read_footprint(include_str!(
            "../../library/tests/fixtures/DIP-8_W7.62mm.kicad_mod"
        ))
        .map_err(|e| e.to_string())
    }
    #[test]
    fn real_dip_and_valid_offset_slots_fit_actual_pad_shapes() -> Result<(), String> {
        let mut definition = dip()?;
        validate_footprint_support(&definition)?;
        let p = &mut definition.pads[0];
        p.shape = "oval".into();
        p.size = Point::new(4.0, 2.0);
        p.roundrect_ratio = None;
        p.drill = Some(pcbcraft_library::Drill {
            size: Point::new(1.6, 0.6),
            offset: Point::new(0.3, 0.2),
        });
        validate_footprint_support(&definition)?;
        let p = &mut definition.pads[0];
        p.shape = "circle".into();
        p.size = Point::new(2.0, 2.0);
        p.drill = Some(pcbcraft_library::Drill {
            size: Point::new(1.4, 0.2),
            offset: Point::new(0.1, 0.1),
        });
        validate_footprint_support(&definition)?;
        Ok(())
    }
    #[test]
    fn diagonal_holes_cannot_escape_circle_oval_or_rounded_corners() -> Result<(), String> {
        for (shape, size, ratio, drill_size, offset) in [
            (
                "circle",
                Point::new(2.0, 2.0),
                None,
                Point::new(0.5, 0.5),
                Point::new(0.6, 0.6),
            ),
            (
                "oval",
                Point::new(4.0, 2.0),
                None,
                Point::new(0.5, 0.5),
                Point::new(1.6, 0.6),
            ),
            (
                "roundrect",
                Point::new(2.0, 2.0),
                Some(0.25),
                Point::new(0.2, 0.2),
                Point::new(0.86, 0.86),
            ),
        ] {
            let mut definition = dip()?;
            let p = &mut definition.pads[0];
            p.shape = shape.into();
            p.size = size;
            p.roundrect_ratio = ratio;
            p.drill = Some(pcbcraft_library::Drill {
                size: drill_size,
                offset,
            });
            assert!(
                validate_footprint_support(&definition).is_err(),
                "{shape} must reject drill escaping its actual outline"
            );
        }
        let mut definition = dip()?;
        let p = &mut definition.pads[0];
        p.shape = "rect".into();
        p.size = Point::new(2.0, 2.0);
        p.roundrect_ratio = None;
        p.drill = Some(pcbcraft_library::Drill {
            size: Point::new(0.5, 0.5),
            offset: Point::new(0.6, 0.6),
        });
        validate_footprint_support(&definition)?;
        Ok(())
    }
}
