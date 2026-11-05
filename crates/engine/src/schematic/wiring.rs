use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::{SymbolDefinition, SymbolPlacement, known_pin_number, parse_symbol_id, symbol_id};

pub(crate) const MAX_WIRES: usize = 4096;

/// An exact terminal on a symbol. Coincident coordinates never establish a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Terminal {
    pub(crate) symbol: u64,
    number: &'static str,
}

impl Terminal {
    /// Return every terminal of a placed symbol, in its definition's pin order.
    pub(super) fn for_symbol(symbol: u64, definition: &'static SymbolDefinition) -> Vec<Self> {
        definition.pins.iter().map(|pin| Self { symbol, number: pin.number }).collect()
    }

    /// Read the two required fields and reject IDs that cannot come from project inspection.
    /// The pin number must be one some built-in definition uses; the command then checks that
    /// this symbol has it, so a fan connector's pin 8 parses here and is refused there.
    pub fn parse(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let symbol = parse_symbol_id(object.get("symbol")?.as_str()?)?;
        if object.len() != 2 {
            return None;
        }
        let number = known_pin_number(object.get("terminal")?.as_str()?)?;
        Some(Self { symbol, number })
    }

    /// Pin 1 or pin 2 of a two-pin symbol, for the two-pad starter footprint. Footprints only
    /// link to symbols with exactly those pins; the loader checks that.
    pub(crate) fn two_pin(symbol: u64, second: bool) -> Self {
        Self { symbol, number: if second { "2" } else { "1" } }
    }

    /// Return the definition's pin number for display and persistence.
    pub fn number(self) -> &'static str {
        self.number
    }

    /// Resolve a stored pin number for a symbol with this definition, or `None` when it has no
    /// such pin.
    pub(crate) fn from_number(symbol: u64, definition: &'static SymbolDefinition, number: &str) -> Option<Self> {
        definition.pin(number).map(|pin| Self { symbol, number: pin.number })
    }

    /// Publish the terminal reference in the same shape that the command accepts.
    pub fn inspect(self) -> Value {
        json!({"symbol": symbol_id(self.symbol), "terminal": self.number})
    }
}

/// An undirected link between two terminals. The sorted endpoints make duplicates comparable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wire {
    pub(crate) ordinal: u64,
    pub(crate) terminals: [Terminal; 2],
}

impl Wire {
    /// Return the exact symbol terminals joined by this wire.
    pub fn terminals(&self) -> [Terminal; 2] {
        self.terminals
    }

    /// Return the persistent wire ID within this session and its two endpoints.
    pub(crate) fn inspect(self) -> Value {
        let [from, to] = self.terminals;
        json!({"id": format!("wire-{}", self.ordinal), "from": from.inspect(), "to": to.inspect()})
    }
}

/// The typed connectivity of a project snapshot. It is derived from wires and symbols.
#[derive(Debug)]
pub struct Connectivity {
    pub nets: Vec<Vec<Terminal>>,
    pub unconnected: Vec<Terminal>,
    by_terminal: BTreeMap<Terminal, usize>,
}

impl Connectivity {
    /// Find the connected group for a terminal without walking the schematic.
    pub fn net_for(&self, terminal: Terminal) -> Option<&[Terminal]> {
        self.by_terminal.get(&terminal).map(|&index| self.nets[index].as_slice())
    }

    /// Serialize the same inspection contract used by existing MCP callers.
    pub(crate) fn inspect(&self) -> Value {
        let nets: Vec<_> =
            self.nets.iter().map(|group| json!({"terminals": group.iter().copied().map(Terminal::inspect).collect::<Vec<_>>()})).collect();
        json!({"nets": nets, "unconnected": self.unconnected.iter().copied().map(Terminal::inspect).collect::<Vec<_>>()})
    }

    /// Derive connected groups and isolated terminals in stable order from explicit wire links.
    pub(crate) fn build(symbols: &[SymbolPlacement], wires: &[Wire]) -> Self {
        let mut adjacency = BTreeMap::<Terminal, Vec<Terminal>>::new();
        for wire in wires {
            let [from, to] = wire.terminals;
            adjacency.entry(from).or_default().push(to);
            adjacency.entry(to).or_default().push(from);
        }
        let unconnected = symbols.iter().flat_map(SymbolPlacement::terminals).filter(|terminal| !adjacency.contains_key(terminal)).collect();
        let mut nets = Vec::new();
        let mut by_terminal = BTreeMap::new();
        while let Some((&first, _)) = adjacency.first_key_value() {
            let mut pending = vec![first];
            let mut group = BTreeSet::new();
            while let Some(terminal) = pending.pop() {
                if let Some(neighbors) = adjacency.remove(&terminal) {
                    group.insert(terminal);
                    pending.extend(neighbors);
                }
            }
            by_terminal.extend(group.iter().map(|&terminal| (terminal, nets.len())));
            nets.push(group.into_iter().collect());
        }
        Self { nets, unconnected, by_terminal }
    }
}
