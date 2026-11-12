//! Immutable schematic-to-board inputs and deterministic reviewed update planning.
//!
//! Baselines retain historical component and terminal data independently of live
//! schematic placements. Session commands own revision authority and publication;
//! these helpers validate and construct candidates without mutating a session.

use super::{Board, Footprint};
use crate::{MAX_OBJECT_ID, MAX_SYMBOLS, Project, schematic::PinNumber};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, Visitor},
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use uuid::Uuid;

/// Explicitly adopt current supported logical inputs before retaining physical removals.
pub const INITIALIZE_BASELINE: &str = "pcb.initialize_update_baseline";
mod commands;
pub(crate) use commands::CachedPlan;
pub use commands::{APPLY_UPDATE, BoardUpdatePreview, CANCEL_UPDATE, PREVIEW_UPDATE};

/// Persisted accepted-input vocabulary; future shapes require an explicit migration.
pub const FORMAT: &str = "pcbcraft-board-baseline/1";
const MULTIPAD_FORMAT: &str = "pcbcraft-board-baseline/2";
/// Bound decoded history independently of the live component count.
pub const MAX_BASELINE_BYTES: usize = 16 * 1024 * 1024;

/// Bound serialization as bytes are produced, including very large individual net records.
struct BaselineWriter {
    bytes: Vec<u8>,
}

impl std::io::Write for BaselineWriter {
    /// Refuse an oversized write before allocation; grow only within the encoded-byte budget.
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let length = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|length| *length <= MAX_BASELINE_BYTES)
            .ok_or_else(|| std::io::Error::other("board baseline exceeds its byte limit"))?;
        if length > self.bytes.capacity() {
            let capacity = self.bytes.capacity().max(2048).saturating_mul(2).max(length).min(MAX_BASELINE_BYTES);
            self.bytes.try_reserve_exact(capacity - self.bytes.len()).map_err(std::io::Error::other)?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    /// Serialization is entirely in memory and has no external flush side effect.
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A frozen input's incarnation and canonical content digest. Equal bytes from a
/// separately captured allocation history cannot alias this source identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCapture {
    pub capture_id: Uuid,
    pub content_digest: String,
}

impl SourceCapture {
    /// Check a caller-created capture address. The caller freezes and hashes its
    /// source bundle before attaching this new capture. Prior accepted history
    /// remains part of that exact immutable input, without self-reference.
    pub fn validate(&self) -> Result<(), String> {
        if self.capture_id.is_nil()
            || self.content_digest.len() != 64
            || !self.content_digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid board source capture identity".into());
        }
        Ok(())
    }
}

/// A logical terminal remains addressable after its schematic symbol disappears.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalKey {
    pub component: u64,
    pub terminal: String,
}

/// One exact desired or accepted physical choice, including explicit mapping.
/// Legacy two-pad markers have no library and remain a distinct compatibility case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub library: Option<String>,
    pub definition: String,
    pub revision: u32,
    #[serde(deserialize_with = "mapping_records")]
    pub mapping: BTreeMap<String, crate::library::parts::PadIds>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unconnected_pads: Vec<String>,
}

impl Package {
    /// Historical multi-pad and unconnected-pad choices retain an explicit version guard.
    pub(crate) fn extended(&self) -> bool {
        crate::library::parts::mapping::extended(&self.mapping, &self.unconnected_pads)
    }
    /// Capture a pinned assignment without copying its placement or coordinates.
    pub(crate) fn from_assignment(assignment: &super::assignment::Assignment) -> Self {
        Self {
            library: Some(assignment.library.clone()),
            definition: assignment.definition.clone(),
            revision: assignment.definition_revision,
            mapping: assignment.mapping.clone(),
            unconnected_pads: assignment.unconnected_pads.clone(),
        }
    }

