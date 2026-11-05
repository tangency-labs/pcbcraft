//! The session and command set behind PCBCraft.
//!
//! One rule shapes this crate: anything a person can do, an agent can do through the same command.
//! The menus and the MCP server both call [`Session::execute`] with an id from [`catalog`]; neither
//! changes the project any other way. This crate knows nothing about windows or transports.
//!
//! The schematic supports in-memory resistor placement and explicit pin connections. The file format is still being discussed
//! (see `VISION.md`).

pub mod schematic;

use schematic::{
    MAX_CENTER_NM, MAX_SYMBOLS, Point, ResistorPlacement,
    wiring::{MAX_WIRES, Terminal, Wire, inspect_netlist},
};
use serde_json::{Map, Value, json};

/// Undo steps kept. The oldest is dropped past this.
const MAX_HISTORY: usize = 1000;
/// Longest project name, in characters.
const MAX_NAME: usize = 200;

#[derive(Clone, Copy, Debug)]
enum ParamKind {
    Name,
    Coordinate,
    Revision,
    Terminal,
}

impl ParamKind {
    /// Publish the parameter's JSON type and bounds to command clients.
    fn schema(self) -> Value {
        match self {
            Self::Name => json!({"type": "string"}),
            Self::Coordinate => json!({"type": "integer", "minimum": -MAX_CENTER_NM, "maximum": MAX_CENTER_NM}),
            Self::Revision => json!({"type": "integer", "minimum": 0}),
            Self::Terminal => json!({
                "type": "object",
                "properties": {
                    "symbol": {"type": "string", "pattern": "^symbol-[1-9][0-9]*$"},
                    "terminal": {"type": "string", "enum": ["1", "2"]},
                },
                "required": ["symbol", "terminal"],
                "additionalProperties": false,
            }),
        }
    }

    /// Check the raw JSON value before a command reads it, without coercing strings or fractions.
    fn validate(self, value: &Value) -> Result<(), &'static str> {
        match self {
            Self::Name if value.is_string() => Ok(()),
            Self::Coordinate if value.as_i64().is_some_and(|n| (-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&n)) => Ok(()),
            Self::Revision if value.as_u64().is_some() => Ok(()),
            Self::Terminal if Terminal::parse(value).is_some() => Ok(()),
            Self::Name => Err("must be a string"),
            Self::Coordinate => Err("must be a whole number of nanometers within the schematic bounds"),
            Self::Revision => Err("must be a nonnegative integer revision"),
            Self::Terminal => Err("must identify a symbol and its terminal number (1 or 2)"),
        }
    }
}

/// A parameter of a command.
#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub name: &'static str,
    pub summary: &'static str,
    kind: ParamKind,
}

/// A command as people and agents see it.
#[derive(Clone, Copy, Debug)]
pub struct Command {
    pub id: &'static str,
    /// What a menu calls it.
    pub title: &'static str,
    pub summary: &'static str,
    /// All required.
    pub params: &'static [Param],
}

impl Command {
    /// Describe the command and its required inputs for discovery by MCP clients.
    pub fn to_json(&self) -> Value {
        let params: Map<String, Value> = self
            .params
            .iter()
            .map(|p| {
                let mut schema = p.kind.schema();
                schema["description"] = json!(p.summary);
                (p.name.to_owned(), schema)
            })
            .collect();
        let required: Vec<&str> = self.params.iter().map(|p| p.name).collect();
        json!({
            "id": self.id,
            "title": self.title,
            "summary": self.summary,
            "params": {"type": "object", "properties": params, "required": required, "additionalProperties": false},
        })
    }
}

const NAME: Param = Param { name: "name", summary: "The project's name", kind: ParamKind::Name };
pub const PLACE_RESISTOR: &str = "schematic.place_resistor";
/// The command that joins two inspected symbol terminals with an undoable wire.
pub const CONNECT_PINS: &str = "schematic.connect_pins";
const EXPECTED_REVISION: Param = Param { name: "expected_revision", summary: "Revision returned by inspect_project", kind: ParamKind::Revision };

