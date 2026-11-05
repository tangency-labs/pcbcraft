//! Where tool calls end up. Today that's a session inside this process. A running window is the
//! next backend: this trait is the seam, so an agent can work on the board a person has open.

use pcbcraft_engine::{Session, catalog};
use serde_json::{Value, json};

/// Something that answers `engine.commands`, `engine.execute` and `project.inspect`.
pub trait Backend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String>;
    /// Where the commands run, for the client's information.
    fn describe(&self) -> String;
}

/// A session of its own, with no window.
#[derive(Debug, Default)]
pub struct Headless {
    session: Session,
}

impl Backend for Headless {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        match method {
            "engine.commands" => Ok(json!(catalog().iter().map(|c| c.to_json()).collect::<Vec<_>>())),
            "engine.execute" => {
                let id = params["command"].as_str().ok_or("missing `command`")?;
                self.session.execute(id, &params["params"]).map_err(|e| e.to_string())
            }
            "project.inspect" => Ok(self.session.inspect()),
            _ => Err(format!("unknown method `{method}`")),
        }
    }

    fn describe(&self) -> String {
        "a headless session in this process, not an open window".to_owned()
    }
}