    /// Preserve a physical or legacy placement's exact logical pad correspondence.
    fn from_footprint(footprint: &Footprint) -> Self {
        let mapping = footprint.mapping.clone().unwrap_or_else(|| {
            BTreeMap::from([("1".into(), footprint.pin_1_pad.to_string().into()), ("2".into(), (3 - footprint.pin_1_pad).to_string().into())])
        });
        Self {
            library: footprint.library.clone(),
            definition: footprint.definition.clone(),
            revision: footprint.definition_revision,
            mapping,
            unconnected_pads: footprint.unconnected_pads.clone(),
        }
    }

    /// Validate bounded complete pad coverage with exclusive physical-pad ownership.
    fn validate(&self, terminals: &[String]) -> Result<(), String> {
        let keys: BTreeSet<_> = self.mapping.keys().cloned().collect();
        let values: BTreeSet<_> = self.mapping.values().flat_map(|pads| pads.pads()).chain(&self.unconnected_pads).collect();
        if self.revision == 0
            || !super::definition::valid_id(&self.definition)
            || self.library.as_ref().is_some_and(|library| !super::definition::valid_id(library))
            || keys != terminals.iter().cloned().collect()
            || values.len() > 128
            || self.mapping.values().map(|pads| pads.pads().len()).sum::<usize>() + self.unconnected_pads.len() != values.len()
            || self.mapping.values().any(|value| !value.valid())
            || self.unconnected_pads.iter().any(|pad| PinNumber::new(pad).is_none())
        {
            return Err("invalid accepted package identity or terminal mapping".into());
        }
        if self.library.is_none()
            && (self.definition != super::FOOTPRINT_DEFINITION
                || self.revision != super::FOOTPRINT_REVISION
                || keys != BTreeSet::from(["1".into(), "2".into()])
                || values != BTreeSet::from([&"1".to_owned(), &"2".to_owned()]))
        {
            return Err("invalid legacy marker package".into());
        }
        Ok(())
    }

    /// Resolve physical pads from the exact retained package; never consult a newer library.
    fn validate_assets(&self, board: &Board) -> Result<(), String> {
        let Some(library) = &self.library else { return Ok(()) };
        let key = (library.clone(), self.definition.clone(), self.revision);
        let definition = board.definitions.get(&key).ok_or("missing pinned board-update package")?;
        let pads: BTreeSet<_> = definition.pads.iter().map(|pad| pad.number.as_str()).collect();
        if pads
            != self
                .mapping
                .values()
                .flat_map(|pads| pads.pads().iter().map(String::as_str))
                .chain(self.unconnected_pads.iter().map(String::as_str))
                .collect()
        {
            return Err("accepted package mapping does not cover its exact pinned pads".into());
        }
        Ok(())
    }

    /// Construct an accepted assignment only for real physical package definitions.
    fn assignment(&self, component: u64) -> Option<super::assignment::Assignment> {
        Some(super::assignment::Assignment {
            unconnected_pads: self.unconnected_pads.clone(),
            component,
            library: self.library.clone()?,
            definition: self.definition.clone(),
            definition_revision: self.revision,
            mapping: self.mapping.clone(),
        })
    }
}

/// Enough historical logical information to inspect a retained footprint without
/// a live component, a schematic placement or an installed symbol source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentState {
    pub library: String,
    pub definition: String,
    pub revision: u32,
    pub reference: String,
    pub value: String,
    pub terminals: Vec<String>,
    pub package: Option<Package>,
}

/// Historical terminal membership, not a second editable connectivity graph or
/// a persistent net ID. Names are display metadata and cannot establish identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetState {
    pub terminals: Vec<TerminalKey>,
    pub names: Vec<String>,
}

/// A proposed or accepted immutable logical input for one addressed physical board.
/// Only an accepted transaction may publish a proposed value as the board baseline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    pub format: String,
    pub project: Uuid,
    pub board: Uuid,
    pub source: SourceCapture,
    #[serde(deserialize_with = "component_records")]
    pub components: BTreeMap<u64, ComponentState>,
    pub nets: Vec<NetState>,
}

