//! Authored intent belongs to logical part terminals or wire ends; visible marks follow their owners.

use crate::{
    CommandError, Project, Session, SheetTarget,
    schematic::{ElectricalType, Point, Terminal, wiring::PinNumber},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Set or remove one logical terminal or wire end's intentional no-connect state.
pub const SET: &str = "schematic.set_no_connect";
/// Inspect no-connect intent and its focused unconnected-pin, dangling-end and conflict diagnostics.
pub const CHECK: &str = "schematic.check_no_connects";
/// New wires and attachment edits require explicit removal of authored termination intent.
pub const WIRE_BLOCKED: &str = "Remove the no-connect marker before connecting a wire.";
/// Bound persisted intent and detailed query allocation independently of total pin count.
pub const MAX_MARKERS: usize = 4096;
const MAX_FINDINGS: usize = 256;
mod wire_ends;
pub use wire_ends::{Attachment, WireAppearance, WireAttachments, WireFinding, WireMarker, WireTarget};

/// A stable logical terminal address, independent of unit geometry and reference text.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub part: String,
    pub terminal: String,
}

impl Target {
    /// Decode a canonical part identity and exact native terminal token, rejecting extra fields.
    pub fn parse(value: &Value) -> Option<Self> {
        let target: Self = serde_json::from_value(value.clone()).ok()?;
        target.ordinal()?;
        PinNumber::new(&target.terminal)?;
        Some(target)
    }

    /// Resolve the compatibility component ID spelling to its stable numeric identity.
    fn ordinal(&self) -> Option<u64> {
        crate::schematic::wiring::parse_id(&self.part, "component-")
    }
}

/// Saved evidence of the terminal meaning accepted when a marker was authored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Intent {
    pub terminal: String,
    pub electrical_type: ElectricalType,
}

/// Validate bounded sparse intent without silently retargeting missing/incompatible terminals.
pub(crate) fn read_intents(records: Vec<Intent>) -> Result<BTreeMap<String, ElectricalType>, String> {
    if records.is_empty() || records.len() > 128 {
        return Err("No-connect intent must be a nonempty list of at most 128 terminals.".into());
    }
    let mut intents = BTreeMap::new();
    for record in records {
        if PinNumber::new(&record.terminal).is_none() || intents.insert(record.terminal, record.electrical_type).is_some() {
            return Err("Invalid or duplicate no-connect terminal.".into());
        }
    }
    Ok(intents)
}

/// Encode only authored records, in exact terminal-token order.
pub(crate) fn write_intents(intents: &BTreeMap<String, ElectricalType>) -> Option<Vec<Intent>> {
    (!intents.is_empty()).then(|| intents.iter().map(|(terminal, &electrical_type)| Intent { terminal: terminal.clone(), electrical_type }).collect())
}

/// One current drawing of a logical terminal, suitable for navigation and marker painting.
#[derive(Clone, Debug, Serialize)]
pub struct Appearance {
    pub sheet: SheetTarget,
    pub symbol: String,
    pub at: Point,
}

/// A focused intent finding. Keys use rule and logical identity, never report ordering.
#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub key: String,
    pub rule: &'static str,
    pub severity: &'static str,
    pub target: Target,
    pub appearances: Vec<Appearance>,
}

/// Authored intent remains visible even when it cannot resolve to a compatible placed terminal.
#[derive(Clone, Debug, Serialize)]
pub struct Marker {
    pub target: Target,
    pub electrical_type: ElectricalType,
    pub appearances: Vec<Appearance>,
    pub issues: Vec<&'static str>,
}

/// Snapshot-qualified results; full type-conflict and power-driver ERC remain separate rules.
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub project: uuid::Uuid,
    pub revision: u64,
    pub markers: Vec<Marker>,
    pub total_markers: usize,
    pub findings: Vec<Finding>,
    pub total_findings: usize,
    pub truncated: bool,
    pub wire_markers: Vec<WireMarker>,
    pub wire_findings: Vec<WireFinding>,
}

