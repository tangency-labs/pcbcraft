//! Read-back comparison of netlist documents. Terminals are matched by their logical address
//! (component ID and terminal number), never by net name or list position, because both can
//! change without any change in connectivity. Nets are paired by persistent ID first, then by
//! an identical terminal set, then by the largest number of shared terminals. Without
//! persistent IDs, ambiguous edits need not identify which net or terminal originally moved.

use super::{Basis, Component, ComponentKind, DefinitionRef, Net, Netlist, PackageRef, TerminalInfo, Unit};
use crate::{CommandError, Session};
use serde::Serialize;
use serde_json::Value;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
};
use uuid::Uuid;

/// Compare a netlist document with what the open project exports now; edits nothing.
pub const COMPARE_NETLIST: &str = "project.compare_netlist";
/// The largest netlist text the command reads, checked before parsing.
pub const MAX_TEXT_BYTES: usize = 8 << 20;
const MAX_REPORT_BYTES: usize = 16 << 20;
const MAX_REPORT_ROWS: usize = 16_384;
// The fixed header includes the two count tuples, booleans and field/list delimiters.
const REPORT_HEADER_BYTES: usize = 512;

/// The result of comparing an expected netlist with one that was found, such as a file read
/// back against the design it was exported from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Comparison<'a> {
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
    pub differences: Vec<Difference<'a>>,
    /// Differences that leave connectivity unchanged, such as a renamed net or a new value.
    pub notes: Vec<Difference<'a>>,
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

/// One difference between the documents. Net-level rows identify each applicable net by its
/// index in that side's export. Moved terminals use `{net: {id, index}}` on either side, or
/// `unconnected`/`unlisted`. Both forms are unambiguous even for equal names and null IDs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Difference<'a> {
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_net: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found_net: Option<usize>,
    /// The changed field, for header, component and terminal notes and for renamed nets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<&'static str>,
    #[serde(skip_serializing_if = "Datum::absent")]
    expected: Datum<'a>,
    #[serde(skip_serializing_if = "Datum::absent")]
    found: Datum<'a>,
    pub message: &'static str,
}

