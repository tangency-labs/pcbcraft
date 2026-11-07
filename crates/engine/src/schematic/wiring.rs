use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::{SymbolDefinition, SymbolPlacement, parse_symbol_id, symbol_id};

pub(crate) const MAX_WIRES: usize = 4096;

/// A pin number as libraries spell it: 1 to 8 ASCII letters, digits, `+`, `-` or `_`, such as
/// `1`, `A3` or `VCC`. It is stored inline, so terminals are small, copyable and own their
/// number instead of borrowing it from a definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PinNumber {
    bytes: [u8; 8],
    len: u8,
}

impl PinNumber {
    /// The pin number for this text, or `None` when it breaks the spelling rule above.
    pub fn new(text: &str) -> Option<Self> {
        let valid = (1..=8).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'_'));
        if !valid {
            return None;
        }
        let mut bytes = [0; 8];
        bytes[..text.len()].copy_from_slice(text.as_bytes());
        Some(Self { bytes, len: text.len() as u8 })
    }

    /// The number as text.
    pub fn as_str(&self) -> &str {
        // Only validated ASCII is ever stored, so this always succeeds.
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or_default()
    }
}

/// An exact terminal on a symbol. Coincident coordinates never establish a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Terminal {
    pub(crate) symbol: u64,
    number: PinNumber,
}

impl Terminal {
    /// Return a placed symbol's terminals, one per pin of its definition, in pin order.
    pub(super) fn for_symbol(symbol: u64, definition: &SymbolDefinition) -> Vec<Self> {
        definition.pins.iter().filter_map(|pin| Some(Self { symbol, number: PinNumber::new(&pin.number)? })).collect()
    }

    /// Read the two required fields and reject IDs that cannot come from project inspection.
    /// The pin number must be well formed; whether this symbol has that pin is checked against
    /// the project, as [`crate::Project::terminal_position`] does.
    pub fn parse(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let symbol = parse_symbol_id(object.get("symbol")?.as_str()?)?;
        if object.len() != 2 {
            return None;
        }
        let number = PinNumber::new(object.get("terminal")?.as_str()?)?;
        Some(Self { symbol, number })
    }

    /// Return the definition's pin number for display and persistence.
    pub fn number(&self) -> &str {
        self.number.as_str()
    }

    /// Resolve a stored pin number on a placed symbol, or `None` when its definition has no such pin.
    pub(crate) fn from_number(symbol: u64, definition: &SymbolDefinition, number: &str) -> Option<Self> {
        definition.pins.iter().find(|pin| pin.number == number)?;
        Some(Self { symbol, number: PinNumber::new(number)? })
    }

    /// Publish the terminal reference in the same shape that the command accepts.
    pub fn inspect(self) -> Value {
        json!({"symbol": symbol_id(self.symbol), "terminal": self.number.as_str()})
    }
}

/// An explicit electrical endpoint. Geometry alone never joins distinct endpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WireEndpoint {
    Terminal(Terminal),
    Junction(u64),
}

impl WireEndpoint {
    /// Publish an endpoint in the shared command and inspection representation.
    pub fn inspect(self) -> Value {
        match self {
            Self::Terminal(terminal) => terminal.inspect(),
            Self::Junction(id) => json!({"junction": format!("junction-{id}")}),
        }
    }

    /// Parse an existing terminal or junction identity, rejecting extra fields.
    pub fn parse(value: &Value) -> Option<Self> {
        if let Some(terminal) = Terminal::parse(value) {
            return Some(Self::Terminal(terminal));
        }
        let object = value.as_object()?;
        if object.len() != 1 {
            return None;
        }
        parse_id(object.get("junction")?.as_str()?, "junction-").map(Self::Junction)
    }
}

/// Read a canonical positive object ID without accepting alternate spellings.
pub(crate) fn parse_id(value: &str, prefix: &str) -> Option<u64> {
    let digits = value.strip_prefix(prefix)?;
    let id = digits.parse::<u64>().ok()?;
    (id > 0 && id <= crate::MAX_OBJECT_ID && !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit())).then_some(id)
}

/// An electrical edge with optional authored bends. Older pin links retain automatic routing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wire {
    pub(crate) ordinal: u64,
    pub(crate) endpoints: [WireEndpoint; 2],
    pub(crate) bends: Option<Vec<super::Point>>,
}

impl Wire {
    /// The stable ID used to select or delete this wire through the shared command.
    pub fn id(&self) -> String {
        format!("wire-{}", self.ordinal)
    }

    /// The stable numeric identity used by explicit wire-segment targets.
    pub fn ordinal(&self) -> u64 {
        self.ordinal
    }

    /// The exact terminal or junction identities joined by this edge.
    pub fn endpoints(&self) -> [WireEndpoint; 2] {
        self.endpoints
    }

    /// Return the directly attached symbol terminals, excluding free endpoints and junctions.
    pub fn terminals(&self) -> Vec<Terminal> {
        self.endpoints
            .iter()
            .filter_map(|end| match end {
                WireEndpoint::Terminal(pin) => Some(*pin),
                WireEndpoint::Junction(_) => None,
            })
            .collect()
    }

    /// Return authored bends, or None for a legacy automatically routed pin link.
    pub fn bends(&self) -> Option<&[super::Point]> {
        self.bends.as_deref()
    }

