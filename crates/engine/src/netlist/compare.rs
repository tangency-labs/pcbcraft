//! Read-back comparison of netlist documents. Terminals are matched by their logical address
//! (component ID and terminal number), never by net name or list position, because both can
//! change without any change in connectivity. Nets are paired by persistent ID first, then by
//! an identical terminal set, then by the largest number of shared terminals. Without
//! persistent IDs, ambiguous edits need not identify which net or terminal originally moved.

use super::{Basis, Component, Net, Netlist, TerminalInfo};
use crate::{CommandError, Session};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, VecDeque},
};

/// Compare a netlist document with what the open project exports now; edits nothing.
pub const COMPARE_NETLIST: &str = "project.compare_netlist";
/// The largest netlist text the command reads, checked before parsing.
pub const MAX_TEXT_BYTES: usize = 8 << 20;

/// The result of comparing an expected netlist with one that was found, such as a file read
/// back against the design it was exported from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Comparison {
    /// No entry in `differences`: the same project, basis and board, the same components with
    /// the same terminals, every terminal on the paired net (or unconnected, or listed nowhere)
    /// in both, and in `accepted` netlists the same pads for every terminal. Without persistent
    /// net IDs the pairing is a best match by shared terminals, not a proof of identity.
    pub matches: bool,
    /// The two documents are equal in every field, so they serialize to the same bytes.
    pub identical: bool,
    pub expected: Counts,
    pub found: Counts,
    /// Connectivity differences; any entry means the documents do not match. In order: project,
    /// basis and board; then, per expected component, a missing component, or its missing and
    /// re-padded terminals in its terminal order, then its extra terminals, then its moved ones;
    /// then components only `found` has; then expected nets with no counterpart, in expected
    /// order; then nets only `found` has.
    pub differences: Vec<Difference>,
    /// Differences that leave connectivity unchanged, such as a renamed net or a new value.
    pub notes: Vec<Difference>,
}

/// What one document lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub components: usize,
    /// Terminals in the components' terminal tables.
    pub terminals: usize,
    pub nets: usize,
    /// Terminal-to-net pairs: the terminals listed across all nets.
    pub connections: usize,
    pub unconnected: usize,
}

impl Counts {
    /// Count what `netlist` lists; terminals listed twice are counted twice.
    fn of(netlist: &Netlist) -> Self {
        Self {
            components: netlist.components.len(),
            terminals: netlist.components.iter().map(|component| component.terminals.len()).sum(),
            nets: netlist.nets.len(),
            connections: netlist.nets.iter().map(|net| net.terminals.len()).sum(),
            unconnected: netlist.unconnected.len(),
        }
    }
}

/// What kind of difference an entry describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The documents describe different projects.
    Project,
    /// One describes the schematic and the other a board's accepted state.
    Basis,
    /// The accepted netlists describe different boards.
    Board,
    PartMissing,
    PartExtra,
    TerminalMissing,
    TerminalExtra,
    /// A terminal changed place: from one net to another, unconnected, or unlisted.
    TerminalMoved,
    /// A terminal maps to different footprint pads.
    PadsChanged,
    /// An expected net has no counterpart in the found document.
    NetMissing,
    /// A found net has no counterpart in the expected document.
    NetExtra,
    /// Notes only: a paired net has another name or authored names.
    NetRenamed,
    /// Notes only: a paired net has another persistent ID.
    NetIdChanged,
    /// Notes only: another project name, board name or board source.
    Header,
    /// Notes only: another reference, value, kind, definition, unit placement or package.
    PartChanged,
    /// Notes only: another terminal name or electrical type.
    TerminalChanged,
}

/// One difference between the documents. `expected` and `found` hold the two values, absent
/// where one side has none. A terminal's place is `"unconnected"`, `"unlisted"` (on no net and
/// not unconnected, as for a unit that is not placed) or a net: `{"net": {"id", "name"}}` on the
/// expected side and `{"net": {"id", "index"}}` on the found side, `index` being the net's
/// position in the found document's `nets`. Moved-terminal entries do not copy the file net's
/// display name. Persistent IDs and design-side references and names can repeat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Difference {
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
    /// The changed field, for header, component and terminal notes and for renamed nets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<&'static str>,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub expected: Value,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub found: Value,
    pub message: String,
}