impl Baseline {
    /// Create an empty accepted input for a newly created board with no prior forward update.
    pub fn empty(project: Uuid, board: Uuid, source: SourceCapture) -> Result<Self, String> {
        let baseline = Self { format: FORMAT.into(), project, board, source, components: BTreeMap::new(), nets: Vec::new() };
        baseline.validate()?;
        Ok(baseline)
    }

    /// Capture physically eligible component and cross-sheet connectivity inputs.
    /// `desired` is authoritative package intent; excluded live choices are retained
    /// elsewhere without becoming physical input. This does not accept changes,
    /// allocate physical objects or alter the project.
    pub fn proposed(project: &Project, desired: &BTreeMap<u64, Package>, source: SourceCapture) -> Result<Self, String> {
        Self::capture(project, desired, source, false)
    }

    /// Capture ordinary physical input, or adopt an existing physical predecessor.
    /// Adoption also includes excluded parts that are already placed, preserving
    /// their real packages until the first reviewed update proposes their removal.
    fn capture(project: &Project, desired: &BTreeMap<u64, Package>, source: SourceCapture, adopt_placed: bool) -> Result<Self, String> {
        source.validate()?;
        let target = project.boards().next().map(|board| board.target).ok_or("no physical board is available")?;
        if desired.keys().any(|id| !project.components.contains_key(id)) {
            return Err("desired package points to an unknown live component".into());
        }
        let mut components = BTreeMap::new();
        let mut terminal_keys = BTreeMap::new();
        for (&id, component) in project.components.iter() {
            if !project.component_is_board_eligible(id) && !(adopt_placed && project.board.by_component.contains_key(&id)) {
                continue;
            }
            let mut terminals: Vec<_> = component.definition.pins().iter().map(|pin| pin.number.as_str().to_owned()).collect();
            terminals.sort();
            for pin in &terminals {
                for terminal in project.component_terminals(id, pin) {
                    terminal_keys.insert(terminal, TerminalKey { component: id, terminal: pin.clone() });
                }
            }
            components.insert(
                id,
                ComponentState {
                    library: component.definition.library.clone(),
                    definition: component.definition.id.clone(),
                    revision: component.definition.revision(),
                    reference: component.reference.clone(),
                    value: component.value.clone(),
                    terminals,
                    package: desired.get(&id).cloned(),
                },
            );
        }
        let graph = project.electrical_connectivity();
        let mut nets = Vec::new();
        for (index, terminals) in graph.nets.iter().enumerate() {
            let mut terminals: Vec<_> = terminals.iter().filter_map(|pin| terminal_keys.get(pin).cloned()).collect();
            terminals.sort();
            terminals.dedup();
            let multiple_lands = terminals.iter().any(|terminal| {
                desired.get(&terminal.component).and_then(|package| package.mapping.get(&terminal.terminal)).is_some_and(|pads| pads.pads().len() > 1)
            });
            if terminals.len() >= 2 || (!terminals.is_empty() && (!graph.net_names(index).is_empty() || multiple_lands)) {
                nets.push(NetState { terminals, names: graph.net_names(index).to_vec() });
            }
        }
        // Unwired terminals are absent from the electrical graph, but their multiple
        // lands still require copper joining them on this physical board.
        let connected: BTreeSet<_> = nets.iter().flat_map(|net| net.terminals.iter().cloned()).collect();
        for (&component, state) in &components {
            if let Some(package) = &state.package {
                for (terminal, pads) in &package.mapping {
                    let key = TerminalKey { component, terminal: terminal.clone() };
                    if pads.pads().len() > 1 && !connected.contains(&key) {
                        nets.push(NetState { terminals: vec![key], names: Vec::new() });
                    }
                }
            }
        }
        nets.sort_by(|left, right| left.terminals.cmp(&right.terminals));
        let format =
            if components.values().any(|component| component.package.as_ref().is_some_and(Package::extended)) { MULTIPAD_FORMAT } else { FORMAT };
        let baseline = Self { format: format.into(), project: target.project, board: target.board, source, components, nets };
        baseline.validate()?;
        baseline.validate_assets(project)?;
        baseline.encode()?;
        for (&id, component) in &baseline.components {
            if let Some(package) = &component.package
                && package.library.is_none()
                && !project.board.footprints().iter().any(|footprint| footprint.component == id && Package::from_footprint(footprint) == *package)
            {
                return Err("a legacy marker choice requires its existing accepted placement".into());
            }
        }
        Ok(baseline)
    }

