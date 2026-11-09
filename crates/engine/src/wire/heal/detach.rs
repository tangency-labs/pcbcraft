//! Intentional symbol disconnection and explicit wire-end contacts at gesture completion.
use super::*;

impl Edit {
    /// Join only the requested degree-one ends, excluding their moving wires from targets.
    /// Resolve each contact against the current graph so two drops onto one route can split
    /// it successively. A named anchor declares one contact without replacing existing fixed
    /// endpoint identities; undeclared coincident identities and crossings remain ambiguous.
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
            let named = self.project.net_labels().iter().any(|label| label.at == point);
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
                    if !named {
                        return Err(CommandError::InvalidParams("wire drop meets ambiguous wire contacts".into()));
                    }
                    if matches!(found, Some(Anchor::Split { .. })) && matches!(target, Anchor::Existing(_)) {
                        found = Some(target);
                    }
                    continue;
                }
                found = Some(target);
            }
            let mut joined = source;
            // An authored marker already bridges interior routes at this point. Attach to its
            // fixed contact rather than splitting a previously intact target wire unnecessarily.
            let target = match found {
                Some(Anchor::Split { .. }) if named => self.pin_contact(point, None)?.map(Anchor::Existing),
                target => target,
            };
            if let Some(target) = target {
                let target = self.resolve(target)?;
                joined = self.merge(source, target)?;
            }
            if let Some(pin) = self.pin_contact(point, found.map(|_| joined))? {
                self.merge(joined, pin)?;
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
        self.transformed_symbol(symbol, to, None, true, revision, auto_heal)
    }

    /// Detach the validated symbol's old pin contacts before applying its new pose, so
    /// preview and commit leave the original wire paths fixed even when only rotating.
    pub(super) fn transformed_symbol_disconnected(
        &self,
        index: usize,
        to: Point,
        rotation: u8,
        revision: u64,
        auto_heal: bool,
    ) -> Result<Project, CommandError> {
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
                let next = &mut Arc::make_mut(&mut edit.project.sheet_mut().wires)[i];
                next.bends = Some(super::super::interior(&path)?);
                next.endpoints = wire.endpoints.map(|end| replacements.get(&end).copied().unwrap_or(end));
            }
        }
        edit.project.sheet_mut().connectivity = Arc::default();
        let detached = edit.project;
        let mut moved = detached.clone();
        let symbol = &mut Arc::make_mut(&mut moved.sheet_mut().symbols)[index];
        symbol.position = to;
        symbol.rotation = rotation;
        moved.healed(&detached, revision, auto_heal)
    }
}