impl Difference {
    fn new(kind: Kind, message: String) -> Self {
        Self { kind, part: None, terminal: None, field: None, expected: Value::Null, found: Value::Null, message }
    }

    fn values(mut self, expected: Value, found: Value) -> Self {
        self.expected = expected;
        self.found = found;
        self
    }

    fn at(mut self, part: &str, terminal: Option<&str>) -> Self {
        self.part = Some(part.to_owned());
        self.terminal = terminal.map(str::to_owned);
        self
    }

    fn field(mut self, field: &'static str) -> Self {
        self.field = Some(field);
        self
    }
}

/// Where a terminal is in one document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Net(usize),
    Unconnected,
    Unlisted,
}

type Key<'a> = (&'a str, &'a str);

/// Each listed terminal's place; a terminal listed twice keeps its first place, although
/// [`Netlist::parse`] refuses such documents.
fn places(netlist: &Netlist) -> BTreeMap<Key<'_>, Place> {
    let mut places = BTreeMap::new();
    for (index, net) in netlist.nets.iter().enumerate() {
        for terminal in &net.terminals {
            places.entry((terminal.part.as_str(), terminal.terminal.as_str())).or_insert(Place::Net(index));
        }
    }
    for terminal in &netlist.unconnected {
        places.entry((terminal.part.as_str(), terminal.terminal.as_str())).or_insert(Place::Unconnected);
    }
    places
}

/// The `{part, terminal}` addresses a net lists, as a set.
fn members(net: &Net) -> BTreeSet<Key<'_>> {
    net.terminals.iter().map(|terminal| (terminal.part.as_str(), terminal.terminal.as_str())).collect()
}

/// Pair each expected net with at most one found net: by equal persistent ID, then by an
/// identical terminal set, then greedily by the most shared terminals. Among equal overlaps the
/// pair with the fewest unshared terminals wins, then the earlier expected and the earlier found
/// net. This is deterministic tie-breaking, not a recovery of missing net identity.
/// Returns the found index for each expected net.
fn pair_nets(expected: &Netlist, found: &Netlist, found_places: &BTreeMap<Key<'_>, Place>) -> Vec<Option<usize>> {
    let mut pairs = vec![None; expected.nets.len()];
    let mut taken = vec![false; found.nets.len()];
    let mut by_id = BTreeMap::new();
    for (index, net) in found.nets.iter().enumerate() {
        if let Some(id) = net.id {
            by_id.entry(id).or_insert(index);
        }
    }
    for (pair, net) in pairs.iter_mut().zip(&expected.nets) {
        if let Some(&index) = net.id.and_then(|id| by_id.get(&id))
            && !taken[index]
        {
            *pair = Some(index);
            taken[index] = true;
        }
    }
    let mut by_members = BTreeMap::<_, VecDeque<usize>>::new();
    for (index, net) in found.nets.iter().enumerate().filter(|(index, _)| !taken[*index]) {
        by_members.entry(members(net)).or_default().push_back(index);
    }
    for (pair, net) in pairs.iter_mut().zip(&expected.nets).filter(|(pair, _)| pair.is_none()) {
        // Only this loop takes from these lists, so the front candidate is still free.
        if let Some(index) = by_members.get_mut(&members(net)).and_then(VecDeque::pop_front) {
            *pair = Some(index);
            taken[index] = true;
        }
    }
    let found_sizes: Vec<usize> = found.nets.iter().map(|net| members(net).len()).collect();
    let mut overlaps = Vec::new();
    for (expected_index, net) in expected.nets.iter().enumerate().filter(|(index, _)| pairs[*index].is_none()) {
        let own = members(net);
        let size = own.len();
        let mut shared = BTreeMap::<usize, usize>::new();
        for key in own {
            if let Some(&Place::Net(index)) = found_places.get(&key)
                && !taken[index]
            {
                *shared.entry(index).or_default() += 1;
            }
        }
        overlaps.extend(shared.into_iter().map(|(found_index, count)| {
            let union = size + found_sizes.get(found_index).copied().unwrap_or(0) - count;
            (Reverse(count), union, expected_index, found_index)
        }));
    }
    overlaps.sort_unstable();
    for (_, _, expected_index, found_index) in overlaps {
        if pairs[expected_index].is_none() && !taken[found_index] {
            pairs[expected_index] = Some(found_index);
            taken[found_index] = true;
        }
    }
    pairs
}

