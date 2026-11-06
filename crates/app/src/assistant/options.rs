//! The model and reasoning effort each agent runs with, chosen in the Assistant's agent row as
//! SolidCraft's chat header offers them. Empty means the agent's own default; nothing is passed.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::agent::Agent;

/// The longest model name accepted, well above any provider's.
const MAX_MODEL_LEN: usize = 128;
/// The longest effort level accepted.
const MAX_EFFORT_LEN: usize = 16;
/// Effort levels offered when the agent doesn't say which its model supports.
const CLAUDE_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const CODEX_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh"];
/// Claude Code's aliases for its latest models, as `claude --help` documents them.
const CLAUDE_MODELS: &[(&str, &str)] = &[("fable", "Fable"), ("opus", "Opus"), ("sonnet", "Sonnet"), ("haiku", "Haiku")];

/// One agent's model and effort. Empty fields pass nothing, leaving the agent's default.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct AgentOptions {
    pub(super) model: String,
    pub(super) effort: String,
}

impl AgentOptions {
    /// Keep only values that are safe to pass as one argument; anything else becomes the default.
    pub(super) fn sanitized(self) -> Self {
        Self {
            model: if valid_model(&self.model) { self.model } else { String::new() },
            effort: if valid_effort(&self.effort) { self.effort } else { String::new() },
        }
    }

    /// The command-line arguments that select these options for `agent`, in order. Codex takes
    /// them after `exec`; Claude Code anywhere before its other flags.
    pub(super) fn cli_args(&self, agent: Agent) -> Vec<String> {
        let mut args = Vec::new();
        let options = self.clone().sanitized();
        match agent {
            Agent::Codex => {
                if !options.model.is_empty() {
                    args.extend(["-m".to_owned(), options.model]);
                }
                if !options.effort.is_empty() {
                    args.extend(["-c".to_owned(), format!("model_reasoning_effort=\"{}\"", options.effort)]);
                }
            }
            Agent::Claude => {
                if !options.model.is_empty() {
                    args.extend(["--model".to_owned(), options.model]);
                }
                if !options.effort.is_empty() {
                    args.extend(["--effort".to_owned(), options.effort]);
                }
            }
        }
        args
    }
}

/// A model name is one argument: letters, digits and `._:/[]-`, not starting with `-`, so it
/// can never read as a flag. Empty is valid and means the default.
pub(super) fn valid_model(model: &str) -> bool {
    model.len() <= MAX_MODEL_LEN && !model.starts_with('-') && model.chars().all(|c| c.is_ascii_alphanumeric() || "._:/[]-".contains(c))
}

/// An effort level is a short lowercase word. Empty is valid and means the default.
fn valid_effort(effort: &str) -> bool {
    effort.len() <= MAX_EFFORT_LEN && effort.chars().all(|c| c.is_ascii_lowercase())
}

/// A model the picker suggests, with the effort levels it supports when the agent says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Model {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) efforts: Vec<String>,
}

/// The models to suggest for `agent`. Claude Code's are its documented aliases. Codex's come
/// from the model list it caches in its home folder; when that can't be read there are none,
/// and a model can still be typed.
pub(super) fn models(agent: Agent) -> Vec<Model> {
    match agent {
        Agent::Claude => {
            CLAUDE_MODELS.iter().map(|(id, label)| Model { id: (*id).to_owned(), label: (*label).to_owned(), efforts: Vec::new() }).collect()
        }
        Agent::Codex => {
            codex_home().and_then(|home| std::fs::read(home.join("models_cache.json")).ok()).map(|bytes| codex_models(&bytes)).unwrap_or_default()
        }
    }
}

/// Codex's home folder: `CODEX_HOME`, or `.codex` in the home folder.
fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME").map(PathBuf::from).or_else(|| std::env::home_dir().map(|home| home.join(".codex")))
}

