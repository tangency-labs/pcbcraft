//! Complete, read-only pin/pad correspondence over one immutable accepted project state.
use super::{Pad, view::BoardTarget};
use crate::{Project, Session, SheetTarget, schematic::Terminal};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

/// A placed symbol terminal with its explicit project and sheet ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalTarget {
    pub sheet: SheetTarget,
    pub terminal: Terminal,
}

/// A physical pad with explicit project and board ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadTarget {
    pub board: BoardTarget,
    pub pad: Pad,
}

/// The inspected endpoint; coordinates and display names never establish identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Terminal(TerminalTarget),
    Pad(PadTarget),
}

/// Strict wire vocabulary for the additive resolver tool, independent of legacy cross_probe.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Address {
    Terminal { project: Uuid, sheet: Uuid, symbol: String, terminal: String },
    Pad { project: Uuid, board: Uuid, footprint: String, pad: String },
}

impl Source {
    /// Discoverable strict address vocabulary, matching the parser's explicit ownership fields.
    pub fn schema() -> Value {
        json!({"type":"object","oneOf":[
            {"type":"object","additionalProperties":false,"required":["kind","project","sheet","symbol","terminal"],"properties":{
                "kind":{"const":"terminal"},"project":{"type":"string","format":"uuid"},"sheet":{"type":"string","format":"uuid"},
                "symbol":{"type":"string","pattern":"^symbol-[1-9][0-9]*$"},"terminal":{"type":"string","pattern":"^[A-Za-z0-9+_-]{1,8}$"}}},
            {"type":"object","additionalProperties":false,"required":["kind","project","board","footprint","pad"],"properties":{
                "kind":{"const":"pad"},"project":{"type":"string","format":"uuid"},"board":{"type":"string","format":"uuid"},
                "footprint":{"type":"string","pattern":"^footprint-[1-9][0-9]*$"},"pad":{"type":"string","pattern":"^[A-Za-z0-9+_-]{1,8}$"}}}
        ]})
    }

    /// Parse only explicit qualified addresses. Existence is checked against the captured project.
    pub fn parse(value: &Value) -> Result<Self, String> {
        match serde_json::from_value::<Address>(value.clone()).map_err(|error| error.to_string())? {
            Address::Terminal { project, sheet, symbol, terminal } => {
                let terminal = Terminal::parse(&json!({"symbol":symbol,"terminal":terminal})).ok_or("invalid schematic terminal")?;
                Ok(Self::Terminal(TerminalTarget { sheet: SheetTarget { project, sheet }, terminal }))
            }
            Address::Pad { project, board, footprint, pad } => {
                let pad = Pad::parse(&footprint, &pad).ok_or("invalid board pad")?.on_board(board);
                Ok(Self::Pad(PadTarget { board: BoardTarget { project, board }, pad }))
            }
        }
    }

    /// Stable tagged address used by query results and the shared MCP boundary.
    pub fn inspect(self) -> Value {
        match self {
            Self::Terminal(target) => {
                let mut value = target.terminal.inspect();
                value["kind"] = json!("terminal");
                value["project"] = json!(target.sheet.project);
                value["sheet"] = json!(target.sheet.sheet);
                value
            }
            Self::Pad(target) => {
                let mut value = target.pad.on_board(target.board.board).inspect();
                value["kind"] = json!("pad");
                value["project"] = json!(target.board.project);
                value
            }
        }
    }

    /// Project ownership is explicit for either endpoint type.
    fn project(self) -> Uuid {
        match self {
            Self::Terminal(target) => target.sheet.project,
            Self::Pad(target) => target.board.project,
        }
    }
}

/// Why a complete counterpart set is available or unavailable; no guessed owner fallback is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Resolved,
    NoProject,
    NoBoard,
    ForeignProject,
    ForeignSheet,
    ForeignBoard,
    Stale,
    MissingTarget,
    Unassigned,
    UnplacedPackage,
    UnplacedSymbol,
    UnresolvedMapping,
    UnconnectedPad,
}

/// All placed appearances and lands of one logical terminal, or an explicit unavailable state.
/// Unavailable results can retain validated source-side endpoints but never invent counterparts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Counterparts {
    pub source: Source,
    pub status: Status,
    pub component: Option<String>,
    pub terminals: Vec<TerminalTarget>,
    pub pads: Vec<PadTarget>,
}

impl Counterparts {
    /// Start with no resolved identities so invalid/stale sources cannot leak another target.
    fn empty(source: Source, status: Status) -> Self {
        Self { source, status, component: None, terminals: Vec::new(), pads: Vec::new() }
    }

    /// Versioned query data; legacy cross_probe keeps its singular response unchanged.
    pub fn inspect(&self) -> Value {
        json!({"format":"pcbcraft-counterparts/1","source":self.source.inspect(),"status":self.status,"component":self.component,
            "terminals":self.terminals.iter().map(|target| Source::Terminal(*target).inspect()).collect::<Vec<_>>(),
            "pads":self.pads.iter().map(|target| Source::Pad(*target).inspect()).collect::<Vec<_>>()})
    }
}