    /// Collect existing assignment intent, falling back to an unchanged placed
    /// package. This is compatibility initialization, not an implicit acceptance.
    pub fn current_choices(project: &Project) -> BTreeMap<u64, Package> {
        let mut choices: BTreeMap<_, _> = project
            .board
            .footprints()
            .iter()
            .filter(|footprint| project.components.contains_key(&footprint.component))
            .map(|footprint| (footprint.component, Package::from_footprint(footprint)))
            .collect();
        choices.extend(
            project
                .board
                .assignments
                .iter()
                .filter(|(component, _)| project.components.contains_key(component))
                .map(|(&component, assignment)| (component, Package::from_assignment(assignment))),
        );
        choices
    }

    /// Decode a bounded standalone baseline and reject malformed/cross-linked history.
    /// Asset resolution remains explicit because removed sources need not be live components.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BASELINE_BYTES {
            return Err("board baseline exceeds its byte limit".into());
        }
        let baseline: Self = serde_json::from_slice(bytes).map_err(|error| format!("invalid board baseline: {error}"))?;
        baseline.validate()?;
        Ok(baseline)
    }

    /// Serialize only a validated bounded baseline; a failed encode returns no partial bytes.
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        use std::io::Write;
        self.validate()?;
        let mut bytes = BaselineWriter { bytes: Vec::new() };
        write!(
            &mut bytes,
            "{{\n  \"format\": \"{}\",\n  \"project\": \"{}\",\n  \"board\": \"{}\",\n  \"source\": ",
            self.format, self.project, self.board
        )
        .map_err(|error| error.to_string())?;
        serde_json::to_writer(&mut bytes, &self.source).map_err(|error| error.to_string())?;
        bytes.write_all(b",\n  \"components\": {\n").map_err(|error| error.to_string())?;
        for (index, (id, component)) in self.components.iter().enumerate() {
            if index != 0 {
                bytes.write_all(b"    ,\n").map_err(|error| error.to_string())?;
            }
            write!(&mut bytes, "    \"{id}\":").map_err(|error| error.to_string())?;
            serde_json::to_writer(&mut bytes, component).map_err(|error| error.to_string())?;
            bytes.write_all(b"\n").map_err(|error| error.to_string())?;
        }
        bytes.write_all(b"  },\n  \"nets\": [\n").map_err(|error| error.to_string())?;
        for (index, net) in self.nets.iter().enumerate() {
            if index != 0 {
                bytes.write_all(b"    ,\n").map_err(|error| error.to_string())?;
            }
            bytes.write_all(b"    ").map_err(|error| error.to_string())?;
            serde_json::to_writer(&mut bytes, net).map_err(|error| error.to_string())?;
            bytes.write_all(b"\n").map_err(|error| error.to_string())?;
        }
        bytes.write_all(b"  ]\n}").map_err(|error| error.to_string())?;
        Ok(bytes.bytes)
    }

    /// Require bounded, canonical records and exact membership without resolving
    /// historical component IDs against the current schematic.
    pub fn validate(&self) -> Result<(), String> {
        self.source.validate()?;
        if !matches!(self.format.as_str(), FORMAT | MULTIPAD_FORMAT)
            || (self.format == FORMAT && self.components.values().any(|component| component.package.as_ref().is_some_and(Package::extended)))
            || self.project.is_nil()
            || self.board.is_nil()
            || self.components.len() > MAX_SYMBOLS
        {
            return Err("unsupported or invalid board baseline header".into());
        }
        for (&id, component) in &self.components {
            if !(1..=MAX_OBJECT_ID).contains(&id)
                || component.revision == 0
                || !super::definition::valid_id(&component.library)
                || !super::definition::valid_id(&component.definition)
                || component.terminals.is_empty()
                || component.terminals.len() > 128
                || !strictly_sorted(&component.terminals)
                || component.terminals.iter().any(|terminal| PinNumber::new(terminal).is_none())
            {
                return Err("invalid historical component identity or terminal set".into());
            }
            crate::schematic::check_reference(&component.reference)?;
            crate::schematic::check_value(&component.value)?;
            if let Some(package) = &component.package {
                package.validate(&component.terminals)?;
            }
        }
        if self.nets.len() > MAX_SYMBOLS * 64 || !self.nets.windows(2).all(|pair| pair[0].terminals < pair[1].terminals) {
            return Err("board baseline nets exceed bounds or are not canonical".into());
        }
        let mut seen = BTreeSet::new();
        for net in &self.nets {
            if net.terminals.is_empty()
                || (net.terminals.len() < 2
                    && net.names.is_empty()
                    && !net.terminals.iter().any(|terminal| {
                        self.components
                            .get(&terminal.component)
                            .and_then(|component| component.package.as_ref())
                            .and_then(|package| package.mapping.get(&terminal.terminal))
                            .is_some_and(|pads| pads.pads().len() > 1)
                    }))
                || net.terminals.len() > MAX_SYMBOLS * 128
                || !strictly_sorted(&net.terminals)
                || net.names.len() > crate::net_labels::MAX_LABELS * crate::sheets::MAX_SHEETS
            {
                return Err("invalid accepted net membership".into());
            }
            let mut names = BTreeSet::new();
            for name in &net.names {
                if name.is_empty() || name.trim() != name || name.chars().count() > 200 || name.chars().any(char::is_control) || !names.insert(name) {
                    return Err("invalid accepted net name".into());
                }
            }
            for terminal in &net.terminals {
                if !seen.insert(terminal)
                    || !self
                        .components
                        .get(&terminal.component)
                        .is_some_and(|component| component.terminals.binary_search(&terminal.terminal).is_ok())
                {
                    return Err("accepted net has a duplicate or unknown component terminal".into());
                }
            }
        }
        Ok(())
    }

    /// Resolve retained package bytes independently of live schematic membership.
    /// When a live component names the exact same immutable symbol revision, its
    /// pins must agree with historical terminals; distinct revisions and removed
    /// sources retain their independently validated historical terminal contracts.
    pub fn validate_assets(&self, project: &Project) -> Result<(), String> {
        for (&id, component) in &self.components {
            if let Some(live) = project.components.get(&id)
                && component.library == live.definition.library
                && component.definition == live.definition.id
                && component.revision == live.definition.revision()
            {
                let live_terminals: BTreeSet<_> = live.definition.pins().iter().map(|pin| pin.number.as_str()).collect();
                if component.terminals.iter().map(String::as_str).collect::<BTreeSet<_>>() != live_terminals {
                    return Err("accepted terminal set contradicts the same pinned symbol revision".into());
                }
            }
            if let Some(package) = &component.package {
                package.validate_assets(&project.board)?;
            }
        }
        Ok(())
    }

    /// Keep physical definitions referenced by accepted history even after their
    /// last live assignment or schematic component is removed.
    pub(crate) fn package_keys(&self) -> impl Iterator<Item = super::definition::Key> + '_ {
        self.components.values().filter_map(|component| {
            let package = component.package.as_ref()?;
            Some((package.library.clone()?, package.definition.clone(), package.revision))
        })
    }

    /// Validate a physical placement through its accepted source without requiring
    /// that component to remain present in the live schematic.
    pub(crate) fn validate_footprint(&self, footprint: &Footprint) -> Result<(), String> {
        if self.components.get(&footprint.component).and_then(|component| component.package.as_ref()) != Some(&Package::from_footprint(footprint)) {
            return Err("footprint differs from its accepted component package".into());
        }
        Ok(())
    }

    /// Validate an accepted unplaced package without resolving live component state.
    pub(crate) fn validate_assignment(&self, assignment: &super::assignment::Assignment) -> bool {
        self.components.get(&assignment.component).and_then(|component| component.package.as_ref()) == Some(&Package::from_assignment(assignment))
    }

    /// Build a snapshot-bound terminal lookup once for a presentation traversal.
    /// Borrowed terminal keys stay local to the immutable baseline; public edits
    /// to net records cannot leave a persistent derived cache stale.
    pub(crate) fn terminal_net_index(&self) -> BTreeMap<(u64, &str), usize> {
        self.nets
            .iter()
            .enumerate()
            .flat_map(|(index, net)| net.terminals.iter().map(move |terminal| ((terminal.component, terminal.terminal.as_str()), index)))
            .collect()
    }

    /// Resolve a display name by the traversal's accepted-net index. Its fallback
    /// ordinal is local to this baseline, never persistent electrical identity.
    pub(crate) fn net_name(&self, index: usize) -> Option<String> {
        self.nets.get(index).map(|net| net.names.first().cloned().unwrap_or_else(|| format!("Net {}", index + 1)))
    }
}