    /// Publish the persistent identity, endpoints and authored geometry without display caches.
    pub(crate) fn inspect(&self) -> Value {
        let [from, to] = self.endpoints;
        let mut out = json!({"id": self.id(), "from": from.inspect(), "to": to.inspect()});
        if let Some(bends) = &self.bends {
            out["bends"] = json!(bends);
        }
        out
    }
}

/// The typed connectivity of a project snapshot, derived from explicit edges and endpoints.
#[derive(Debug)]
pub struct Connectivity {
    pub nets: Vec<Vec<Terminal>>,
    pub unconnected: Vec<Terminal>,
    junctions: Vec<Vec<u64>>,
    by_terminal: BTreeMap<Terminal, usize>,
}

impl Connectivity {
    /// Return this snapshot's Net Inspector row index. This is presentation ordering, not a
    /// persistent electrical net identity, and can change after wiring edits.
    pub fn net_index_for(&self, terminal: Terminal) -> Option<usize> {
        self.by_terminal.get(&terminal).copied()
    }

    /// Find the connected group for a terminal without walking the schematic.
    pub fn net_for(&self, terminal: Terminal) -> Option<&[Terminal]> {
        self.by_terminal.get(&terminal).map(|&index| self.nets[index].as_slice())
    }

    /// Preserve the existing terminal report and include junction identities for free wiring.
    pub(crate) fn inspect(&self) -> Value {
        let nets: Vec<_> = self
            .nets
            .iter()
            .zip(&self.junctions)
            .map(|(group, junctions)| {
                let mut net = json!({"terminals": group.iter().copied().map(Terminal::inspect).collect::<Vec<_>>()});
                if !junctions.is_empty() {
                    net["junctions"] = json!(junctions.iter().map(|id| format!("junction-{id}")).collect::<Vec<_>>());
                }
                net
            })
            .collect();
        json!({"nets": nets, "unconnected": self.unconnected.iter().copied().map(Terminal::inspect).collect::<Vec<_>>()})
    }

    /// Walk explicit endpoint identities; crossings and coincident distinct points stay separate.
    pub(crate) fn build(symbols: &[SymbolPlacement], wires: &[Wire]) -> Self {
        let mut adjacency = BTreeMap::<WireEndpoint, Vec<WireEndpoint>>::new();
        for wire in wires {
            let [from, to] = wire.endpoints;
            adjacency.entry(from).or_default().push(to);
            adjacency.entry(to).or_default().push(from);
        }
        let unconnected = symbols
            .iter()
            .flat_map(SymbolPlacement::terminals)
            .filter(|terminal| !adjacency.contains_key(&WireEndpoint::Terminal(*terminal)))
            .collect();
        let mut nets = Vec::new();
        let mut junctions = Vec::new();
        let mut by_terminal = BTreeMap::new();
        while let Some((&first, _)) = adjacency.first_key_value() {
            let mut pending = vec![first];
            let mut group = BTreeSet::new();
            let mut points = BTreeSet::new();
            while let Some(endpoint) = pending.pop() {
                if let Some(neighbors) = adjacency.remove(&endpoint) {
                    match endpoint {
                        WireEndpoint::Terminal(terminal) => {
                            group.insert(terminal);
                        }
                        WireEndpoint::Junction(id) => {
                            points.insert(id);
                        }
                    }
                    pending.extend(neighbors);
                }
            }
            by_terminal.extend(group.iter().map(|&terminal| (terminal, nets.len())));
            nets.push(group.into_iter().collect());
            junctions.push(points.into_iter().collect());
        }
        Self { nets, junctions, unconnected, by_terminal }
    }
}

/// Build an orthogonal path through anchors; diagonal gaps get one deterministic elbow.
/// Duplicate and collinear intermediate points are removed, retaining endpoint coordinates.
pub fn orthogonal_path(anchors: impl IntoIterator<Item = super::Point>, horizontal_first: bool) -> Vec<super::Point> {
    let mut path: Vec<super::Point> = Vec::new();
    for point in anchors {
        if let Some(previous) = path.last().copied()
            && previous.x_nm != point.x_nm
            && previous.y_nm != point.y_nm
        {
            let elbow = if horizontal_first {
                super::Point { x_nm: point.x_nm, y_nm: previous.y_nm }
            } else {
                super::Point { x_nm: previous.x_nm, y_nm: point.y_nm }
            };
            append_point(&mut path, elbow);
        }
        append_point(&mut path, point);
    }
    path
}

/// Append a path point while collapsing only redundant points on the same straight run.
fn append_point(path: &mut Vec<super::Point>, point: super::Point) {
    if path.last() == Some(&point) {
        return;
    }
    if path.len() >= 2 {
        let a = path[path.len() - 2];
        let b = path[path.len() - 1];
        let between = |a: i64, b: i64, c: i64| (a.min(c)..=a.max(c)).contains(&b);
        if (a.x_nm == b.x_nm && b.x_nm == point.x_nm && between(a.y_nm, b.y_nm, point.y_nm))
            || (a.y_nm == b.y_nm && b.y_nm == point.y_nm && between(a.x_nm, b.x_nm, point.x_nm))
        {
            path.pop();
        }
    }
    path.push(point);
}