const CATALOG: &[Command] = &[
    Command { id: "project.new", title: "New Project", summary: "Start a new empty project, replacing the open one. Undoable.", params: &[NAME] },
    Command { id: "project.rename", title: "Rename Project", summary: "Change the open project's name.", params: &[NAME] },
    Command {
        id: PLACE_RESISTOR,
        title: "Place Resistor",
        summary: "Place a 1k resistor on the open schematic. Undoable.",
        params: &[
            Param { name: "x_nm", summary: "Symbol center X in nanometers, increasing right", kind: ParamKind::Coordinate },
            Param { name: "y_nm", summary: "Symbol center Y in nanometers, increasing down", kind: ParamKind::Coordinate },
            EXPECTED_REVISION,
        ],
    },
    Command {
        id: CONNECT_PINS,
        title: "Connect Pins",
        summary: "Join two existing schematic terminals. Undoable.",
        params: &[
            Param { name: "from", summary: "First symbol ID and terminal number from project inspection", kind: ParamKind::Terminal },
            Param { name: "to", summary: "Second symbol ID and terminal number from project inspection", kind: ParamKind::Terminal },
            EXPECTED_REVISION,
        ],
    },
    Command { id: "edit.undo", title: "Undo", summary: "Undo the last change.", params: &[] },
    Command { id: "edit.redo", title: "Redo", summary: "Redo the last undone change.", params: &[] },
];

/// Every command there is.
pub fn catalog() -> &'static [Command] {
    CATALOG
}

/// The command with this id.
pub fn command(id: &str) -> Option<&'static Command> {
    CATALOG.iter().find(|c| c.id == id)
}

/// Why a command did not run. The project is unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandError {
    Unknown(String),
    InvalidParams(String),
    /// The command is real, but can't run in this state, like undo with nothing to undo.
    Unavailable(String),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(id) => write!(f, "unknown command `{id}`"),
            Self::InvalidParams(m) | Self::Unavailable(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for CommandError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    resistors: Vec<ResistorPlacement>,
    wires: Vec<Wire>,
}

impl Project {
    /// Return the schematic's resistor placements in placement order.
    pub fn resistors(&self) -> &[ResistorPlacement] {
        &self.resistors
    }
}

/// The open project and its history. There is one per window or per MCP server.
#[derive(Debug, Default)]
pub struct Session {
    project: Option<Project>,
    undo: Vec<Option<Project>>,
    redo: Vec<Option<Project>>,
    /// Goes up with every change, undo and redo included, so it never repeats.
    revision: u64,
}

impl Session {
    pub fn project(&self) -> Option<&Project> {
        self.project.as_ref()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether the command can run now, for greying out a menu item. Unknown ids can't.
    pub fn available(&self, id: &str) -> bool {
        match id {
            "project.new" => true,
            "project.rename" | PLACE_RESISTOR | CONNECT_PINS => self.project.is_some(),
            "edit.undo" => !self.undo.is_empty(),
            "edit.redo" => !self.redo.is_empty(),
            _ => false,
        }
    }

    /// Return the current project, revision, and undo and redo availability as JSON.
    /// The schematic's `wires` list identifies each link by its two terminal references.
    /// The `netlist` groups terminals joined by those wires and lists terminals with no wire as `unconnected`.
    pub fn inspect(&self) -> Value {
        json!({
            "project": self.project.as_ref().map(|p| json!({
                "name": p.name,
                "schematic": {
                    "symbols": p.resistors.iter().map(ResistorPlacement::inspect).collect::<Vec<_>>(),
                    "wires": p.wires.iter().copied().map(Wire::inspect).collect::<Vec<_>>(),
                    "netlist": inspect_netlist(&p.resistors, &p.wires),
                },
            })),
            "revision": self.revision,
            "can_undo": !self.undo.is_empty(),
            "can_redo": !self.redo.is_empty(),
        })
    }

    /// Run a command. On success, answers [`Self::inspect`]. On failure, nothing has changed.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value, CommandError> {
        let command = command(id).ok_or_else(|| CommandError::Unknown(id.to_owned()))?;
        check_params(command, params)?;
        if !self.available(id) {
            return Err(CommandError::Unavailable(match id {
                "project.rename" | PLACE_RESISTOR | CONNECT_PINS => "no project is open".to_owned(),
                "edit.undo" => "nothing to undo".to_owned(),
                "edit.redo" => "nothing to redo".to_owned(),
                _ => format!("`{id}` can't run now"),
            }));
        }
        match id {
            "project.new" => self.change(Project { name: name_param(params)?, resistors: Vec::new(), wires: Vec::new() }),
            "project.rename" => {
                let name = name_param(params)?;
                let mut project = self.open_project()?.clone();
                if project.name != name {
                    project.name = name;
                    self.change(project);
                }
            }
            PLACE_RESISTOR => self.place_resistor(params)?,
            CONNECT_PINS => self.connect_pins(params)?,
            "edit.undo" => self.step(false),
            "edit.redo" => self.step(true),
            _ => return Err(CommandError::Unknown(id.to_owned())),
        }
        Ok(self.inspect())
    }