/// A component as messages name it: its reference with its ID.
fn label(component: &Component) -> String {
    format!("{} ({})", component.reference, component.id)
}

/// A net as a difference names it: its persistent ID (or null) and its name.
fn net_value(net: &Net) -> Value {
    json!({"net": {"id": net.id, "name": net.name}})
}

/// A terminal's place as a JSON value: the net it is on, `"unconnected"` or `"unlisted"`.
/// A net index outside `netlist` gives null.
fn place_value(netlist: &Netlist, place: Place) -> Value {
    match place {
        Place::Net(index) => netlist.nets.get(index).map_or(Value::Null, net_value),
        Place::Unconnected => json!("unconnected"),
        Place::Unlisted => json!("unlisted"),
    }
}

/// A terminal's place for a message, such as "on net GND"; a net index outside `netlist` gives
/// an empty string.
fn place_text(netlist: &Netlist, place: Place) -> String {
    match place {
        Place::Net(index) => netlist.nets.get(index).map_or_else(String::new, |net| format!("on net {}", net.name)),
        Place::Unconnected => "unconnected".into(),
        Place::Unlisted => "on no net".into(),
    }
}

/// A terminal's place in the found document as a JSON value. A net is named by its persistent
/// ID and its index in `found.nets`, never by its name, so a long name in the file is not copied
/// once per terminal. Design-side names can still repeat in moved-terminal messages.
fn found_value(found: &Netlist, place: Place) -> Value {
    match place {
        Place::Net(index) => found.nets.get(index).map_or(Value::Null, |net| json!({"net": {"id": net.id, "index": index}})),
        Place::Unconnected | Place::Unlisted => place_value(found, place),
    }
}

/// A found terminal's place for a message: a net is named by its index in the file and the
/// design net it pairs with, if any (`back` maps each found net to its paired expected net).
fn found_text(expected: &Netlist, back: &[Option<usize>], place: Place) -> String {
    match place {
        Place::Net(index) => match back.get(index).copied().flatten().and_then(|pair| expected.nets.get(pair)) {
            Some(net) => format!("on file net {index}, paired with net {}", net.name),
            None => format!("on file net {index}, which pairs with no expected net"),
        },
        Place::Unconnected | Place::Unlisted => place_text(expected, place),
    }
}