/// Borrow rich payloads until the row passes the budget; do not clone a long name or array.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum Datum<'a> {
    Absent,
    Text(&'a str),
    Id(Uuid),
    Basis(Basis),
    Source(&'a crate::board::update::SourceCapture),
    Kind(ComponentKind),
    Definition(&'a DefinitionRef),
    Units(&'a [Unit]),
    Package(&'a PackageRef),
    Strings(&'a [String]),
    Electrical(super::ElectricalType),
    Net { net: NetRef },
}

impl Datum<'_> {
    /// Whether the value should be omitted in the report.
    fn absent(&self) -> bool {
        matches!(self, Self::Absent)
    }
}

/// The position in the named export, plus its optional persistent identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct NetRef {
    id: Option<Uuid>,
    index: usize,
}

impl<'a> Difference<'a> {
    /// Start a row with no optional addresses or values.
    fn new(kind: Kind, message: &'static str) -> Self {
        Self {
            kind,
            part: None,
            terminal: None,
            expected_net: None,
            found_net: None,
            field: None,
            expected: Datum::Absent,
            found: Datum::Absent,
            message,
        }
    }

    /// Attach the expected and found values without copying them.
    fn values(mut self, expected: Datum<'a>, found: Datum<'a>) -> Self {
        self.expected = expected;
        self.found = found;
        self
    }

    /// Address a component or one of its logical terminals.
    fn at(mut self, part: &'a str, terminal: Option<&'a str>) -> Self {
        self.part = Some(part);
        self.terminal = terminal;
        self
    }

    /// Name the applicable net indices in each export.
    fn nets(mut self, expected: Option<usize>, found: Option<usize>) -> Self {
        self.expected_net = expected;
        self.found_net = found;
        self
    }

    /// Name the field changed by an annotation-only edit.
    fn field(mut self, field: &'static str) -> Self {
        self.field = Some(field);
        self
    }
}

/// Charge escaped JSON bytes without buffering them. A rejected row is never stored.
struct ByteBudget(usize);

impl io::Write for ByteBudget {
    /// Count a serialized fragment without storing it.
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.checked_sub(bytes.len()).ok_or_else(|| io::Error::other("netlist comparison budget exceeded"))?;
        Ok(bytes.len())
    }

    /// No buffered data to flush.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Report<'a> {
    bytes: ByteBudget,
    rows_left: usize,
    differences: Vec<Difference<'a>>,
    notes: Vec<Difference<'a>>,
}

impl<'a> Report<'a> {
    /// Append a row only if it fits the shared note/difference byte and row budgets.
    fn push(&mut self, row: Difference<'a>, is_note: bool) -> Result<(), CommandError> {
        let too_large = || CommandError::Unavailable("Netlist comparison report exceeds its size limit (16 MiB or 16,384 entries).".into());
        self.rows_left = self.rows_left.checked_sub(1).ok_or_else(too_large)?;
        self.bytes.0 = self.bytes.0.checked_sub(1).ok_or_else(too_large)?; // Allow for a list separator.
        serde_json::to_writer(&mut self.bytes, &row).map_err(|_| too_large())?;
        if is_note {
            self.notes.push(row);
        } else {
            self.differences.push(row);
        }
        Ok(())
    }

    /// Add a connectivity difference to the bounded report.
    fn difference(&mut self, row: Difference<'a>) -> Result<(), CommandError> {
        self.push(row, false)
    }

    /// Add an annotation-only change to the bounded report.
    fn note(&mut self, row: Difference<'a>) -> Result<(), CommandError> {
        self.push(row, true)
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

/// Identify a terminal's place without copying a potentially long net name.
fn place_value(netlist: &Netlist, place: Place) -> Datum<'static> {
    match place {
        Place::Net(index) => netlist.nets.get(index).map_or(Datum::Absent, |net| Datum::Net { net: NetRef { id: net.id, index } }),
        Place::Unconnected => Datum::Text("unconnected"),
        Place::Unlisted => Datum::Text("unlisted"),
    }
}

/// Compare two decoded documents without editing either. Refuse reports exceeding the byte
/// or row limit. The function compares data as supplied; use [`Netlist::parse`] for file text.
pub fn compare<'a>(expected: &'a Netlist, found: &'a Netlist) -> Result<Comparison<'a>, CommandError> {
    compare_with_budget(expected, found, MAX_REPORT_BYTES - REPORT_HEADER_BYTES, MAX_REPORT_ROWS)
}

/// Assemble a report with a shared escaped-row byte and row count budget, excluding the header.
fn compare_with_budget<'a>(expected: &'a Netlist, found: &'a Netlist, bytes: usize, rows: usize) -> Result<Comparison<'a>, CommandError> {
    let mut report = Report { bytes: ByteBudget(bytes), rows_left: rows, differences: Vec::new(), notes: Vec::new() };
    if expected.project.id != found.project.id {
        report
            .difference(Difference::new(Kind::Project, "Project IDs differ.").values(Datum::Id(expected.project.id), Datum::Id(found.project.id)))?;
    } else if expected.project.name != found.project.name {
        report.note(
            Difference::new(Kind::Header, "Project names differ.")
                .field("project.name")
                .values(Datum::Text(&expected.project.name), Datum::Text(&found.project.name)),
        )?;
    }
    if expected.basis != found.basis {
        report.difference(Difference::new(Kind::Basis, "Netlist bases differ.").values(Datum::Basis(expected.basis), Datum::Basis(found.basis)))?;
    }
    compare_boards(expected, found, &mut report)?;

    let found_parts: BTreeMap<_, _> = found.components.iter().rev().map(|component| (component.id.as_str(), component)).collect();
    let expected_ids: BTreeSet<_> = expected.components.iter().map(|component| component.id.as_str()).collect();
    let expected_places = places(expected);
    let found_places = places(found);
    let pairs = pair_nets(expected, found, &found_places);
    let mut paired = vec![false; found.nets.len()];
    for component in &expected.components {
        let Some(other) = found_parts.get(component.id.as_str()) else {
            report.difference(Difference::new(Kind::PartMissing, "Part is missing from the file.").at(&component.id, None))?;
            continue;
        };
        compare_part(component, other, &mut report)?;
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
                report.difference(
                    Difference::new(Kind::TerminalMoved, "Terminal connection changed.")
                        .at(&component.id, Some(&terminal.terminal))
                        .values(place_value(expected, was), place_value(found, is)),
                )?;
            }
        }
    }
    for component in found.components.iter().filter(|component| !expected_ids.contains(component.id.as_str())) {
        report.difference(Difference::new(Kind::PartExtra, "File has an extra part.").at(&component.id, None))?;
    }

    for (index, (net, pair)) in expected.nets.iter().zip(&pairs).enumerate() {
        let Some((other_index, other)) = pair.and_then(|i| found.nets.get(i).map(|net| (i, net))) else {
            report.difference(Difference::new(Kind::NetMissing, "Design net has no counterpart.").nets(Some(index), None))?;
            continue;
        };
        paired[other_index] = true;
        if net.id != other.id {
            report.note(
                Difference::new(Kind::NetIdChanged, "Paired nets have different persistent IDs.")
                    .nets(Some(index), Some(other_index))
                    .values(net.id.map_or(Datum::Absent, Datum::Id), other.id.map_or(Datum::Absent, Datum::Id)),
            )?;
        }
        if net.name != other.name {
            report.note(
                Difference::new(Kind::NetRenamed, "Paired nets have different display names.")
                    .nets(Some(index), Some(other_index))
                    .field("name")
                    .values(Datum::Text(&net.name), Datum::Text(&other.name)),
            )?;
        }
        if net.names != other.names {
            report.note(
                Difference::new(Kind::NetRenamed, "Paired nets have different authored names.")
                    .nets(Some(index), Some(other_index))
                    .field("names")
                    .values(Datum::Strings(&net.names), Datum::Strings(&other.names)),
            )?;
        }
    }
    for (index, _) in paired.iter().enumerate().filter(|(_, matched)| !**matched) {
        report.difference(Difference::new(Kind::NetExtra, "File net has no counterpart.").nets(None, Some(index)))?;
    }
    Ok(Comparison {
        matches: report.differences.is_empty(),
        identical: expected == found,
        expected: Counts::of(expected),
        found: Counts::of(found),
        differences: report.differences,
        notes: report.notes,
    })
}

/// A different board, or a board on only one side, is a difference; another name or accepted
/// source of the same board is a note.
fn compare_boards<'a>(expected: &'a Netlist, found: &'a Netlist, report: &mut Report<'a>) -> Result<(), CommandError> {
    match (&expected.board, &found.board) {
        (Some(a), Some(b)) if a.id == b.id => {
            if a.name != b.name {
                report.note(
                    Difference::new(Kind::Header, "Board names differ.").field("board.name").values(Datum::Text(&a.name), Datum::Text(&b.name)),
                )?;
            }
            if a.source != b.source {
                report.note(
                    Difference::new(Kind::Header, "Accepted sources differ.")
                        .field("board.source")
                        .values(a.source.as_ref().map_or(Datum::Absent, Datum::Source), b.source.as_ref().map_or(Datum::Absent, Datum::Source)),
                )?;
            }
        }
        (None, None) => {}
        (a, b) => {
            let id = |board: &Option<super::BoardRef>| board.as_ref().map_or(Datum::Absent, |board| Datum::Id(board.id));
            report.difference(Difference::new(Kind::Board, "The netlists describe different boards.").values(id(a), id(b)))?;
        }
    }
    Ok(())
}

