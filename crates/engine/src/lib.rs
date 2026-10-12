//! The single mutation boundary used by UI, CLI and MCP.
pub mod library;
use pcbcraft_doc::*;
use pcbcraft_geom::{Point, positive};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::VecDeque;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("stale revision: expected {expected}, current {current}")]
    Stale { expected: u64, current: u64 },
}
impl From<String> for Error {
    fn from(s: String) -> Self {
        Self::Invalid(s)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub command: String,
    #[serde(default)]
    pub params: Value,
}
#[derive(Clone, Debug, Serialize)]
pub struct CommandSpec {
    pub id: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn id_schema() -> Value {
    json!({"type":"integer","minimum":1})
}
fn point_schema() -> Value {
    schema(
        json!({"x":{"type":"number","description":"millimetres"},"y":{"type":"number","description":"millimetres"}}),
        &["x", "y"],
    )
}
fn pin_schema() -> Value {
    schema(
        json!({"component":id_schema(),"pin":{"type":"string"}}),
        &["component", "pin"],
    )
}
pub fn commands() -> Vec<CommandSpec> {
    let entries = [
        (
            "project.rename",
            "Rename the current project",
            schema(json!({"name":{"type":"string"}}), &["name"]),
        ),
        (
            "component.add",
            "Place an original built-in symbol: resistor, capacitor, led or connector",
            schema(
                json!({"kind":{"type":"string","enum":["resistor","capacitor","led","connector"]},"position":point_schema(),"reference":{"type":"string"},"value":{"type":"string"}}),
                &["kind", "position"],
            ),
        ),
        (
            "component.place_library",
            "Place a supported single-unit KiCad symbol and embed its definition in the project",
            schema(
                json!({"path":{"type":"string"},"symbol":{"type":"string"},"position":point_schema(),"reference":{"type":"string"},"value":{"type":"string"}}),
                &["path", "symbol", "position"],
            ),
        ),
        (
            "footprint.assign",
            "Assign a KiCad front SMD footprint with matching numbered pads to a component",
            schema(
                json!({"id":id_schema(),"path":{"type":"string"}}),
                &["id", "path"],
            ),
        ),
        (
            "component.move",
            "Move a schematic component; connected wire endpoints follow",
            schema(
                json!({"id":id_schema(),"position":point_schema()}),
                &["id", "position"],
            ),
        ),
        (
            "component.rotate",
            "Set schematic component rotation in degrees",
            schema(
                json!({"id":id_schema(),"degrees":{"type":"number"}}),
                &["id", "degrees"],
            ),
        ),
        (
            "component.value",
            "Change the component value",
            schema(
                json!({"id":id_schema(),"value":{"type":"string"}}),
                &["id", "value"],
            ),
        ),
        (
            "net.create",
            "Create a named logical net",
            schema(json!({"name":{"type":"string"}}), &["name"]),
        ),
        (
            "net.connect",
            "Assign unassigned or same-net component pins to a net",
            schema(
                json!({"net":id_schema(),"pins":{"type":"array","items":pin_schema(),"minItems":1,"maxItems":128}}),
                &["net", "pins"],
            ),
        ),
        (
            "wire.connect",
            "Connect two pins; infer their existing net or atomically create a net if neither is assigned",
            schema(
                json!({"net":id_schema(),"from":pin_schema(),"to":pin_schema()}),
                &["from", "to"],
            ),
        ),
        (
            "board.sync",
            "Reconcile schematic components and pin nets into PCB footprints; retain placements and report unassigned library symbols",
            schema(json!({}), &[]),
        ),
        (
            "board.resize",
            "Set rectangular board dimensions, in millimetres",
            schema(
                json!({"width":{"type":"number","exclusiveMinimum":0},"height":{"type":"number","exclusiveMinimum":0}}),
                &["width", "height"],
            ),
        ),
        (
            "board.rules.set",
            "Set minimum copper clearance in millimetres",
            schema(
                json!({"clearance":{"type":"number","exclusiveMinimum":0}}),
                &["clearance"],
            ),
        ),
        (
            "board.cutout.add",
            "Add a dimensioned circular Edge.Cuts cutout strictly within the board",
            schema(
                json!({"center":point_schema(),"radius":{"type":"number","exclusiveMinimum":0}}),
                &["center", "radius"],
            ),
        ),
        (
            "footprint.move",
            "Move a board footprint; existing routed tracks stay fixed",
            schema(
                json!({"id":id_schema(),"position":point_schema()}),
                &["id", "position"],
            ),
        ),
        (
            "footprint.rotate",
            "Set footprint rotation in degrees; existing tracks stay fixed",
            schema(
                json!({"id":id_schema(),"degrees":{"type":"number"}}),
                &["id", "degrees"],
            ),
        ),
        (
            "track.add",
            "Add a straight copper segment; checks report clearance errors rather than hiding them",
            schema(
                json!({"net":id_schema(),"start":point_schema(),"end":point_schema(),"width":{"type":"number","exclusiveMinimum":0},"layer":{"type":"string","enum":["F.Cu","B.Cu"]}}),
                &["net", "start", "end"],
            ),
        ),
        (
            "track.route",
            "Connect two same-net front-layer pads with an orthogonal two-segment route (not shove routing)",
            schema(
                json!({"from":schema(json!({"footprint":id_schema(),"pad":{"type":"string"}}),&["footprint","pad"]),"to":schema(json!({"footprint":id_schema(),"pad":{"type":"string"}}),&["footprint","pad"]),"width":{"type":"number","exclusiveMinimum":0}}),
                &["from", "to"],
            ),
        ),
        (
            "object.delete",
            "Delete a component, wire, track, footprint, cutout or unused net by stable ID",
            schema(json!({"id":id_schema()}), &["id"]),
        ),
    ];
    entries
        .into_iter()
        .map(|(id, description, input_schema)| CommandSpec {
            id,
            description,
            input_schema,
        })
        .collect()
}

#[derive(Clone)]
pub struct Session {
    pub project: Project,
    pub revision: u64,
    next_id: Id,
    undo: VecDeque<Project>,
    redo: VecDeque<Project>,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            project: Project::default(),
            revision: 0,
            next_id: 1,
            undo: VecDeque::new(),
            redo: VecDeque::new(),
        }
    }
}
impl Session {
    fn check_revision(&self, expected: Option<u64>) -> Result<(), Error> {
        if let Some(expected) = expected
            && expected != self.revision
        {
            return Err(Error::Stale {
                expected,
                current: self.revision,
            });
        }
        Ok(())
    }
    pub fn execute(
        &mut self,
        id: &str,
        params: Value,
        expected: Option<u64>,
    ) -> Result<Value, Error> {
        let result = self.batch(
            &[Command {
                command: id.into(),
                params,
            }],
            expected,
        )?;
        Ok(
            json!({"revision":self.revision,"result":result.get("results").and_then(Value::as_array).and_then(|a|a.first()).cloned().unwrap_or(Value::Null)}),
        )
    }
    pub fn batch(&mut self, commands: &[Command], expected: Option<u64>) -> Result<Value, Error> {
        self.check_revision(expected)?;
        if commands.is_empty() || commands.len() > 128 {
            return Err(Error::Invalid("batch must contain 1–128 commands".into()));
        }
        let mut candidate = self.project.clone();
        let mut next = self.next_id;
        let mut results = vec![];
        for command in commands {
            results.push(run(
                &mut candidate,
                &mut next,
                &command.command,
                &command.params,
            )?);
            candidate.validate()?;
        }
        self.commit(candidate);
        self.next_id = next;
        Ok(json!({"revision":self.revision,"results":results}))
    }
    fn commit(&mut self, p: Project) {
        self.undo.push_back(std::mem::replace(&mut self.project, p));
        while self.undo.len() > 32 {
            self.undo.pop_front();
        }
        self.redo.clear();
        self.revision = self.revision.saturating_add(1);
    }
    pub fn undo(&mut self) -> Result<Value, Error> {
        let p = self
            .undo
            .pop_back()
            .ok_or_else(|| Error::Invalid("nothing to undo".into()))?;
        self.redo.push_back(std::mem::replace(&mut self.project, p));
        self.revision = self.revision.saturating_add(1);
        Ok(json!({"revision":self.revision}))
    }
    pub fn redo(&mut self) -> Result<Value, Error> {
        let p = self
            .redo
            .pop_back()
            .ok_or_else(|| Error::Invalid("nothing to redo".into()))?;
        self.undo.push_back(std::mem::replace(&mut self.project, p));
        self.revision = self.revision.saturating_add(1);
        Ok(json!({"revision":self.revision}))
    }
    pub fn replace(&mut self, p: Project) -> Result<Value, Error> {
        p.validate()?;
        self.next_id = self
            .next_id
            .max(p.ids().max().unwrap_or(0).saturating_add(1));
        self.commit(p);
        Ok(json!({"revision":self.revision}))
    }
    pub fn inspect(&self) -> Value {
        json!({"revision":self.revision,"project":self.project,"history":{"undo":self.undo.len(),"redo":self.redo.len()},"capabilities":{"schematic":"flat explicit pin nets; original built-ins and embedded single-unit KiCad library symbols","pcb":"embedded front SMD circle/rect/roundrect/oval pads; straight/orthogonal tracks, rectangular outline, circular cutouts","checks":"initial pin assignment and conservative copper bounds; not comprehensive ERC/DRC","kicad":"native library reading and limited PCB export; no general project import","routing":"no obstacle avoidance or shove","3d":false}})
    }
    pub fn checks(&self) -> Value {
        json!({"revision":self.revision,"erc":pcbcraft_rules::erc(&self.project),"drc":pcbcraft_rules::drc(&self.project),"airwires":pcbcraft_rules::airwires(&self.project),"scope":"Initial checks: pin assignment, net population, capsule tracks, rectangular edge and circular cutout clearance. Imported noncircular pads use sampled convex outlines with conservative chord-error clearance bounds and inscribed connectivity bounds, so conservative false findings are possible. No full pin-type, via, zone or hierarchy checks."})
    }
    pub fn sample() -> Result<Self, Error> {
        let mut s = Self::default();
        s.execute("project.rename", json!({"name":"LED indicator"}), None)?;
        s.execute("component.add",json!({"kind":"connector","reference":"J1","value":"5V input","position":{"x":15,"y":26}}),None)?;
        s.execute(
            "component.add",
            json!({"kind":"resistor","reference":"R1","value":"330 Ω","position":{"x":40,"y":14}}),
            None,
        )?;
        s.execute(
            "component.add",
            json!({"kind":"led","reference":"D1","value":"Green LED","position":{"x":65,"y":26}}),
            None,
        )?;
        for name in ["VCC", "LED", "GND"] {
            s.execute("net.create", json!({"name":name}), None)?;
        }
        for (net, a, ap, b, bp) in [
            (4, 1, "1", 2, "1"),
            (5, 2, "2", 3, "1"),
            (6, 1, "2", 3, "2"),
        ] {
            s.execute(
                "wire.connect",
                json!({"net":net,"from":{"component":a,"pin":ap},"to":{"component":b,"pin":bp}}),
                None,
            )?;
        }
        s.execute("board.sync", json!({}), None)?;
        for (id, x, y) in [(10, 12, 20), (11, 28, 12), (12, 44, 20)] {
            s.execute(
                "footprint.move",
                json!({"id":id,"position":{"x":x,"y":y}}),
                None,
            )?;
        }
        s.execute(
            "board.cutout.add",
            json!({"center":{"x":5,"y":5},"radius":1.6}),
            None,
        )?;
        s.execute(
            "board.cutout.add",
            json!({"center":{"x":55,"y":35},"radius":1.6}),
            None,
        )?;
        s.undo.clear();
        s.redo.clear();
        Ok(s)
    }
}