impl crate::Session {
    /// Adopt one complete supported source snapshot as an engine-only opt-in
    /// baseline. Subsequent deletions retain physical state; repeated calls are
    /// no-ops and never silently accept later schematic connectivity changes.
    pub(crate) fn initialize_update_baseline(&mut self, params: &serde_json::Value) -> Result<(), crate::CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        if project.board.baseline.is_some() {
            return Ok(());
        }
        let source = capture_source(project)?;
        let mut choices = Baseline::current_choices(project);
        for footprint in project.board.footprints().iter().filter(|footprint| !project.component_is_board_eligible(footprint.component)) {
            // Adoption describes actual physical state, not an excluded component's
            // independently retained desired package choice.
            choices.insert(footprint.component, Package::from_footprint(footprint));
        }
        let baseline = Baseline::capture(project, &choices, source, true).map_err(crate::CommandError::InvalidParams)?;
        if baseline.components.values().any(|component| component.package.is_none()) {
            return Err(crate::CommandError::InvalidParams(
                "Assign a supported footprint to every board-eligible component before initializing reviewed board updates.".into(),
            ));
        }
        let mut next = project.clone();
        Arc::make_mut(&mut next.board).baseline = Some(Arc::new(baseline));
        crate::persistence::encode_documents(Arc::new(next.clone())).map_err(crate::CommandError::InvalidParams)?;
        self.change(next);
        Ok(())
    }
}

