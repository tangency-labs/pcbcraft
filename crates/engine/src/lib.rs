//! The session and command set behind PCBCraft.
//!
//! One rule shapes this crate: anything a person can do, an agent can do through the same command.
//! The menus and the MCP server both call [`Session::execute`] with an id from [`catalog`]; neither
//! changes the project any other way. This crate knows nothing about windows or transports.
//!
//! The project holds only a name so far. The board and schematic model belongs here, as more
//! commands; the file format is still being discussed (see `VISION.md`), so nothing saves yet.

use serde_json::{Map, Value, json};

/// Undo steps kept. The oldest is dropped past this.
const MAX_HISTORY: usize = 1000;
/// Longest project name, in characters.
const MAX_NAME: usize = 200;

/// A parameter of a command. All are strings for now.
#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub name: &'static str,
    pub summary: &'static str,
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
    pub fn to_json(&self) -> Value {
        let params: Map<String, Value> =
            self.params.iter().map(|p| (p.name.to_owned(), json!({"type": "string", "description": p.summary}))).collect();
        let required: Vec<&str> = self.params.iter().map(|p| p.name).collect();
        json!({
            "id": self.id,
            "title": self.title,
            "summary": self.summary,
            "params": {"type": "object", "properties": params, "required": required, "additionalProperties": false},
        })
    }
}

const NAME: Param = Param { name: "name", summary: "The project's name" };

const CATALOG: &[Command] = &[
    Command { id: "project.new", title: "New Project", summary: "Start a new empty project, replacing the open one. Undoable.", params: &[NAME] },
    Command { id: "project.rename", title: "Rename Project", summary: "Change the open project's name.", params: &[NAME] },
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
            "project.rename" => self.project.is_some(),
            "edit.undo" => !self.undo.is_empty(),
            "edit.redo" => !self.redo.is_empty(),
            _ => false,
        }
    }

    /// The whole state as JSON: what agents read, and what tests compare.
    pub fn inspect(&self) -> Value {
        json!({
            "project": self.project.as_ref().map(|p| json!({"name": p.name})),
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
                "project.rename" => "no project is open".to_owned(),
                "edit.undo" => "nothing to undo".to_owned(),
                "edit.redo" => "nothing to redo".to_owned(),
                _ => format!("`{id}` can't run now"),
            }));
        }
        match id {
            "project.new" => self.change(Project { name: name_param(params)? }),
            "project.rename" => {
                let name = name_param(params)?;
                if self.project.as_ref().is_some_and(|p| p.name == name) {
                    return Ok(self.inspect());
                }
                self.change(Project { name });
            }
            "edit.undo" => self.step(false),
            "edit.redo" => self.step(true),
            _ => return Err(CommandError::Unknown(id.to_owned())),
        }
        Ok(self.inspect())
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

/// Params must be an object (or nothing) holding exactly the command's params, each a string.
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
        match given.get(param.name) {
            Some(Value::String(_)) => {}
            Some(_) => return Err(CommandError::InvalidParams(format!("`{}` must be a string", param.name))),
            None => return Err(CommandError::InvalidParams(format!("`{}` needs `{}`", command.id, param.name))),
        }
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
    fn history_is_bounded() {
        let mut s = Session::default();
        for i in 0..MAX_HISTORY + 5 {
            s.execute("project.new", &json!({"name": i.to_string()})).unwrap();
        }
        assert_eq!(s.undo.len(), MAX_HISTORY);
    }
}
