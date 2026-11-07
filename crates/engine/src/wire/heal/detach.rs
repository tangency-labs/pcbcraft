//! Intentional symbol disconnection and explicit wire-end contacts at gesture completion.
use super::*;

impl Edit {
    /// Join only the requested degree-one ends, excluding their moving wires from targets.
    /// Resolve each contact against the current graph so two drops onto one route can split
    /// it successively. Coordinate coincidence never chooses between distinct identities.
    pub(super) fn connect_wire_ends(&mut self, ends: &[u64]) -> Result<(), CommandError> {
        let mut excluded = BTreeSet::new();
        for &id in ends {
            let end = WireEndpoint::Junction(id);
            let wires: Vec<_> = self.project.wires.iter().filter(|wire| wire.endpoints.contains(&end)).collect();
            if wires.len() != 1 || !self.project.junctions.contains_key(&id) {
                return Err(CommandError::InvalidParams("connection must name a degree-one free end".into()));
            }
            excluded.insert(wires[0].ordinal);
        }
        for &id in ends {
            let source = WireEndpoint::Junction(id);
            let point = self.project.endpoint_position(source).ok_or_else(|| CommandError::InvalidParams("free end no longer exists".into()))?;
            let mut found = None;
            for wire in self.project.wires.iter().filter(|wire| !excluded.contains(&wire.ordinal)) {
                let path = self.project.wire_path(wire);
                if !path.windows(2).any(|pair| on_segment(point, pair[0], pair[1])) {
                    continue;
                }
                let target = if path.first() == Some(&point) {
                    Anchor::Existing(wire.endpoints[0])
                } else if path.last() == Some(&point) {
                    Anchor::Existing(wire.endpoints[1])
                } else {
                    Anchor::Split { wire: wire.ordinal, at: point }
                };
                if found.is_some_and(|old| old != target) {
                    return Err(CommandError::InvalidParams("wire drop meets ambiguous wire contacts".into()));
                }
                found = Some(target);
            }
            if let Some(target) = found {
                let pins: Vec<_> = self
                    .project
                    .symbols
                    .iter()
                    .flat_map(|symbol| symbol.terminals().into_iter().zip(symbol.pins()))
                    .filter(|(_, pin)| *pin == point)
                    .map(|(pin, _)| pin)
                    .collect();
                if pins.len() > 1 {
                    return Err(CommandError::InvalidParams("wire drop meets coincident component pins".into()));
                }
                let target = self.resolve(target)?;
                let joined = self.merge(source, target)?;
                if let Some(&pin) = pins.first() {
                    self.merge(joined, WireEndpoint::Terminal(pin))?;
                }
            }
        }
        Ok(())
    }
}

impl Project {
    /// Move a symbol while leaving its old routes and pin branches in place. Every vacated
    /// connected pin becomes one junction shared by its old wires. Preview and commit use
    /// the same bounded allocator; invalid geometry fails without changing the snapshot.
    /// Moving to the same position is a no-op. Exact contacts at the destination still join.
    pub fn moved_symbol_disconnected(&self, symbol: &str, to: Point, revision: u64, auto_heal: bool) -> Result<Project, CommandError> {
        let index = super::super::parse_id(symbol, "symbol-")
            .and_then(|id| self.symbol_index(id))
            .ok_or_else(|| CommandError::InvalidParams("symbol does not exist".into()))?;
        if !valid_point(&to) {
            return Err(CommandError::InvalidParams("symbol exceeds schematic bounds".into()));
        }
        if self.symbols[index].position == to {
            return Ok(self.clone());
        }
        let mut edit = Edit::new(self, revision);
        let mut replacements = BTreeMap::new();
        for pin in self.symbols[index].terminals() {
            let end = WireEndpoint::Terminal(pin);
            if self.wires.iter().any(|wire| wire.endpoints.contains(&end)) {
                let point = self.endpoint_position(end).ok_or_else(|| CommandError::InvalidParams("pin no longer exists".into()))?;
                replacements.insert(end, edit.free(point)?);
            }
        }
        for (i, wire) in self.wires.iter().enumerate() {
            if wire.endpoints.iter().any(|end| replacements.contains_key(end)) {
                let path = self.wire_path(wire);
                let next = &mut Arc::make_mut(&mut edit.project.wires)[i];
                next.bends = Some(super::super::interior(&path)?);
                next.endpoints = wire.endpoints.map(|end| replacements.get(&end).copied().unwrap_or(end));
            }
        }
        edit.project.connectivity = Arc::default();
        let detached = edit.project;
        let mut moved = detached.clone();
        Arc::make_mut(&mut moved.symbols)[index].position = to;
        moved.healed(&detached, revision, auto_heal)
    }
}