/// Hash the complete canonical pre-Apply native bundle, including prior accepted
/// state. A fresh incarnation is attached only after hashing, so repeated previews
/// of equal input have equal digests without recursively including their new ID.
pub(crate) fn capture_source(project: &Project) -> Result<SourceCapture, crate::CommandError> {
    use sha2::{Digest, Sha256};
    let documents = crate::persistence::encode_documents(Arc::new(project.clone())).map_err(crate::CommandError::InvalidParams)?;
    let mut digest = Sha256::new();
    digest.update(b"pcbcraft-board-source-bundle/1\0");
    for (path, bytes) in &documents {
        digest.update((path.len() as u64).to_be_bytes());
        digest.update(path.as_bytes());
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    }
    Ok(SourceCapture { capture_id: crate::fresh_id()?, content_digest: format!("{:x}", digest.finalize()) })
}

/// A component-level change carries stable identity rather than reference text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComponentChange {
    Added { component: u64 },
    Removed { component: u64 },
    Definition { component: u64 },
    Package { component: u64 },
    Mapping { component: u64 },
}

/// A reviewed update cannot accept incomplete physical package intent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Blocker {
    MissingPackage { component: u64 },
}

/// Deterministic differences between accepted and proposed logical inputs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Preview {
    pub components: Vec<ComponentChange>,
    pub removed_nets: Vec<NetState>,
    pub added_nets: Vec<NetState>,
    pub blockers: Vec<Blocker>,
}