/// Compare `found` with `expected`. Edits nothing and never fails. Connectivity differences go
/// to `differences` in the order [`Comparison::differences`] gives; changed annotations and net
/// metadata go to `notes`. Order-only changes can leave both lists empty while `identical` is
/// false. The pure function does not validate its inputs; use [`Netlist::parse`] for file text.
pub fn compare(expected: &Netlist, found: &Netlist) -> Comparison {
    let mut differences = Vec::new();
    let mut notes = Vec::new();
    if expected.project.id != found.project.id {
        differences.push(
            Difference::new(Kind::Project, format!("Expected project {}, found project {}.", expected.project.id, found.project.id))
                .values(json!(expected.project.id), json!(found.project.id)),
        );
    } else if expected.project.name != found.project.name {
        notes.push(
            Difference::new(Kind::Header, format!("Expected project name {}, found {}.", expected.project.name, found.project.name))
                .field("project.name")
                .values(json!(expected.project.name), json!(found.project.name)),
        );
    }
    if expected.basis != found.basis {
        differences.push(
            Difference::new(Kind::Basis, format!("Expected a {} netlist, found a {} netlist.", basis(expected.basis), basis(found.basis)))
                .values(json!(expected.basis), json!(found.basis)),
        );
    }
    compare_boards(expected, found, &mut differences, &mut notes);

    let found_parts: BTreeMap<_, _> = found.components.iter().rev().map(|component| (component.id.as_str(), component)).collect();
    let expected_ids: BTreeSet<_> = expected.components.iter().map(|component| component.id.as_str()).collect();
    let expected_places = places(expected);
    let found_places = places(found);
    let pairs = pair_nets(expected, found, &found_places);
    let mut back = vec![None; found.nets.len()];
    for (expected_index, pair) in pairs.iter().enumerate() {
        if let Some(slot) = pair.and_then(|index| back.get_mut(index)) {
            *slot = Some(expected_index);
        }
    }
    for component in &expected.components {
        let Some(other) = found_parts.get(component.id.as_str()) else {
            differences.push(
                Difference::new(Kind::PartMissing, format!("{} is missing.", label(component)))
                    .at(&component.id, None)
                    .values(json!(component.id), Value::Null),
            );
            continue;
        };
        compare_part(component, other, &mut differences, &mut notes);
        let numbers: BTreeSet<&str> = other.terminals.iter().map(|terminal| terminal.terminal.as_str()).collect();
        for terminal in component.terminals.iter().filter(|terminal| numbers.contains(terminal.terminal.as_str())) {
            let key = (component.id.as_str(), terminal.terminal.as_str());
            let was = expected_places.get(&key).copied().unwrap_or(Place::Unlisted);
            let is = found_places.get(&key).copied().unwrap_or(Place::Unlisted);
            let same = match was {
                Place::Net(index) => pairs.get(index).copied().flatten().map(Place::Net) == Some(is),
                Place::Unconnected | Place::Unlisted => was == is,
            };
            if !same {
                differences.push(
                    Difference::new(
                        Kind::TerminalMoved,
                        format!(
                            "{} terminal {}: expected {}, found {}.",
                            label(component),
                            terminal.terminal,
                            place_text(expected, was),
                            found_text(expected, &back, is)
                        ),
                    )
                    .at(&component.id, Some(&terminal.terminal))
                    .values(place_value(expected, was), found_value(found, is)),
                );
            }
        }
    }
    for component in found.components.iter().filter(|component| !expected_ids.contains(component.id.as_str())) {
        differences.push(
            Difference::new(Kind::PartExtra, format!("{} is not expected.", label(component)))
                .at(&component.id, None)
                .values(Value::Null, json!(component.id)),
        );
    }

    let mut paired = vec![false; found.nets.len()];
    for (net, pair) in expected.nets.iter().zip(&pairs) {
        let Some(other) = pair.and_then(|index| found.nets.get(index)) else {
            differences.push(Difference::new(Kind::NetMissing, format!("Net {} has no counterpart.", net.name)).values(net_value(net), Value::Null));
            continue;
        };
        if let Some(seen) = pair.and_then(|index| paired.get_mut(index)) {
            *seen = true;
        }
        if net.id != other.id {
            notes.push(
                Difference::new(Kind::NetIdChanged, format!("Net {} has another persistent ID.", net.name)).values(json!(net.id), json!(other.id)),
            );
        }
        if net.name != other.name {
            notes.push(
                Difference::new(Kind::NetRenamed, format!("Net {} has another name in the file.", net.name))
                    .field("name")
                    .values(json!(net.name), json!(other.name)),
            );
        }
        if net.names != other.names {
            notes.push(
                Difference::new(Kind::NetRenamed, format!("Net {} has other authored names.", net.name))
                    .field("names")
                    .values(json!(net.names), json!(other.names)),
            );
        }
    }
    for (index, (net, _)) in found.nets.iter().zip(&paired).enumerate().filter(|(_, (_, seen))| !**seen) {
        differences.push(Difference::new(Kind::NetExtra, format!("File net {index} is not expected.")).values(Value::Null, net_value(net)));
    }
    Comparison {
        matches: differences.is_empty(),
        identical: expected == found,
        expected: Counts::of(expected),
        found: Counts::of(found),
        differences,
        notes,
    }
}

/// The basis as messages spell it, the same word the format uses.
fn basis(basis: Basis) -> &'static str {
    match basis {
        Basis::Schematic => "schematic",
        Basis::Accepted => "accepted",
    }
}

/// A different board, or a board on only one side, is a difference; another name or accepted
/// source of the same board is a note.
fn compare_boards(expected: &Netlist, found: &Netlist, differences: &mut Vec<Difference>, notes: &mut Vec<Difference>) {
    match (&expected.board, &found.board) {
        (Some(a), Some(b)) if a.id == b.id => {
            if a.name != b.name {
                notes.push(
                    Difference::new(Kind::Header, format!("Expected board name {}, found {}.", a.name, b.name))
                        .field("board.name")
                        .values(json!(a.name), json!(b.name)),
                );
            }
            if a.source != b.source {
                notes.push(
                    Difference::new(Kind::Header, format!("Board {} has another accepted source.", a.name))
                        .field("board.source")
                        .values(json!(a.source), json!(b.source)),
                );
            }
        }
        (None, None) => {}
        (a, b) => {
            let id = |board: &Option<super::BoardRef>| board.as_ref().map_or(Value::Null, |board| json!(board.id));
            differences.push(Difference::new(Kind::Board, "The netlists describe different boards.".into()).values(id(a), id(b)));
        }
    }
}

