use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::{ResistorPlacement, symbol_id};

pub(crate) const MAX_WIRES: usize = 4096;

/// An exact terminal on a symbol. Coincident coordinates never establish a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Terminal {
    pub(crate) symbol: u64,
    number: &'static str,
}

impl Terminal {
    /// Read the two required fields and reject IDs that cannot come from project inspection.
    pub(crate) fn parse(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let symbol_id = object.get("symbol")?.as_str()?.strip_prefix("symbol-")?;
        let symbol = symbol_id.parse::<u64>().ok()?;
        if object.len() != 2 || symbol == 0 || symbol_id != symbol.to_string() {
            return None;
        }
        let number = match object.get("terminal")?.as_str()? {
            "1" => "1",
            "2" => "2",
            _ => return None,
        };
        Some(Self { symbol, number })
    }

    /// Publish the terminal reference in the same shape that the command accepts.
    fn inspect(self) -> Value {
        json!({"symbol": symbol_id(self.symbol), "terminal": self.number})
    }
}

/// An undirected link between two terminals. The sorted endpoints make duplicates comparable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Wire {
    pub(crate) ordinal: u64,
    pub(crate) terminals: [Terminal; 2],
}

impl Wire {
    /// Return the persistent wire ID within this session and its two endpoints.
    pub(crate) fn inspect(self) -> Value {
        let [from, to] = self.terminals;
        json!({"id": format!("wire-{}", self.ordinal), "from": from.inspect(), "to": to.inspect()})
    }
}

/// Derive connected groups and isolated terminals in stable order from explicit wire links.
pub(crate) fn inspect_netlist(symbols: &[ResistorPlacement], wires: &[Wire]) -> Value {
    let mut adjacency = BTreeMap::<Terminal, Vec<Terminal>>::new();
    for wire in wires {
        let [from, to] = wire.terminals;
        adjacency.entry(from).or_default().push(to);
        adjacency.entry(to).or_default().push(from);
    }

    let unconnected: Vec<_> = symbols
        .iter()
        .flat_map(|symbol| ["1", "2"].map(|number| Terminal { symbol: symbol.ordinal, number }))
        .filter(|terminal| !adjacency.contains_key(terminal))
        .map(Terminal::inspect)
        .collect();

    let mut nets = Vec::new();
    while let Some((&first, _)) = adjacency.first_key_value() {
        let mut pending = vec![first];
        let mut group = BTreeSet::new();
        while let Some(terminal) = pending.pop() {
            if let Some(neighbors) = adjacency.remove(&terminal) {
                group.insert(terminal);
                pending.extend(neighbors);
            }
        }
        nets.push(json!({"terminals": group.into_iter().map(Terminal::inspect).collect::<Vec<_>>()}));
    }
    json!({"nets": nets, "unconnected": unconnected})
}