    /// Borrow the current project or report that the command has no target.
    fn open_project(&self) -> Result<&Project, CommandError> {
        self.project.as_ref().ok_or_else(|| CommandError::Unavailable("no project is open".to_owned()))
    }

    /// Add a resistor from validated parameters if the caller's revision is still current.
    /// Use the resulting revision as the identity so undo and a new placement cannot reuse it.
    fn place_resistor(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        if project.resistors.len() >= MAX_SYMBOLS {
            return Err(CommandError::Unavailable(format!("the schematic already has {MAX_SYMBOLS} symbols")));
        }
        let mut project = project.clone();
        project.resistors.push(ResistorPlacement {
            ordinal: self.revision + 1,
            reference_number: project.resistors.len() + 1,
            position: Point { x_nm: params["x_nm"].as_i64().unwrap_or_default(), y_nm: params["y_nm"].as_i64().unwrap_or_default() },
        });
        self.change(project);
        Ok(())
    }

    /// Add one wire and undo step after validating both terminals against the current project.
    fn connect_pins(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let endpoint = |name: &str| {
            Terminal::parse(&params[name])
                .filter(|terminal| project.resistors.iter().any(|symbol| symbol.ordinal == terminal.symbol))
                .ok_or_else(|| CommandError::InvalidParams(format!("`{name}` must reference an existing symbol terminal")))
        };
        let mut terminals = [endpoint("from")?, endpoint("to")?];
        terminals.sort_unstable();
        if terminals[0] == terminals[1] {
            return Err(CommandError::InvalidParams("a wire needs two distinct terminals".to_owned()));
        }
        if project.wires.iter().any(|wire| wire.terminals == terminals) {
            return Err(CommandError::InvalidParams("these terminals are already joined by a wire".to_owned()));
        }
        if project.wires.len() >= MAX_WIRES {
            return Err(CommandError::Unavailable(format!("the schematic already has {MAX_WIRES} wires")));
        }
        let mut project = project.clone();
        project.wires.push(Wire { ordinal: self.revision + 1, terminals });
        self.change(project);
        Ok(())
    }

    /// Reject an edit prepared against a project state that has since changed.
    fn check_revision(&self, params: &Value) -> Result<(), CommandError> {
        if params["expected_revision"].as_u64() != Some(self.revision) {
            return Err(CommandError::Unavailable(format!("project changed, inspect the current revision ({}) and retry", self.revision)));
        }
        Ok(())
    }

    fn change(&mut self, project: Project) {
        self.undo.push(self.project.replace(project));
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.revision += 1;
    }

    fn step(&mut self, forward: bool) {
        let (from, to) = if forward { (&mut self.redo, &mut self.undo) } else { (&mut self.undo, &mut self.redo) };
        if let Some(state) = from.pop() {
            to.push(std::mem::replace(&mut self.project, state));
            self.revision += 1;
        }
    }
}

/// Params must be an object (or nothing) holding exactly the command's params.
fn check_params(command: &Command, params: &Value) -> Result<(), CommandError> {
    let empty = Map::new();
    let given = match params {
        Value::Null => &empty,
        Value::Object(o) => o,
        _ => return Err(CommandError::InvalidParams(format!("`{}` takes an object of parameters", command.id))),
    };
    if let Some(extra) = given.keys().find(|k| !command.params.iter().any(|p| p.name == k.as_str())) {
        return Err(CommandError::InvalidParams(format!("`{}` has no parameter `{extra}`", command.id)));
    }
    for param in command.params {
        let value = given.get(param.name).ok_or_else(|| CommandError::InvalidParams(format!("`{}` needs `{}`", command.id, param.name)))?;
        param.kind.validate(value).map_err(|message| CommandError::InvalidParams(format!("`{}` {message}", param.name)))?;
    }
    Ok(())
}