impl Preview {
    /// Report whether no physical/electrical change awaits acceptance. Live
    /// reference/value changes and a new capture UUID alone do not create updates.
    pub fn is_noop(&self) -> bool {
        self.components.is_empty() && self.removed_nets.is_empty() && self.added_nets.is_empty() && self.blockers.is_empty()
    }
}

/// Compare normalized accepted inputs without mutating either snapshot. Caller
/// commands must additionally enforce opening/revision authority before Apply.
pub fn preview(accepted: &Baseline, proposed: &Baseline) -> Result<Preview, String> {
    accepted.validate()?;
    proposed.validate()?;
    if accepted.project != proposed.project || accepted.board != proposed.board {
        return Err("board update inputs have different owners".into());
    }
    let mut changes = Vec::new();
    let mut blockers = Vec::new();
    for (&id, component) in &proposed.components {
        if component.package.is_none() {
            blockers.push(Blocker::MissingPackage { component: id });
        }
        match accepted.components.get(&id) {
            None => changes.push(ComponentChange::Added { component: id }),
            Some(previous) => {
                if (previous.library.as_str(), previous.definition.as_str(), previous.revision, &previous.terminals)
                    != (component.library.as_str(), component.definition.as_str(), component.revision, &component.terminals)
                {
                    changes.push(ComponentChange::Definition { component: id });
                }
                let identity = |package: &Option<Package>| package.as_ref().map(|p| (p.library.clone(), p.definition.clone(), p.revision));
                if identity(&previous.package) != identity(&component.package) {
                    changes.push(ComponentChange::Package { component: id });
                } else if previous.package.as_ref().map(|p| &p.mapping) != component.package.as_ref().map(|p| &p.mapping) {
                    changes.push(ComponentChange::Mapping { component: id });
                }
            }
        }
    }
    for id in accepted.components.keys().filter(|id| !proposed.components.contains_key(id)) {
        changes.push(ComponentChange::Removed { component: *id });
    }
    let before: BTreeSet<_> = accepted.nets.iter().collect();
    let after: BTreeSet<_> = proposed.nets.iter().collect();
    let removed_nets = accepted.nets.iter().filter(|net| !after.contains(net)).cloned().collect();
    let added_nets = proposed.nets.iter().filter(|net| !before.contains(net)).cloned().collect();
    Ok(Preview { components: changes, removed_nets, added_nets, blockers })
}