/// Report terminals only one side lists and different pads as differences, and changed
/// annotation, definition, units, package, terminal names and electrical types as notes.
fn compare_part(component: &Component, other: &Component, differences: &mut Vec<Difference>, notes: &mut Vec<Difference>) {
    let name = label(component);
    let fields = [
        ("reference", json!(component.reference), json!(other.reference)),
        ("value", json!(component.value), json!(other.value)),
        ("kind", json!(component.kind), json!(other.kind)),
        ("definition", json!(component.definition), json!(other.definition)),
        ("units", json!(component.units), json!(other.units)),
        ("package", json!(component.package), json!(other.package)),
    ];
    for (field, a, b) in fields.into_iter().filter(|(_, a, b)| a != b) {
        notes.push(Difference::new(Kind::PartChanged, format!("{name} has another {field}.")).at(&component.id, None).field(field).values(a, b));
    }
    let found_terminals: BTreeMap<&str, &TerminalInfo> =
        other.terminals.iter().rev().map(|terminal| (terminal.terminal.as_str(), terminal)).collect();
    let own: BTreeSet<&str> = component.terminals.iter().map(|terminal| terminal.terminal.as_str()).collect();
    for terminal in &component.terminals {
        let Some(found) = found_terminals.get(terminal.terminal.as_str()) else {
            differences.push(
                Difference::new(Kind::TerminalMissing, format!("{name} terminal {} is missing.", terminal.terminal))
                    .at(&component.id, Some(&terminal.terminal)),
            );
            continue;
        };
        let at = Some(terminal.terminal.as_str());
        if terminal.pads != found.pads {
            differences.push(
                Difference::new(Kind::PadsChanged, format!("{name} terminal {} maps to other pads.", terminal.terminal))
                    .at(&component.id, at)
                    .values(json!(terminal.pads), json!(found.pads)),
            );
        }
        for (field, a, b) in
            [("name", json!(terminal.name), json!(found.name)), ("electrical_type", json!(terminal.electrical_type), json!(found.electrical_type))]
        {
            if a != b {
                notes.push(
                    Difference::new(Kind::TerminalChanged, format!("{name} terminal {} has another {field}.", terminal.terminal))
                        .at(&component.id, at)
                        .field(field)
                        .values(a, b),
                );
            }
        }
    }
    for terminal in other.terminals.iter().filter(|terminal| !own.contains(terminal.terminal.as_str())) {
        differences.push(
            Difference::new(Kind::TerminalExtra, format!("{name} terminal {} is not expected.", terminal.terminal))
                .at(&component.id, Some(&terminal.terminal)),
        );
    }
}

impl Session {
    /// Compare the netlist text in `params["netlist"]` with what the open project exports now:
    /// the project netlist for a `schematic` document, or the accepted netlist of the board it
    /// names for an `accepted` one. The file is the found side and the design the expected
    /// side. Edits nothing. Fails when no project is open, the text is not a valid netlist,
    /// its board is not in the project, or the design's export fails.
    pub(crate) fn compare_netlist(&self, params: &Value) -> Result<Comparison, CommandError> {
        let project = self.open_project()?;
        let text = params["netlist"].as_str().ok_or_else(|| CommandError::InvalidParams("`netlist` must be netlist text".into()))?;
        let found = Netlist::parse(text).map_err(CommandError::InvalidParams)?;
        let design = match (found.basis, &found.board) {
            (Basis::Schematic, _) => project.netlist(),
            (Basis::Accepted, Some(board)) => {
                let unknown = || CommandError::InvalidParams(format!("the netlist's board {} is not in the open project", board.id));
                let target = project.resolve_board(Some(&board.id.to_string())).map_err(|_| unknown())?;
                project.board_view(target).ok_or_else(unknown)?.netlist()
            }
            (Basis::Accepted, None) => return Err(CommandError::InvalidParams("an accepted netlist names its board".into())),
        };
        let design = design.map_err(CommandError::Unavailable)?;
        Ok(compare(&design.netlist, &found))
    }
}

#[cfg(test)]
mod tests;