fn parse<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, Error> {
    serde_json::from_value(v.clone()).map_err(|e| Error::Invalid(e.to_string()))
}
fn allocate(next: &mut Id) -> Result<Id, Error> {
    let id = *next;
    if id == 0 || id > u32::MAX as u64 {
        return Err(Error::Invalid("object ID space exhausted".into()));
    }
    *next = next.saturating_add(1);
    Ok(id)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Add {
    kind: String,
    position: Point,
    reference: Option<String>,
    value: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaceLibrary {
    path: String,
    symbol: String,
    position: Point,
    reference: Option<String>,
    value: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssignFootprint {
    id: Id,
    path: String,
}
fn component_pads(c: &Component) -> Vec<Pad> {
    if let Some(library) = &c.library_footprint {
        library
            .definition
            .pads
            .iter()
            .map(|pad| Pad {
                number: pad.number.clone(),
                offset: pad.position,
                diameter: pad.size.x.min(pad.size.y),
                net: c
                    .pins
                    .iter()
                    .find(|p| p.number == pad.number)
                    .and_then(|p| p.net),
                geometry: Some(pad.clone()),
            })
            .collect()
    } else {
        c.pins
            .iter()
            .enumerate()
            .map(|(i, pin)| Pad {
                number: pin.number.clone(),
                offset: Point::new(if i == 0 { -2.5 } else { 2.5 }, 0.0),
                diameter: 1.6,
                net: pin.net,
                geometry: None,
            })
            .collect()
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Move {
    id: Id,
    position: Point,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rotate {
    id: Id,
    degrees: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ValueParams {
    id: Id,
    value: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Name {
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Connect {
    net: Id,
    pins: Vec<PinRef>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireParams {
    net: Option<Id>,
    from: PinRef,
    to: PinRef,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Resize {
    width: f64,
    height: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Clearance {
    clearance: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cut {
    center: Point,
    radius: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackParams {
    net: Id,
    start: Point,
    end: Point,
    #[serde(default = "default_width")]
    width: f64,
    #[serde(default = "front")]
    layer: Layer,
}
#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct PadRef {
    footprint: Id,
    pad: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Route {
    from: PadRef,
    to: PadRef,
    #[serde(default = "default_width")]
    width: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Object {
    id: Id,
}
fn default_width() -> f64 {
    0.35
}
fn front() -> Layer {
    Layer::Front
}
fn missing() -> Error {
    Error::Invalid("object not found".into())
}
fn assign(p: &mut Project, net: Id, pins: &[PinRef]) -> Result<(), Error> {
    if pins.is_empty() || pins.len() > 128 || !p.nets.iter().any(|n| n.id == net) {
        return Err(Error::Invalid("invalid pin list or unknown net".into()));
    }
    for r in pins {
        let pin = p
            .components
            .iter_mut()
            .find(|c| c.id == r.component)
            .and_then(|c| c.pins.iter_mut().find(|pin| pin.number == r.pin))
            .ok_or_else(missing)?;
        if pin.net.is_some_and(|n| n != net) {
            return Err(Error::Invalid(
                "pin already belongs to another net; explicit net merge is not implemented".into(),
            ));
        }
        pin.net = Some(net);
        for f in p
            .board
            .footprints
            .iter_mut()
            .filter(|f| f.component == r.component)
        {
            for pad in f.pads.iter_mut().filter(|pad| pad.number == r.pin) {
                pad.net = Some(net);
            }
        }
    }
    Ok(())
}
fn pad(p: &Project, r: &PadRef) -> Result<(Point, Id), Error> {
    let f = p.footprint(r.footprint).ok_or_else(missing)?;
    let position = f.pad_position(&r.pad).ok_or_else(missing)?;
    let net = f
        .pads
        .iter()
        .find(|pad| pad.number == r.pad)
        .and_then(|pad| pad.net)
        .ok_or_else(|| Error::Invalid("pad has no net".into()))?;
    Ok((position, net))
}
fn run(p: &mut Project, next: &mut Id, command: &str, params: &Value) -> Result<Value, Error> {
    match command {
        "project.rename" => {
            let a: Name = parse(params)?;
            p.name = a.name;
            Ok(json!({"name":p.name}))
        }
        "component.add" => {
            let a: Add = parse(params)?;
            let template = TEMPLATES
                .iter()
                .find(|t| t.kind == a.kind)
                .ok_or_else(|| Error::Invalid("unknown symbol kind".into()))?;
            let reference = match a.reference {
                Some(r) => r,
                None => {
                    let mut n = 1;
                    while p
                        .components
                        .iter()
                        .any(|c| c.reference == format!("{}{n}", template.prefix))
                    {
                        n += 1;
                    }
                    format!("{}{n}", template.prefix)
                }
            };
            let id = allocate(next)?;
            let mut c =
                component_template(&a.kind, id, reference, a.position).ok_or_else(missing)?;
            if let Some(value) = a.value {
                c.value = value;
            }
            p.components.push(c);
            Ok(json!({"id":id}))
        }
        "component.place_library" => {
            let a: PlaceLibrary = parse(params)?;
            let path = std::path::Path::new(&a.path);
            let definition = library::symbol(path, &a.symbol)?;
            definition
                .validate()
                .map_err(|e| Error::Invalid(e.to_string()))?;
            if !definition.unsupported.is_empty() || definition.pins.iter().any(|p| p.unit > 1) {
                return Err(Error::Invalid(format!(
                    "placement currently requires a single-unit symbol with supported graphics and pin styles; multi-unit symbols can be inspected. Unsupported: {}",
                    definition.unsupported.join(", ")
                )));
            }
            let pins: Vec<Pin> = definition
                .pins
                .iter()
                .filter(|p| p.unit <= 1 && p.body_style <= 1)
                .map(|p| Pin {
                    number: p.number.clone(),
                    name: p.name.clone(),
                    offset: p.position,
                    net: None,
                    electrical_type: p.electrical_type.clone(),
                    direction: Some(p.angle),
                })
                .collect();
            if pins.is_empty() || pins.len() > 64 {
                return Err(Error::Invalid("symbol placement requires 1–64 pins".into()));
            }
            let prefix = if definition.reference.is_empty() {
                "U"
            } else {
                &definition.reference
            };
            let reference = a.reference.unwrap_or_else(|| {
                let mut n = 1;
                while p
                    .components
                    .iter()
                    .any(|c| c.reference == format!("{prefix}{n}"))
                {
                    n += 1;
                }
                format!("{prefix}{n}")
            });
            let id = allocate(next)?;
            p.components.push(Component {
                id,
                reference,
                value: a.value.unwrap_or_else(|| definition.value.clone()),
                kind: "library".into(),
                position: a.position,
                rotation: 0.0,
                pins,
                library_symbol: Some(LibrarySymbol {
                    library: path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("library")
                        .into(),
                    unit: 1,
                    definition,
                }),
                library_footprint: None,
            });
            Ok(json!({"id":id}))
        }
        "footprint.assign" => {
            let a: AssignFootprint = parse(params)?;
            let path = std::path::Path::new(&a.path);
            let definition = library::footprint(path)?;
            definition
                .validate()
                .map_err(|e| Error::Invalid(e.to_string()))?;
            validate_footprint_support(&definition)?;
            let c = p
                .components
                .iter_mut()
                .find(|c| c.id == a.id)
                .ok_or_else(missing)?;
            let pins: std::collections::BTreeSet<_> =
                c.pins.iter().map(|p| p.number.as_str()).collect();
            let pads: std::collections::BTreeSet<_> =
                definition.pads.iter().map(|p| p.number.as_str()).collect();
            if pins != pads {
                return Err(Error::Invalid(
                    "footprint pad numbers must match every symbol pin exactly".into(),
                ));
            }
            c.library_footprint = Some(LibraryFootprint {
                library: path
                    .parent()
                    .and_then(|p| p.file_stem())
                    .and_then(|s| s.to_str())
                    .unwrap_or("library")
                    .into(),
                definition,
            });
            if let Some(f) = p.board.footprints.iter_mut().find(|f| f.component == a.id) {
                f.pads = component_pads(c);
            }
            Ok(json!({"component":a.id}))
        }
        "component.move" => {
            let a: Move = parse(params)?;
            p.components
                .iter_mut()
                .find(|c| c.id == a.id)
                .ok_or_else(missing)?
                .position = a.position;
            Ok(json!({"id":a.id}))
        }
        "component.rotate" => {
            let a: Rotate = parse(params)?;
            if !a.degrees.is_finite() {
                return Err(Error::Invalid("rotation must be finite".into()));
            }
            p.components
                .iter_mut()
                .find(|c| c.id == a.id)
                .ok_or_else(missing)?
                .rotation = a.degrees.rem_euclid(360.0);
            Ok(json!({"id":a.id}))
        }
        "component.value" => {
            let a: ValueParams = parse(params)?;
            p.components
                .iter_mut()
                .find(|c| c.id == a.id)
                .ok_or_else(missing)?
                .value = a.value;
            Ok(json!({"id":a.id}))
        }
        "net.create" => {
            let a: Name = parse(params)?;
            let id = allocate(next)?;
            p.nets.push(Net { id, name: a.name });
            Ok(json!({"id":id}))
        }
        "net.connect" => {
            let a: Connect = parse(params)?;
            assign(p, a.net, &a.pins)?;
            Ok(json!({"net":a.net,"pins":a.pins.len()}))
        }
        "wire.connect" => {
            let a: WireParams = parse(params)?;
            if a.from == a.to {
                return Err(Error::Invalid("wire needs two distinct pins".into()));
            }
            let from = p.pin(&a.from).ok_or_else(missing)?.net;
            let to = p.pin(&a.to).ok_or_else(missing)?.net;
            let net = if let Some(net) = a.net.or(from).or(to) {
                net
            } else {
                let id = allocate(next)?;
                let mut index = p.nets.len() + 1;
                while p.nets.iter().any(|n| n.name == format!("N${index}")) {
                    index += 1;
                }
                p.nets.push(Net {
                    id,
                    name: format!("N${index}"),
                });
                id
            };
            assign(p, net, &[a.from.clone(), a.to.clone()])?;
            let id = allocate(next)?;
            p.wires.push(Wire {
                id,
                net,
                from: a.from,
                to: a.to,
            });
            Ok(json!({"id":id,"net":net}))
        }
        "board.sync" => {
            if params.as_object().is_none_or(|m| !m.is_empty()) {
                return Err(Error::Invalid("board.sync takes an empty object".into()));
            }
            let mut skipped = vec![];
            for (index, c) in p.components.iter().enumerate() {
                if c.library_symbol.is_some() && c.library_footprint.is_none() {
                    skipped.push(c.id);
                    continue;
                }
                if let Some(f) = p.board.footprints.iter_mut().find(|f| f.component == c.id) {
                    f.pads = component_pads(c);
                } else {
                    let id = allocate(next)?;
                    p.board.footprints.push(Footprint {
                        id,
                        component: c.id,
                        position: Point::new(
                            10.0 + (index % 4) as f64 * 12.0,
                            10.0 + (index / 4) as f64 * 8.0,
                        ),
                        rotation: 0.0,
                        pads: component_pads(c),
                    });
                }
            }
            Ok(
                json!({"footprints":p.board.footprints.iter().map(|f|json!({"id":f.id,"component":f.component})).collect::<Vec<_>>(),"unassigned_components":skipped}),
            )
        }
        "board.resize" => {
            let a: Resize = parse(params)?;
            p.board.width = a.width;
            p.board.height = a.height;
            Ok(json!({"width":a.width,"height":a.height}))
        }
        "board.rules.set" => {
            let a: Clearance = parse(params)?;
            p.board.clearance = a.clearance;
            Ok(json!({"clearance":a.clearance}))
        }
        "board.cutout.add" => {
            let a: Cut = parse(params)?;
            let id = allocate(next)?;
            p.board.cutouts.push(Cutout {
                id,
                center: a.center,
                radius: a.radius,
            });
            Ok(json!({"id":id}))
        }
        "footprint.move" => {
            let a: Move = parse(params)?;
            p.board
                .footprints
                .iter_mut()
                .find(|f| f.id == a.id)
                .ok_or_else(missing)?
                .position = a.position;
            Ok(json!({"id":a.id}))
        }
        "footprint.rotate" => {
            let a: Rotate = parse(params)?;
            if !a.degrees.is_finite() {
                return Err(Error::Invalid("rotation must be finite".into()));
            }
            p.board
                .footprints
                .iter_mut()
                .find(|f| f.id == a.id)
                .ok_or_else(missing)?
                .rotation = a.degrees.rem_euclid(360.0);
            Ok(json!({"id":a.id}))
        }
        "track.add" => {
            let a: TrackParams = parse(params)?;
            let id = allocate(next)?;
            p.board.tracks.push(Track {
                id,
                net: a.net,
                start: a.start,
                end: a.end,
                width: a.width,
                layer: a.layer,
            });
            Ok(json!({"id":id}))
        }
        "track.route" => {
            let a: Route = parse(params)?;
            let (start, net) = pad(p, &a.from)?;
            let (end, end_net) = pad(p, &a.to)?;
            if net != end_net || start.distance(end) < 1e-6 || !positive(a.width) {
                return Err(Error::Invalid(
                    "route requires distinct pads on the same net and a valid width".into(),
                ));
            }
            let bend = Point::new(end.x, start.y);
            let mut ids = vec![];
            for (from, to) in [(start, bend), (bend, end)] {
                if from.distance(to) > 1e-6 {
                    let id = allocate(next)?;
                    ids.push(id);
                    p.board.tracks.push(Track {
                        id,
                        net,
                        start: from,
                        end: to,
                        width: a.width,
                        layer: Layer::Front,
                    });
                }
            }
            Ok(json!({"ids":ids,"net":net}))
        }
        "object.delete" => {
            let a: Object = parse(params)?;
            if !p.ids().any(|id| id == a.id) {
                return Err(missing());
            }
            if p.nets.iter().any(|n| n.id == a.id)
                && (p
                    .components
                    .iter()
                    .flat_map(|c| &c.pins)
                    .any(|pin| pin.net == Some(a.id))
                    || p.board.tracks.iter().any(|t| t.net == a.id))
            {
                return Err(Error::Invalid("cannot delete a net that is in use".into()));
            }
            p.components.retain(|c| c.id != a.id);
            p.nets.retain(|n| n.id != a.id);
            p.wires
                .retain(|w| w.id != a.id && w.from.component != a.id && w.to.component != a.id);
            p.board
                .footprints
                .retain(|f| f.id != a.id && f.component != a.id);
            p.board.tracks.retain(|t| t.id != a.id);
            p.board.cutouts.retain(|c| c.id != a.id);
            Ok(json!({"deleted":a.id}))
        }
        _ => Err(Error::Invalid(format!("unknown command: {command}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn library_fixture(name: &str) -> String {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../library/tests/fixtures")
            .join(name)
            .to_string_lossy()
            .into_owned()
    }
    #[test]
    fn imported_definitions_survive_roundtrip_and_assignment_is_undoable() -> Result<(), Error> {
        let mut s = Session::default();
        s.execute("component.place_library",json!({"path":library_fixture("device.kicad_sym"),"symbol":"R","position":{"x":20,"y":15}}),None)?;
        s.execute("board.sync", json!({}), None)?;
        assert!(
            s.project.board.footprints.is_empty(),
            "unassigned symbols must not invent a physical package"
        );
        s.execute(
            "footprint.assign",
            json!({"id":1,"path":library_fixture("R_0805_2012Metric.kicad_mod")}),
            None,
        )?;
        assert!(
            s.project
                .component(1)
                .is_some_and(|c| c.library_footprint.is_some())
        );
        s.undo()?;
        assert!(
            s.project
                .component(1)
                .is_some_and(|c| c.library_footprint.is_none())
        );
        s.redo()?;
        s.execute("board.sync", json!({}), None)?;
        let f = s.project.board.footprints.first().ok_or_else(missing)?;
        assert_eq!(f.pads.len(), 2);
        assert!(
            f.pads
                .iter()
                .all(|p| p.geometry.as_ref().is_some_and(|g| g.shape == "roundrect"))
        );
        assert!(
            f.pads
                .first()
                .is_some_and(|p| (p.offset.x + 0.9125).abs() < 1e-6)
        );
        let text = serde_json::to_string(&s.project).map_err(|e| Error::Invalid(e.to_string()))?;
        let loaded: Project =
            serde_json::from_str(&text).map_err(|e| Error::Invalid(e.to_string()))?;
        loaded.validate()?;
        assert_eq!(loaded, s.project);
        let mut tampered = loaded;
        if let Some(pad) = tampered
            .board
            .footprints
            .first_mut()
            .and_then(|f| f.pads.first_mut())
        {
            pad.geometry = None;
        }
        assert!(
            tampered.validate().is_err(),
            "embedded physical geometry must not silently become generic"
        );
        Ok(())
    }
    #[test]
    fn multiunit_placement_and_mismatched_footprint_roll_back() -> Result<(), Error> {
        let mut s = Session::default();
        let before = s.inspect();
        assert!(s.execute("component.place_library",json!({"path":library_fixture("lm358.kicad_sym"),"symbol":"LM358","position":{"x":20,"y":15}}),None).is_err());
        assert_eq!(s.inspect(), before);
        s.execute("component.place_library",json!({"path":library_fixture("device.kicad_sym"),"symbol":"R","position":{"x":20,"y":15}}),None)?;
        // A project with a corrupted pin map is rejected at the document boundary.
        let mut corrupted = s.project.clone();
        if let Some(pin) = corrupted
            .components
            .first_mut()
            .and_then(|c| c.pins.first_mut())
        {
            pin.number = "99".into();
        }
        assert!(s.replace(corrupted).is_err());
        let before = s.inspect();
        let batch = [
            Command {
                command: "project.rename".into(),
                params: json!({"name":"uncommitted"}),
            },
            Command {
                command: "footprint.assign".into(),
                params: json!({"id":999,"path":library_fixture("R_0805_2012Metric.kicad_mod")}),
            },
        ];
        assert!(s.batch(&batch, None).is_err());
        assert_eq!(s.inspect(), before);
        Ok(())
    }
    #[test]
    fn auto_net_and_wire_commit_as_one_undo_step() -> Result<(), Error> {
        let mut s = Session::default();
        for x in [10, 30] {
            s.execute(
                "component.add",
                json!({"kind":"resistor","position":{"x":x,"y":10}}),
                None,
            )?;
        }
        s.execute(
            "wire.connect",
            json!({"from":{"component":1,"pin":"2"},"to":{"component":2,"pin":"1"}}),
            None,
        )?;
        assert_eq!(s.project.nets.len(), 1);
        assert_eq!(s.project.wires.len(), 1);
        s.undo()?;
        assert!(s.project.nets.is_empty());
        assert!(s.project.wires.is_empty());
        assert_eq!(s.project.components.len(), 2);
        Ok(())
    }
    #[test]
    fn sample_wire_paths_never_run_through_foreign_pins() -> Result<(), Error> {
        let s = Session::sample()?;
        for w in &s.project.wires {
            let path = s.project.wire_path(w).ok_or_else(missing)?;
            for c in &s.project.components {
                for pin in &c.pins {
                    if pin.net == Some(w.net) {
                        continue;
                    }
                    let point = c.pin_position(&pin.number).ok_or_else(missing)?;
                    for segment in path.windows(2) {
                        if let Some((a, b)) = segment.first().zip(segment.get(1)) {
                            assert!(
                                pcbcraft_geom::point_segment(point, *a, *b) > 0.5,
                                "distinct nets must not overlap visually in sample"
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }
    #[test]
    fn counterclockwise_pad_rotation_matches_y_down_board_coordinates() -> Result<(), Error> {
        let mut s = Session::sample()?;
        s.execute("footprint.rotate", json!({"id":11,"degrees":90}), None)?;
        let f = s.project.footprint(11).ok_or_else(missing)?;
        let pin = f.pad_position("1").ok_or_else(missing)?;
        assert!((pin.x - 28.0).abs() < 1e-6);
        assert!((pin.y - 14.5).abs() < 1e-6);
        Ok(())
    }
    #[test]
    fn failed_batch_is_atomic_and_stale_edits_reject() {
        let mut s = Session::default();
        let before = s.inspect();
        let batch = [
            Command {
                command: "net.create".into(),
                params: json!({"name":"GND"}),
            },
            Command {
                command: "component.move".into(),
                params: json!({"id":999,"position":{"x":1,"y":2}}),
            },
        ];
        assert!(s.batch(&batch, None).is_err());
        assert_eq!(s.inspect(), before);
        assert!(
            s.execute("net.create", json!({"name":"GND"}), Some(100))
                .is_err()
        );
        assert_eq!(s.inspect(), before);
    }
    #[test]
    fn undo_branch_never_reuses_ids() {
        let mut s = Session::default();
        assert!(s.execute("net.create", json!({"name":"A"}), None).is_ok());
        assert!(s.undo().is_ok());
        assert!(s.execute("net.create", json!({"name":"B"}), None).is_ok());
        assert_eq!(s.project.nets.first().map(|n| n.id), Some(2));
        assert!(s.redo().is_err());
    }
    #[test]
    fn sample_is_valid_and_pcb_sync_retains_placements() {
        let result = Session::sample();
        assert!(result.is_ok());
        if let Ok(mut s) = result {
            assert!(s.project.validate().is_ok());
            let before = s.project.board.footprints.clone();
            assert!(s.execute("board.sync", json!({}), None).is_ok());
            assert_eq!(before, s.project.board.footprints);
            assert!(pcbcraft_rules::erc(&s.project).is_empty());
        }
    }
    #[test]
    fn hostile_inputs_leave_state_untouched() {
        let mut s = Session::default();
        let before = s.inspect();
        for params in [
            json!({"kind":"resistor","position":{"x":1e200,"y":0}}),
            json!({"kind":"resistor","position":{"x":0,"y":0},"unexpected":true}),
            json!({"kind":"bogus","position":{"x":0,"y":0}}),
        ] {
            assert!(s.execute("component.add", params, None).is_err());
            assert_eq!(s.inspect(), before);
        }
    }
}