impl Report {
    /// Count every finding while retaining a bounded detailed prefix for UI/MCP consumers.
    fn finding(&mut self, rule: &'static str, target: &Target, appearances: Vec<Appearance>) {
        self.total_findings += 1;
        if self.findings.len() < MAX_FINDINGS {
            self.findings.push(Finding {
                key: format!("{rule}/{}/{}", target.part, target.terminal),
                rule,
                severity: if rule == "unconnected_pin" { "warning" } else { "error" },
                target: target.clone(),
                appearances,
            });
        }
        self.truncated = self.total_findings > self.findings.len();
    }
}

impl Project {
    /// Enforce intent budgets at every publication/encoding boundary, including compound pastes.
    pub(crate) fn validate_no_connects(&self) -> Result<(), String> {
        if self.components.values().any(|part| part.no_connects.len() > 128 || part.no_connects.keys().any(|token| PinNumber::new(token).is_none()))
            || self.no_connect_count() > MAX_MARKERS
            || self.sheets.iter().any(|sheet| sheet.no_connect_ends.iter().any(|id| !sheet.junctions.contains_key(id)))
        {
            return Err("No-connect intent exceeds its terminal or project limits.".into());
        }
        Ok(())
    }

    /// Whether this logical terminal carries explicit no-connect intent, regardless of geometry.
    pub fn has_no_connect(&self, terminal: Terminal) -> bool {
        self.terminal_symbol(terminal).and_then(|symbol| self.component(symbol)).is_some_and(|part| part.no_connects.contains_key(terminal.number()))
    }

    /// Convert a picked appearance to the logical address shared by all units of its part.
    pub fn no_connect_target(&self, terminal: Terminal) -> Option<Target> {
        let symbol = self.terminal_symbol(terminal)?;
        self.terminal_position(terminal)?;
        Some(Target { part: symbol.component_id(), terminal: terminal.number().into() })
    }

    /// Check current connectivity without modifying graph caches' semantic contents or authored
    /// state. A wire stub or named contact counts as a connection; repeated bare appearances
    /// alone do not. Missing/type-changed records produce findings instead of suppressing others.
    pub fn check_no_connects(&self, revision: u64) -> Report {
        let mut locations = BTreeMap::new();
        let mut wired = BTreeSet::new();
        for sheet in self.sheets.iter() {
            let target = SheetTarget { project: self.id(), sheet: sheet.sheet_id };
            let Some(view) = self.for_sheet(target) else { continue };
            for symbol in view.symbols() {
                locations.insert(symbol.ordinal, (target, symbol.clone()));
                for terminal in symbol.terminals() {
                    if view.connectivity().net_index_for(terminal).is_some() {
                        wired.insert(terminal);
                    }
                }
            }
        }
        let appearances = |component, token: &str| {
            self.component_terminals(component, token)
                .filter_map(|terminal| {
                    let (sheet, symbol) = locations.get(&terminal.symbol)?;
                    let pin = symbol.unit_definition().pin_index(token)?;
                    Some(Appearance { sheet: *sheet, symbol: symbol.id(), at: *symbol.pins().get(pin)? })
                })
                .collect::<Vec<_>>()
        };
        let connected = |component, token: &str| self.component_terminals(component, token).any(|terminal| wired.contains(&terminal));
        let mut report = Report {
            project: self.id(),
            revision,
            markers: Vec::new(),
            total_markers: 0,
            findings: Vec::new(),
            total_findings: 0,
            truncated: false,
            wire_markers: Vec::new(),
            wire_findings: Vec::new(),
        };
        for part in self.components.values() {
            for (token, &expected) in &part.no_connects {
                let target = Target { part: part.id(), terminal: token.clone() };
                let positions = appearances(part.ordinal, token);
                let pin = part.definition.pins().iter().find(|pin| pin.number == *token);
                let mut issues = Vec::new();
                if pin.is_none() || positions.is_empty() {
                    issues.push("no_connect_missing_target");
                }
                if pin.is_some_and(|pin| pin.electrical_type != expected) {
                    issues.push("no_connect_incompatible_terminal");
                }
                if connected(part.ordinal, token) {
                    issues.push("no_connect_connected");
                }
                for &rule in &issues {
                    report.finding(rule, &target, positions.clone());
                }
                report.total_markers += 1;
                if report.markers.len() < MAX_FINDINGS {
                    report.markers.push(Marker { target, electrical_type: expected, appearances: positions, issues });
                }
            }
            for pin in part.definition.pins() {
                if !part.no_connects.contains_key(&pin.number)
                    && pin.electrical_type != ElectricalType::NoConnect
                    && self.component_terminal(part.ordinal, &pin.number).is_some()
                    && !connected(part.ordinal, &pin.number)
                {
                    let target = Target { part: part.id(), terminal: pin.number.clone() };
                    report.finding(
                        "unconnected_pin",
                        &target,
                        if report.findings.len() < MAX_FINDINGS { appearances(part.ordinal, &pin.number) } else { Vec::new() },
                    );
                }
            }
        }
        self.check_wire_no_connects(&mut report);
        report.truncated = report.total_findings > report.findings.len() + report.wire_findings.len()
            || report.total_markers > report.markers.len() + report.wire_markers.len();
        report
    }
}