impl Project {
    /// Resolve complete correspondence from accepted placements and exact pinned definitions.
    /// Desired assignments never replace an existing placement's mapping during inspection.
    /// This bounded query changes no caches, authored bytes, revision, library data or history.
    pub fn resolve_counterparts(&self, source: Source) -> Counterparts {
        let mut result = Counterparts::empty(source, Status::MissingTarget);
        if source.project() != self.id() {
            result.status = Status::ForeignProject;
            return result;
        }
        match source {
            Source::Terminal(target) => {
                let Some(sheet) = self.for_sheet(target.sheet) else {
                    result.status = Status::ForeignSheet;
                    return result;
                };
                let Some(symbol) = sheet.terminal_symbol(target.terminal) else { return result };
                if symbol.unit_definition().pin_index(target.terminal.number()).is_none() {
                    return result;
                }
                self.resolve_terminal_link(symbol.component, target.terminal.number(), &mut result);
            }
            Source::Pad(target) => {
                let Some(board) = self.board_target() else {
                    result.status = Status::NoBoard;
                    return result;
                };
                if board != target.board || target.pad.board.is_some_and(|id| id != board.board) {
                    result.status = Status::ForeignBoard;
                    return result;
                }
                let Some(footprint) = self.board.footprint_for_pad(target.pad) else { return result };
                let numbers = self.board.pad_numbers(footprint);
                if numbers.is_empty() {
                    result.status = Status::UnresolvedMapping;
                    return result;
                }
                if !numbers.contains(&target.pad.number) {
                    return result;
                }
                result.component = Some(format!("component-{}", footprint.component));
                result.pads.push(PadTarget { board, pad: target.pad.on_board(board.board) });
                if footprint.unconnected_pads.iter().any(|number| number == target.pad.number.as_str()) {
                    result.status = Status::UnconnectedPad;
                    return result;
                }
                let Some((component, pin)) = footprint.component_pin(target.pad.number) else {
                    result.status = Status::UnresolvedMapping;
                    return result;
                };
                self.resolve_terminal_link(component, pin, &mut result);
            }
        }
        result
    }

    /// Find every placed appearance through the existing component/unit index and sheet ownership.
    fn counterpart_terminals(&self, component: u64, pin: &str) -> Vec<TerminalTarget> {
        self.component_terminals(component, pin)
            .filter_map(|terminal| {
                let sheet = self.sheets.iter().find(|sheet| sheet.symbols.binary_search_by_key(&terminal.symbol, |symbol| symbol.ordinal).is_ok())?;
                Some(TerminalTarget { sheet: SheetTarget { project: self.id(), sheet: sheet.sheet_id }, terminal })
            })
            .collect()
    }

    /// Resolve a logical terminal using actual placement mappings, including all physical lands.
    /// Distinguish package intent without placement from missing or inconsistent accepted mappings.
    fn resolve_terminal_link(&self, component: u64, pin: &str, result: &mut Counterparts) {
        result.component = Some(format!("component-{component}"));
        result.terminals = self.counterpart_terminals(component, pin);
        let Some(board) = self.board_target() else {
            result.status = Status::NoBoard;
            return;
        };
        let Some(footprint) = self.board.by_component.get(&component).and_then(|index| self.board.footprints().get(*index)) else {
            result.status = match self.package_assignments().get(&component) {
                None => Status::Unassigned,
                Some(assignment) if self.assignment_definition(&assignment.key()).is_none() => Status::UnresolvedMapping,
                Some(_) => Status::UnplacedPackage,
            };
            return;
        };
        let mut numbers = footprint.pads_for(pin);
        let actual = self.board.pad_numbers(footprint);
        if numbers.is_empty() || numbers.iter().any(|number| !actual.contains(number)) {
            result.status = Status::UnresolvedMapping;
            return;
        }
        numbers.sort();
        numbers.dedup();
        result.pads =
            numbers.into_iter().map(|number| PadTarget { board, pad: Pad { board: Some(board.board), footprint: footprint.id, number } }).collect();
        result.status = if !result.terminals.is_empty() {
            Status::Resolved
        } else if self.components.get(&component).is_some_and(|part| part.definition.pin_index(pin).is_some()) {
            Status::UnplacedSymbol
        } else {
            Status::UnresolvedMapping
        };
    }
}

impl Session {
    /// Resolve against one captured revision. Stale requests return no counterpart identities,
    /// including after Undo/Redo or reopening the same project in this session.
    pub fn resolve_counterparts(&self, source: Source, expected_revision: u64) -> Counterparts {
        if expected_revision != self.revision() {
            return Counterparts::empty(source, Status::Stale);
        }
        self.project().map_or_else(|| Counterparts::empty(source, Status::NoProject), |project| project.resolve_counterparts(source))
    }
}

#[cfg(test)]
mod tests;