/// Build a complete physical candidate with its matching accepted baseline installed,
/// without changing the input project. Revision checks and single-step publication
/// belong to the caller; unsupported/missing assets produce no partial mutation.
pub fn apply_candidate(project: &Project, accepted: &Baseline, proposed: &Baseline) -> Result<(Project, Baseline), String> {
    let target = project.boards().next().map(|board| board.target).ok_or("no physical board is available")?;
    if target.project != proposed.project || target.board != proposed.board {
        return Err("board update target is not the current board".into());
    }
    let plan = preview(accepted, proposed)?;
    if !plan.blockers.is_empty() {
        return Err("board update has unresolved package assignments".into());
    }
    proposed.validate_assets(project)?;
    let choices = proposed.components.iter().filter_map(|(&id, component)| component.package.clone().map(|package| (id, package))).collect();
    if Baseline::proposed(project, &choices, proposed.source.clone())? != *proposed {
        return Err("board update no longer matches the current schematic".into());
    }
    if project.board.footprints().iter().any(|footprint| {
        accepted.components.get(&footprint.component).and_then(|component| component.package.as_ref()) != Some(&Package::from_footprint(footprint))
    }) {
        return Err("existing footprint differs from its accepted source package".into());
    }
    if plan.is_noop() {
        return Ok((project.clone(), accepted.clone()));
    }
    let mut next = project.clone();
    let board = Arc::make_mut(&mut next.board);
    board.footprints.retain(|footprint| proposed.components.contains_key(&footprint.component));
    board.by_component = board.footprints.iter().enumerate().map(|(index, footprint)| (footprint.component, index)).collect();
    for footprint in &mut board.footprints {
        let package =
            proposed.components.get(&footprint.component).and_then(|component| component.package.as_ref()).ok_or("missing accepted package")?;
        footprint.library = package.library.clone();
        footprint.definition = package.definition.clone();
        footprint.definition_revision = package.revision;
        footprint.mapping = package.library.as_ref().map(|_| package.mapping.clone());
        footprint.unconnected_pads = package.unconnected_pads.clone();
        footprint.pin_1_pad = if package.library.is_some() {
            0
        } else {
            match package.mapping.get("1").and_then(|pads| pads.first()) {
                Some("1") => 1,
                Some("2") => 2,
                _ => return Err("invalid legacy marker mapping".into()),
            }
        };
        if !footprint.valid() {
            return Err("replacement footprint is outside supported bounds".into());
        }
    }
    // Excluded components keep their authored choices for a later unexclude.
    // They do not become accepted physical members or force a pending update.
    board.assignments.retain(|id, _| project.components.contains_key(id) && !project.component_is_board_eligible(*id));
    board.assignments.extend(
        proposed
            .components
            .iter()
            .filter_map(|(&id, component)| component.package.as_ref().and_then(|package| package.assignment(id)).map(|assignment| (id, assignment))),
    );
    let used: BTreeSet<_> =
        board.assignments.values().map(super::assignment::Assignment::key).chain(board.footprints.iter().filter_map(Footprint::key)).collect();
    board.definitions.retain(|key, _| used.contains(key));
    board.baseline = Some(Arc::new(proposed.clone()));
    Ok((next, proposed.clone()))
}

/// Check canonical identity order while rejecting duplicates without allocating a set.
fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

/// Bound accepted terminal mappings while decoding and reject duplicate JSON keys
/// before a later entry can silently replace the historical pin correspondence.
fn mapping_records<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<String, crate::library::parts::PadIds>, D::Error> {
    struct Mapping;
    impl<'de> Visitor<'de> for Mapping {
        type Value = BTreeMap<String, crate::library::parts::PadIds>;

        /// Describe the exact mapping vocabulary and its supported terminal count.
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("at most 128 unique terminal-to-pad mappings")
        }

        /// Check bounds and uniqueness while consuming entries, including duplicate
        /// keys whose final overwritten value would otherwise form a valid bijection.
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut mapping = BTreeMap::new();
            while let Some((terminal, pad)) = map.next_entry::<String, crate::library::parts::PadIds>()? {
                if mapping.len() >= 128 || PinNumber::new(&terminal).is_none() || !pad.valid() || mapping.insert(terminal, pad).is_some() {
                    return Err(serde::de::Error::custom("duplicate, excessive or invalid accepted terminal mapping"));
                }
            }
            Ok(mapping)
        }
    }
    deserializer.deserialize_map(Mapping)
}

/// Reject duplicate and noncanonical numeric map keys rather than letting JSON
/// silently overwrite one historical component with another spelling of its ID.
fn component_records<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<u64, ComponentState>, D::Error> {
    struct Records;
    impl<'de> Visitor<'de> for Records {
        type Value = BTreeMap<u64, ComponentState>;

        /// Describe the bounded canonical object expected from the native codec.
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a bounded map of canonical positive component IDs")
        }

        /// Enforce uniqueness while parsing, before a duplicate entry can overwrite history.
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut records = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, ComponentState>()? {
                let id = key.parse::<u64>().map_err(serde::de::Error::custom)?;
                if key != id.to_string() || records.len() >= MAX_SYMBOLS || records.insert(id, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate, excessive or noncanonical baseline component ID"));
                }
            }
            Ok(records)
        }
    }
    deserializer.deserialize_map(Records)
}

#[cfg(test)]
mod retention_tests;
#[cfg(test)]
mod tests;
