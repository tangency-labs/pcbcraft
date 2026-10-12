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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_symbol: Option<LibrarySymbol>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_footprint: Option<LibraryFootprint>,
}
impl Component {
    /// Local drawing extents used for selection and labels, independent of the renderer.
    pub fn drawing_points(&self) -> Vec<Point> {
        let mut points: Vec<Point> = self.pins.iter().map(|p| p.offset).collect();
        if let Some(library) = &self.library_symbol {
            for graphic in library
                .definition
                .graphics
                .iter()
                .filter(|g| g.unit <= 1 && g.body_style <= 1)
            {
                points.extend(graphic.primitive.path_points());
            }
        } else {
            points.extend([Point::new(-3.0, -2.0), Point::new(3.0, 2.0)]);
        }
        points
    }
    pub fn pin_position(&self, number: &str) -> Option<Point> {
        self.pins
            .iter()
            .find(|p| p.number == number)
            .map(|p| self.position.offset(p.offset.rotate(-self.rotation)))
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
}
impl Default for Project {
    fn default() -> Self {
        Self {
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
            Some(local.rotate(-self.component(r.component)?.rotation))
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
                    || symbol.definition.pins.iter().any(|p| p.unit > 1)
                {
                    return Err("invalid or unsupported embedded library symbol".into());
                }
                let pins: Vec<_> = symbol
                    .definition
                    .pins
                    .iter()
                    .filter(|p| {
                        (p.unit == 0 || p.unit == symbol.unit)
                            && (p.body_style == 0 || p.body_style == 1)
                    })
                    .collect();
                if pins.len() != c.pins.len()
                    || pins.iter().any(|p| {
                        !c.pins.iter().any(|pin| {
                            pin.number == p.number
                                && pin.name == p.name
                                && pin.offset == p.position
                                && pin.direction == Some(p.angle)
                                && pin.electrical_type == p.electrical_type
                        })
                    })
                {
                    return Err("component pins do not match embedded symbol definition".into());
                }
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

pub fn validate_footprint_support(
    definition: &pcbcraft_library::FootprintDefinition,
) -> Result<(), String> {
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
    let mut numbers = BTreeSet::new();
    for p in &definition.pads {
        if p.pad_type != "smd"
            || !["circle", "rect", "roundrect", "oval"].contains(&p.shape.as_str())
            || (p.shape == "circle" && (p.size.x - p.size.y).abs() > 1e-6)
            || !p.layers.iter().any(|l| l == "F.Cu")
            || p.layers.iter().any(|l| l == "B.Cu" || l == "*.Cu")
            || p.drill.is_some()
            || p.number.is_empty()
            || !numbers.insert(&p.number)
        {
            return Err("initial imported footprint placement supports unique numbered front SMD circle/rect/roundrect/oval pads; through-hole, custom pads and back copper are not supported yet".into());
        }
    }
    if definition.pads.is_empty() || definition.pads.len() > 64 {
        return Err("imported footprint must have 1–64 pads".into());
    }
    Ok(())
}

fn orthogonal_path(a: Point, b: Point, da: Point, db: Point) -> Vec<Point> {
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
