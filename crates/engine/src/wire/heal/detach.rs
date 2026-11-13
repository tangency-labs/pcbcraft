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
            let named =
                self.project.net_labels().iter().any(|label| label.at == point) || self.project.power_terminals().any(|(_, _, at)| at == point);
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
    /// Partition electrical contacts at a validated selection boundary without moving geometry.
    /// Each cut creates one free junction shared by the wires on its detached side. Moving
    /// junctions keep their IDs (and attached intent); stationary routes retain their exact paths.
    /// The ordinary edit allocator bounds new IDs, and failure discards the entire candidate.
    pub(crate) fn detached_selection(&self, items: &crate::selection::Selection, revision: u64) -> Result<Project, CommandError> {
        let mut incident = BTreeMap::<WireEndpoint, (bool, bool)>::new();
        for wire in self.wires.iter() {
            let moving = items.wires.contains(&wire.id());
            for end in wire.endpoints {
                let sides = incident.entry(end).or_default();
                if moving {
                    sides.0 = true;
                } else {
                    sides.1 = true;
                }
            }
        }
        let mut edit = Edit::new(self, revision);
        let mut cuts = BTreeMap::new();
        for (end, (moving_wires, fixed_wires)) in incident {
            let moving_owner = match end {
                WireEndpoint::Terminal(pin) => items.symbols.contains(&format!("symbol-{}", pin.symbol)),
                WireEndpoint::NetLabel(id) => items.net_labels.contains(&format!("net-label-{id}")),
                WireEndpoint::Junction(_) => moving_wires,
                WireEndpoint::Label(_) | WireEndpoint::SheetPin(_) => false,
            };
            if (moving_owner && fixed_wires) || (!moving_owner && moving_wires) {
                let at = self.endpoint_position(end).ok_or_else(|| CommandError::InvalidParams("selection contact no longer exists".into()))?;
                cuts.insert(end, (!moving_owner, edit.free(at)?));
            }
        }
        for (index, wire) in self.wires.iter().enumerate() {
            let moving = items.wires.contains(&wire.id());
            let endpoints =
                wire.endpoints.map(|end| cuts.get(&end).filter(|(cut_moving, _)| *cut_moving == moving).map_or(end, |(_, replacement)| *replacement));
            if endpoints != wire.endpoints {
                let path = self.wire_path(wire);
                let next = &mut Arc::make_mut(&mut edit.project.sheet_mut().wires)[index];
                next.bends = Some(super::super::interior(&path)?);
                next.endpoints = endpoints;
            }
        }
        edit.project.sheet_mut().connectivity = Arc::default();
        Ok(edit.project)
    }

    /// Move a symbol while leaving its old routes and pin branches in place. Every vacated
    /// connected pin becomes one junction shared by its old wires. Preview and commit use
    /// the same bounded allocator; invalid geometry fails without changing the snapshot.
    /// Moving to the same position is a no-op. Exact contacts at the destination still join.
    pub fn moved_symbol_disconnected(&self, symbol: &str, to: Point, revision: u64, auto_heal: bool) -> Result<Project, CommandError> {
        self.transformed_symbol(symbol, to, None, None, true, revision, auto_heal)
    }

    /// Detach the validated symbol's old pin contacts before applying its new pose, so
    /// preview and commit leave the original wire paths fixed even when only rotating or mirroring.
    pub(super) fn transformed_symbol_disconnected(
        &self,
        index: usize,
        to: Point,
        rotation: u8,
        mirrored: bool,
        revision: u64,
        auto_heal: bool,
    ) -> Result<Project, CommandError> {
        let items = crate::selection::Selection::one_symbol(self.symbols[index].id());
        let detached = self.detached_selection(&items, revision)?;
        let mut moved = detached.clone();
        let symbol = &mut Arc::make_mut(&mut moved.sheet_mut().symbols)[index];
        symbol.position = to;
        symbol.rotation = rotation;
        symbol.mirrored = mirrored;
        moved.healed(&detached, revision, auto_heal)
    }
}