fn name_param(params: &Value) -> Result<String, CommandError> {
    let name = params["name"].as_str().unwrap_or_default().trim();
    if name.is_empty() {
        return Err(CommandError::InvalidParams("the name can't be empty".to_owned()));
    }
    if name.chars().count() > MAX_NAME {
        return Err(CommandError::InvalidParams(format!("the name can't be longer than {MAX_NAME} characters")));
    }
    if name.chars().any(char::is_control) {
        return Err(CommandError::InvalidParams("the name can't contain control characters".to_owned()));
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(session: &Session) -> Option<&str> {
        session.project().map(|p| p.name.as_str())
    }

    #[test]
    fn undo_and_redo_walk_the_history_and_the_revision_never_repeats() {
        let mut s = Session::default();
        s.execute("project.new", &json!({"name": "a"})).unwrap();
        s.execute("project.rename", &json!({"name": "b"})).unwrap();
        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(name(&s), Some("a"));
        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(name(&s), None);
        s.execute("edit.redo", &Value::Null).unwrap();
        assert_eq!(name(&s), Some("a"));
        assert_eq!(s.revision(), 5);
    }

    #[test]
    fn a_new_change_clears_redo() {
        let mut s = Session::default();
        s.execute("project.new", &json!({"name": "a"})).unwrap();
        s.execute("edit.undo", &Value::Null).unwrap();
        s.execute("project.new", &json!({"name": "b"})).unwrap();
        assert!(!s.available("edit.redo"));
    }

    #[test]
    fn a_refused_command_changes_nothing() {
        let mut s = Session::default();
        let before = s.inspect();
        for (id, params) in [
            ("no.such", Value::Null),
            ("project.rename", json!({"name": "x"})),
            ("edit.undo", Value::Null),
            ("project.new", json!({})),
            ("project.new", json!({"name": "  "})),
            ("project.new", json!({"name": 3})),
            ("project.new", json!({"name": "x", "extra": "y"})),
            ("project.new", json!({"name": "a\nb"})),
            ("project.new", json!({"name": "n".repeat(MAX_NAME + 1)})),
            ("project.new", json!([])),
        ] {
            assert!(s.execute(id, &params).is_err(), "{id} {params}");
            assert_eq!(s.inspect(), before, "{id} {params}");
        }
    }

    #[test]
    fn renaming_to_the_same_name_is_not_a_change() {
        let mut s = Session::default();
        s.execute("project.new", &json!({"name": "a"})).unwrap();
        s.execute("project.rename", &json!({"name": " a "})).unwrap();
        assert_eq!(s.revision(), 1);
    }

    #[test]
    fn placing_a_resistor_preserves_position_and_identity_through_history() {
        let mut s = Session::default();
        s.execute("project.new", &json!({"name": "blinky"})).unwrap();
        let place = |revision| json!({"x_nm": 500, "y_nm": -200, "expected_revision": revision});
        let state = s.execute(PLACE_RESISTOR, &place(1)).unwrap();
        let symbol = &state["project"]["schematic"]["symbols"][0];
        assert_ne!(symbol["id"], symbol["component_id"]);
        assert_eq!(symbol["reference"], "R1");
        assert_eq!(symbol["pins"][0], json!({"number": "1", "x_nm": 500 - schematic::PIN_OFFSET_NM, "y_nm": -200}));
        s.execute("project.rename", &json!({"name": "renamed"})).unwrap();
        assert_eq!(&s.inspect()["project"]["schematic"]["symbols"][0], symbol);
        s.execute("edit.undo", &Value::Null).unwrap();
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!(s.project().unwrap().resistors().is_empty());
        s.execute("edit.redo", &Value::Null).unwrap();
        assert_eq!(&s.inspect()["project"]["schematic"]["symbols"][0], symbol);
        s.execute("edit.undo", &Value::Null).unwrap();
        let state = s.execute(PLACE_RESISTOR, &place(s.revision())).unwrap();
        assert_ne!(state["project"]["schematic"]["symbols"][0]["id"], symbol["id"]);
        assert!(!s.available("edit.redo"));
    }

    /// Exercise rejected input against the full inspection snapshot, including the revision and undo state.
    #[test]
    fn invalid_placement_leaves_the_project_and_history_unchanged() {
        let mut s = Session::default();
        assert!(!s.available(PLACE_RESISTOR));
        let valid = json!({"x_nm": 0, "y_nm": 0, "expected_revision": 1});
        assert!(s.execute(PLACE_RESISTOR, &valid).is_err());
        s.execute("project.new", &json!({"name": "blinky"})).unwrap();
        let before = s.inspect();
        for params in [
            json!({"x_nm": 0, "y_nm": 0}),
            json!({"x_nm": "0", "y_nm": 0, "expected_revision": 1}),
            json!({"x_nm": 0.25, "y_nm": 0, "expected_revision": 1}),
            json!({"x_nm": MAX_CENTER_NM + 1, "y_nm": 0, "expected_revision": 1}),
            json!({"x_nm": 0, "y_nm": -MAX_CENTER_NM - 1, "expected_revision": 1}),
            json!({"x_nm": 0, "y_nm": 0, "expected_revision": 0}),
            json!({"x_nm": 0, "y_nm": 0, "expected_revision": -1}),
            json!({"x_nm": 0, "y_nm": 0, "expected_revision": 1, "extra": "pin"}),
        ] {
            assert!(s.execute(PLACE_RESISTOR, &params).is_err(), "{params}");
            assert_eq!(s.inspect(), before);
        }
        s.execute(PLACE_RESISTOR, &valid).unwrap();
    }

    #[test]
    fn history_is_bounded() {
        let mut s = Session::default();
        for i in 0..MAX_HISTORY + 5 {
            s.execute("project.new", &json!({"name": i.to_string()})).unwrap();
        }
        assert_eq!(s.undo.len(), MAX_HISTORY);
    }
}

#[cfg(test)]
mod wiring_tests {
    use super::*;

    /// Build three placements, including two with coincident pins, without connecting any of them.
    fn three_resistors() -> Session {
        let mut session = Session::default();
        session.execute("project.new", &json!({"name": "wiring"})).unwrap();
        for x_nm in [0, 0, 20 * schematic::GRID_NM] {
            session.execute(PLACE_RESISTOR, &json!({"x_nm": x_nm, "y_nm": 0, "expected_revision": session.revision()})).unwrap();
        }
        session
    }

    fn terminal(symbol: &str, number: &str) -> Value {
        json!({"symbol": symbol, "terminal": number})
    }

    #[test]
    fn explicit_wires_form_a_net_and_undo_restores_the_previous_membership() {
        let mut session = three_resistors();
        let a = terminal("symbol-2", "1");
        let b = terminal("symbol-3", "2");
        let c = terminal("symbol-4", "1");
        let initial = session.inspect();
        assert_eq!(initial["project"]["schematic"]["netlist"]["nets"], json!([]));
        assert_eq!(initial["project"]["schematic"]["netlist"]["unconnected"].as_array().unwrap().len(), 6);

        let first = session.execute(CONNECT_PINS, &json!({"from": b, "to": a, "expected_revision": 4})).unwrap();
        let second = session.execute(CONNECT_PINS, &json!({"from": c, "to": b, "expected_revision": 5})).unwrap();
        let schematic = &second["project"]["schematic"];
        assert_eq!(schematic["wires"][0], json!({"id": "wire-5", "from": a, "to": b}));
        assert_eq!(schematic["netlist"]["nets"], json!([{"terminals": [a, b, c]}]));
        assert_eq!(schematic["netlist"]["unconnected"].as_array().unwrap().len(), 3);

        let undone = session.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(undone["project"]["schematic"], first["project"]["schematic"]);
        let redone = session.execute("edit.redo", &Value::Null).unwrap();
        assert_eq!(&redone["project"]["schematic"], schematic);
    }

    /// Invalid endpoints, reverse duplicates and stale requests must leave the full snapshot intact.
    #[test]
    fn rejected_connections_do_not_change_the_project() {
        let mut session = three_resistors();
        let a = terminal("symbol-2", "1");
        let b = terminal("symbol-3", "2");
        session.execute(CONNECT_PINS, &json!({"from": a, "to": b, "expected_revision": 4})).unwrap();
        let before = session.inspect();
        for params in [
            json!({"from": b, "to": a, "expected_revision": 5}),
            json!({"from": a, "to": a, "expected_revision": 5}),
            json!({"from": a, "to": terminal("symbol-999", "1"), "expected_revision": 5}),
            json!({"from": a, "to": terminal("symbol-4", "3"), "expected_revision": 5}),
            json!({"from": a, "to": terminal("symbol-04", "1"), "expected_revision": 5}),
            json!({"from": a, "to": {"symbol": "symbol-4", "terminal": "1", "extra": true}, "expected_revision": 5}),
            json!({"from": a, "to": terminal("symbol-4", "1"), "expected_revision": 4}),
        ] {
            assert!(session.execute(CONNECT_PINS, &params).is_err(), "{params}");
            assert_eq!(session.inspect(), before);
        }
    }
}