/// Physical connectivity is pad membership, not the ordering used for picking a representative.
fn same_pads(a: Option<&[String]>, b: Option<&[String]>) -> bool {
    a == b
        || match (a, b) {
            (Some(a), Some(b)) => a.iter().collect::<BTreeSet<_>>() == b.iter().collect::<BTreeSet<_>>(),
            _ => false,
        }
}

/// Report terminals only one side lists and different pads as differences, and changed
/// annotation, definition, units, package, terminal names and electrical types as notes.
fn compare_part<'a>(component: &'a Component, other: &'a Component, report: &mut Report<'a>) -> Result<(), CommandError> {
    let fields = [
        ("reference", Datum::Text(&component.reference), Datum::Text(&other.reference)),
        ("value", Datum::Text(&component.value), Datum::Text(&other.value)),
        ("kind", Datum::Kind(component.kind), Datum::Kind(other.kind)),
        ("definition", Datum::Definition(&component.definition), Datum::Definition(&other.definition)),
        ("units", Datum::Units(&component.units), Datum::Units(&other.units)),
        ("package", component.package.as_ref().map_or(Datum::Absent, Datum::Package), other.package.as_ref().map_or(Datum::Absent, Datum::Package)),
    ];
    for (field, a, b) in fields.into_iter().filter(|(_, a, b)| a != b) {
        report.note(Difference::new(Kind::PartChanged, "Part metadata changed.").at(&component.id, None).field(field).values(a, b))?;
    }
    let found_terminals: BTreeMap<&str, &TerminalInfo> =
        other.terminals.iter().rev().map(|terminal| (terminal.terminal.as_str(), terminal)).collect();
    let own: BTreeSet<&str> = component.terminals.iter().map(|terminal| terminal.terminal.as_str()).collect();
    for terminal in &component.terminals {
        let Some(found) = found_terminals.get(terminal.terminal.as_str()) else {
            report.difference(
                Difference::new(Kind::TerminalMissing, "Terminal is missing from the file.").at(&component.id, Some(&terminal.terminal)),
            )?;
            continue;
        };
        let at = Some(terminal.terminal.as_str());
        if !same_pads(terminal.pads.as_deref(), found.pads.as_deref()) {
            report.difference(Difference::new(Kind::PadsChanged, "Terminal maps to different pads.").at(&component.id, at).values(
                terminal.pads.as_deref().map_or(Datum::Absent, Datum::Strings),
                found.pads.as_deref().map_or(Datum::Absent, Datum::Strings),
            ))?;
        }
        for (field, a, b) in [
            ("name", terminal.name.as_deref().map_or(Datum::Absent, Datum::Text), found.name.as_deref().map_or(Datum::Absent, Datum::Text)),
            (
                "electrical_type",
                terminal.electrical_type.map_or(Datum::Absent, Datum::Electrical),
                found.electrical_type.map_or(Datum::Absent, Datum::Electrical),
            ),
        ] {
            if a != b {
                report.note(Difference::new(Kind::TerminalChanged, "Terminal metadata changed.").at(&component.id, at).field(field).values(a, b))?;
            }
        }
    }
    for terminal in other.terminals.iter().filter(|terminal| !own.contains(terminal.terminal.as_str())) {
        report.difference(Difference::new(Kind::TerminalExtra, "File has an extra terminal.").at(&component.id, Some(&terminal.terminal)))?;
    }
    Ok(())
}

impl Session {
    /// Compare the netlist text in `params["netlist"]` with what the open project exports now:
    /// the project netlist for a `schematic` document, or the accepted netlist of the board it
    /// names for an `accepted` one. The file is the found side and the design the expected
    /// side. Edits nothing. Fails when no project is open, the text is not a valid netlist,
    /// its board is not in the project, the export fails, or the report exceeds its budget.
    pub(crate) fn compare_netlist(&self, params: &Value) -> Result<Value, CommandError> {
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
        serde_json::to_value(compare(&design.netlist, &found)?).map_err(|err| CommandError::Unavailable(err.to_string()))
    }
}

#[cfg(test)]
mod tests;