impl Session {
    /// Author/remove only validated terminal or wire-end intent at the expected revision. An unchanged
    /// request is a no-op; incompatible retained records can always be explicitly removed.
    pub(crate) fn set_no_connect(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let attachment =
            Attachment::parse(&params["target"]).ok_or_else(|| CommandError::InvalidParams("Expected a part terminal or wire-end target.".into()))?;
        if let Attachment::WireEnd(target) = attachment {
            return self.set_wire_no_connect(&target, params);
        }
        let target = Target::parse(&params["target"]).ok_or_else(|| CommandError::InvalidParams("Expected a part terminal target.".into()))?;
        let id = target.ordinal().ok_or_else(|| CommandError::InvalidParams("Invalid part identity.".into()))?;
        let marked = params["marked"].as_bool().ok_or_else(|| CommandError::InvalidParams("marked must be a boolean.".into()))?;
        let original = self.open_project()?;
        let part = original.components.get(&id).ok_or_else(|| CommandError::InvalidParams("Unknown part instance.".into()))?;
        if !marked && !part.no_connects.contains_key(&target.terminal) && !part.definition.pins().iter().any(|pin| pin.number == target.terminal) {
            return Err(CommandError::InvalidParams("Unknown terminal.".into()));
        }
        let expected = if marked {
            if original.component_terminal(id, &target.terminal).is_none() {
                return Err(CommandError::InvalidParams("No placed appearance of this terminal.".into()));
            }
            Some(
                part.definition
                    .pins()
                    .iter()
                    .find(|pin| pin.number == target.terminal)
                    .ok_or_else(|| CommandError::InvalidParams("Unknown terminal.".into()))?
                    .electrical_type,
            )
        } else {
            None
        };
        if part.no_connects.get(&target.terminal).copied() == expected {
            return Ok(());
        }
        if expected.is_some()
            && !part.no_connects.contains_key(&target.terminal)
            && (part.no_connects.len() >= 128 || original.no_connect_count() >= MAX_MARKERS)
        {
            return Err(CommandError::Unavailable("No-connect marker limit reached.".into()));
        }
        let mut project =
            if original.layout.is_none() { original.with_board_folders(".").map_err(CommandError::InvalidParams)? } else { original.clone() };
        if !project.has_sheet_occurrences() {
            project = project.with_sheet_occurrences().map_err(CommandError::InvalidParams)?;
        }
        let part = Arc::make_mut(&mut project.components).get_mut(&id).ok_or_else(|| CommandError::InvalidParams("Unknown part instance.".into()))?;
        if let Some(electrical_type) = expected {
            part.no_connects.insert(target.terminal, electrical_type);
        } else {
            part.no_connects.remove(&target.terminal);
        }
        self.change(project)
    }

    /// Return one source-revision-qualified focused check through the normal shared query path.
    pub(crate) fn no_connect_report(&self) -> Result<Value, CommandError> {
        Ok(json!({"revision": self.revision(), "no_connect_check": self.open_project()?.check_no_connects(self.revision())}))
    }
}

#[cfg(test)]
mod tests;
