use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::{SymbolPlacement, SymbolUnit, parse_symbol_id, symbol_id};

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
    pub(super) fn for_symbol(symbol: u64, definition: &SymbolUnit) -> Vec<Self> {
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
    pub(crate) fn from_number(symbol: u64, definition: &SymbolUnit, number: &str) -> Option<Self> {
        definition.pins.iter().find(|pin| pin.number == number)?;
        Some(Self { symbol, number: PinNumber::new(number)? })
    }

    /// Publish the terminal reference in the same shape that the command accepts.
    pub fn inspect(self) -> Value {
        json!({"symbol": symbol_id(self.symbol), "terminal": self.number.as_str()})
    }
}

/// An explicit electrical endpoint. Ordinary coincident endpoints remain separate unless
/// joined by a wire or an authored named contact marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WireEndpoint {
    Terminal(Terminal),
    Junction(u64),
    Label(u64),
    NetLabel(u64),
    SheetPin(u64),
}

impl WireEndpoint {
    /// Publish an endpoint in the shared command and inspection representation.
    pub fn inspect(self) -> Value {
        match self {
            Self::Terminal(terminal) => terminal.inspect(),
            Self::NetLabel(id) => json!({"net_label":format!("net-label-{id}")}),
            Self::Label(id) => json!({"label":format!("label-{id}")}),
            Self::SheetPin(id) => json!({"sheet_pin":format!("label-{id}")}),
            Self::Junction(id) => json!({"junction": format!("junction-{id}")}),
        }
    }