/// Read Codex's cached model list: listed models with valid names, and their effort levels.
/// Anything unreadable gives no suggestions rather than an error.
fn codex_models(bytes: &[u8]) -> Vec<Model> {
    const MAX_CACHE_BYTES: usize = 1 << 20;
    if bytes.len() > MAX_CACHE_BYTES {
        return Vec::new();
    }
    let Ok(cache) = serde_json::from_slice::<serde_json::Value>(bytes) else { return Vec::new() };
    cache["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|model| model["visibility"].as_str().is_none_or(|visibility| visibility == "list"))
        .filter_map(|model| {
            let id = model["slug"].as_str().filter(|id| !id.is_empty() && valid_model(id))?.to_owned();
            let label = model["display_name"].as_str().filter(|label| !label.trim().is_empty()).unwrap_or(&id).chars().take(64).collect();
            let efforts = model["supported_reasoning_levels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|level| level["effort"].as_str().or_else(|| level.as_str()))
                .filter(|effort| !effort.is_empty() && valid_effort(effort))
                .map(str::to_owned)
                .collect();
            Some(Model { id, label, efforts })
        })
        .take(64)
        .collect()
}

/// The effort levels to offer for `model`: the ones it lists, or the agent's usual levels.
pub(super) fn efforts(agent: Agent, model: &str, models: &[Model]) -> Vec<String> {
    match models.iter().find(|known| known.id == model).filter(|known| !known.efforts.is_empty()) {
        Some(known) => known.efforts.clone(),
        None => match agent {
            Agent::Claude => CLAUDE_EFFORTS,
            Agent::Codex => CODEX_EFFORTS,
        }
        .iter()
        .map(|effort| (*effort).to_owned())
        .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each CLI gets its own flags, and defaults pass nothing.
    #[test]
    fn options_become_each_clis_flags() {
        assert!(AgentOptions::default().cli_args(Agent::Claude).is_empty());
        let options = AgentOptions { model: "opus".to_owned(), effort: "high".to_owned() };
        assert_eq!(options.cli_args(Agent::Claude), ["--model", "opus", "--effort", "high"]);
        assert_eq!(options.cli_args(Agent::Codex), ["-m", "opus", "-c", "model_reasoning_effort=\"high\""]);
        let effort_only = AgentOptions { model: String::new(), effort: "xhigh".to_owned() };
        assert_eq!(effort_only.cli_args(Agent::Codex), ["-c", "model_reasoning_effort=\"xhigh\""]);
    }

    /// Values that could read as flags or break the TOML override fall back to the default.
    #[test]
    fn unsafe_values_fall_back_to_the_default() {
        for model in ["--dangerously-skip-permissions", "a model", "gpt\"x", &"m".repeat(200)] {
            assert!(!valid_model(model), "{model}");
        }
        for model in ["", "claude-fable-5-1[1m]", "gpt-6.1-sol", "org/model:latest"] {
            assert!(valid_model(model), "{model}");
        }
        let hostile = AgentOptions { model: "-x".to_owned(), effort: "high\" sandbox=\"none".to_owned() };
        assert!(hostile.cli_args(Agent::Codex).is_empty());
    }

    /// Codex's cache gives listed models with their levels; hidden, malformed or oversized
    /// entries give nothing.
    #[test]
    fn codex_models_come_from_its_cache() {
        let cache = br#"{"models":[
            {"slug":"gpt-a","display_name":"GPT A","visibility":"list","supported_reasoning_levels":[{"effort":"low"},{"effort":"max"}]},
            {"slug":"hidden","visibility":"hide"},
            {"slug":"-bad","visibility":"list"},
            {"slug":"gpt-b","visibility":"list"}
        ]}"#;
        let models = codex_models(cache);
        assert_eq!(models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(), ["gpt-a", "gpt-b"]);
        assert_eq!(efforts(Agent::Codex, "gpt-a", &models), ["low", "max"]);
        assert_eq!(efforts(Agent::Codex, "gpt-b", &models), CODEX_EFFORTS);
        assert_eq!(efforts(Agent::Claude, "opus", &models), CLAUDE_EFFORTS);
        assert!(codex_models(b"not json").is_empty());
        assert!(codex_models(&vec![b' '; 2 << 20]).is_empty());
    }
}