    /// Parse a terminal, junction, named/hierarchical label or sheet pin, rejecting extra fields.
    pub fn parse(value: &Value) -> Option<Self> {
        if let Some(terminal) = Terminal::parse(value) {
            return Some(Self::Terminal(terminal));
        }
        let object = value.as_object()?;
        if object.len() != 1 {
            return None;
        }
        if let Some(id) = object.get("net_label").and_then(Value::as_str).and_then(|s| parse_id(s, "net-label-")) {
            return Some(Self::NetLabel(id));
        }
        if let Some(id) = object.get("label").and_then(Value::as_str).and_then(|s| parse_id(s, "label-")) {
            return Some(Self::Label(id));
        }
        if let Some(id) = object.get("sheet_pin").and_then(Value::as_str).and_then(|s| parse_id(s, "label-")) {
            return Some(Self::SheetPin(id));
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

    /// Return directly attached component terminals; hierarchy anchors have separate identities.
    pub fn terminals(&self) -> Vec<Terminal> {
        self.endpoints
            .iter()
            .filter_map(|end| match end {
                WireEndpoint::Terminal(pin) => Some(*pin),
                _ => None,
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
#[derive(Clone, Debug)]
pub struct Connectivity {
    pub nets: Vec<Vec<Terminal>>,
    pub(crate) names: Vec<Vec<String>>,
    pub unconnected: Vec<Terminal>,
    junctions: Vec<Vec<u64>>,
    interfaces: Vec<Vec<WireEndpoint>>,
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

    /// Report terminal groups, junction/interface identities and any resolved authored names.
    pub(crate) fn inspect(&self) -> Value {
        let nets: Vec<_> = self
            .nets
            .iter()
            .zip(&self.junctions)
            .zip(&self.interfaces)
            .enumerate()
            .map(|(index, ((group, junctions), interfaces))| {
                let mut net = json!({"terminals": group.iter().copied().map(Terminal::inspect).collect::<Vec<_>>()});
                if !self.names[index].is_empty() {
                    net["name"] = json!(self.names[index][0]);
                    net["names"] = json!(self.names[index]);
                }
                if !junctions.is_empty() {
                    net["junctions"] = json!(junctions.iter().map(|id| format!("junction-{id}")).collect::<Vec<_>>());
                }
                if !interfaces.is_empty() {
                    net["interfaces"] = json!(interfaces.iter().map(|end| end.inspect()).collect::<Vec<_>>());
                }
                net
            })
            .collect();
        json!({"nets": nets, "unconnected": self.unconnected.iter().copied().map(Terminal::inspect).collect::<Vec<_>>()})
    }

    /// Walk explicit endpoint identities; crossings and coincident distinct points stay separate.
    pub(crate) fn build(symbols: &[SymbolPlacement], wires: &[Wire]) -> Self {
        Self::build_linked(symbols, wires, std::iter::empty())
    }

    /// Include explicit hierarchy edges without merging names or coincident coordinates.
    pub(crate) fn build_linked(symbols: &[SymbolPlacement], wires: &[Wire], links: impl IntoIterator<Item = [WireEndpoint; 2]>) -> Self {
        let mut adjacency = BTreeMap::<WireEndpoint, Vec<WireEndpoint>>::new();
        for wire in wires {
            let [from, to] = wire.endpoints;
            adjacency.entry(from).or_default().push(to);
            adjacency.entry(to).or_default().push(from);
        }
        for [from, to] in links {
            adjacency.entry(from).or_default().push(to);
            adjacency.entry(to).or_default().push(from);
        }
        // Repeated appearances of the same logical terminal are electrical aliases, including
        // shared supply pins. Unit IDs and coordinates never establish separate physical pins.
        let mut logical = BTreeMap::<_, Vec<Terminal>>::new();
        for symbol in symbols.iter().filter(|symbol| symbol.definition.units.len() > 1) {
            for terminal in symbol.terminals() {
                logical.entry((symbol.component, terminal.number().to_owned())).or_default().push(terminal);
            }
        }
        for terminals in logical.values() {
            // Multiple drawings of an otherwise bare pin do not make it connected.
            if terminals.iter().any(|pin| adjacency.contains_key(&WireEndpoint::Terminal(*pin))) {
                for pair in terminals.windows(2) {
                    let (from, to) = (WireEndpoint::Terminal(pair[0]), WireEndpoint::Terminal(pair[1]));
                    adjacency.entry(from).or_default().push(to);
                    adjacency.entry(to).or_default().push(from);
                }
            }
        }
        let unconnected = symbols
            .iter()
            .flat_map(SymbolPlacement::terminals)
            .filter(|terminal| !adjacency.contains_key(&WireEndpoint::Terminal(*terminal)))
            .collect();
        let mut interfaces = Vec::new();
        let mut nets = Vec::new();
        let mut junctions = Vec::new();
        let mut by_terminal = BTreeMap::new();
        while let Some((&first, _)) = adjacency.first_key_value() {
            let mut pending = vec![first];
            let mut group = BTreeSet::new();
            let mut points = BTreeSet::new();
            let mut ports = BTreeSet::new();
            while let Some(endpoint) = pending.pop() {
                if let Some(neighbors) = adjacency.remove(&endpoint) {
                    match endpoint {
                        WireEndpoint::Terminal(terminal) => {
                            group.insert(terminal);
                        }
                        WireEndpoint::NetLabel(_) | WireEndpoint::Label(_) | WireEndpoint::SheetPin(_) => {
                            ports.insert(endpoint);
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
            interfaces.push(ports.into_iter().collect());
        }
        let names = vec![Vec::new(); nets.len()];
        Self { nets, names, junctions, interfaces, unconnected, by_terminal }
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

impl Connectivity {
    /// Preferred authored name, or a view-local number when the group is unnamed.
    pub fn net_name(&self, index: usize) -> String {
        self.names.get(index).and_then(|names| names.first()).cloned().unwrap_or_else(|| format!("Net {}", index + 1))
    }
    /// All authored names on this group, including aliases introduced by wired connections.
    pub fn net_names(&self, index: usize) -> &[String] {
        self.names.get(index).map_or(&[], Vec::as_slice)
    }
    /// Find a group from any endpoint, including label-only and free-wire groups.
    pub fn net_index_for_endpoint(&self, endpoint: WireEndpoint) -> Option<usize> {
        match endpoint {
            WireEndpoint::Terminal(pin) => self.net_index_for(pin),
            WireEndpoint::Junction(id) => self.junctions.iter().position(|group| group.contains(&id)),
            _ => self.interfaces.iter().position(|group| group.contains(&endpoint)),
        }
    }
    /// Derive deterministic names without storing a second persistent net identity.
    pub(crate) fn assign_names(&mut self, names: &BTreeMap<WireEndpoint, (u8, String)>) {
        for (index, ends) in self.interfaces.iter().enumerate() {
            let sorted: BTreeSet<_> = ends
                .iter()
                .copied()
                .chain(self.nets[index].iter().copied().map(WireEndpoint::Terminal))
                .filter_map(|end| names.get(&end))
                .cloned()
                .collect();
            let mut seen = BTreeSet::new();
            self.names[index] = sorted.into_iter().map(|(_, name)| name).filter(|name| seen.insert(name.clone())).collect();
        }
    }
    /// Project the complete graph onto local endpoints, retaining connections made remotely
    /// without displaying remote terminals as if they belonged to the current sheet.
    pub(crate) fn retain_endpoints(&mut self, own: &BTreeSet<WireEndpoint>) {
        let mut nets = Vec::new();
        let mut names = Vec::new();
        let mut junctions = Vec::new();
        let mut interfaces = Vec::new();
        for index in 0..self.nets.len() {
            let pins: Vec<_> = self.nets[index].iter().copied().filter(|pin| own.contains(&WireEndpoint::Terminal(*pin))).collect();
            let points: Vec<_> = self.junctions[index].iter().copied().filter(|id| own.contains(&WireEndpoint::Junction(*id))).collect();
            let ports: Vec<_> = self.interfaces[index].iter().copied().filter(|end| own.contains(end)).collect();
            if pins.is_empty() && points.is_empty() && ports.is_empty() {
                continue;
            }
            nets.push(pins);
            names.push(self.names[index].clone());
            junctions.push(points);
            interfaces.push(ports);
        }
        self.unconnected.retain(|pin| own.contains(&WireEndpoint::Terminal(*pin)));
        self.by_terminal = nets.iter().enumerate().flat_map(|(index, pins)| pins.iter().map(move |pin| (*pin, index))).collect();
        self.nets = nets;
        self.names = names;
        self.junctions = junctions;
        self.interfaces = interfaces;
    }
}
