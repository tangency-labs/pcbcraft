//! Capture a named marker's actual contacts before dragging it away from their old position.
use super::*;

impl Edit {
    /// Apply a validated pose while retaining exact contacts of an offset naming terminal.
    /// Capture against the old geometry, so fixed pins and unsplit interiors remain connected.
    pub(super) fn pose_symbol(&mut self, index: usize, to: Point, rotation: u8) -> Result<(), CommandError> {
        let symbol = self.project.symbols().get(index).ok_or_else(|| CommandError::InvalidParams("symbol does not exist".into()))?;
        let contact = self.project.power_terminals().find(|(end, _, from)| {
            let WireEndpoint::Terminal(pin) = end else { return false };
            pin.symbol == symbol.ordinal
                && symbol.unit_definition().pin_index(pin.number()).is_some_and(|i| symbol.unit_definition().pin_positions(to, rotation)[i] != *from)
        });
        let captured = match contact {
            Some((end, _, from)) => Some((end, from, self.capture_named_contact(end, from)?)),
            None => None,
        };
        let symbol = &mut Arc::make_mut(&mut self.project.sheet_mut().symbols)[index];
        symbol.position = to;
        symbol.rotation = rotation;
        if let Some((end, from, fixed)) = captured {
            let at = self.project.endpoint_position(end).ok_or_else(|| CommandError::InvalidParams("missing named terminal".into()))?;
            self.retain_fixed_contacts(end, from, at, fixed)?;
        }
        self.project.named_graph = Arc::default();
        Ok(())
    }

    /// Retain all exact authored wire contacts of a globally naming terminal. Fixed pins stay
    /// put and receive a connecting leg; wire interiors split using the shared bounded allocator.
    pub(super) fn capture_named_contact(&mut self, end: WireEndpoint, from: Point) -> Result<BTreeSet<WireEndpoint>, CommandError> {
        let mut fixed: BTreeSet<_> = self
            .project
            .symbols()
            .iter()
            .flat_map(|s| s.terminals().into_iter().map(WireEndpoint::Terminal).zip(s.pins()))
            .chain(self.project.hierarchy_endpoints())
            .filter_map(|(other, at)| (other != end && at == from).then_some(other))
            .collect();
        let wires: Vec<_> = self.project.wires().to_vec();
        for wire in wires {
            self.compare()?;
            if wire.endpoints().contains(&end) || !super::super::path_contains_point(&self.project.wire_path(&wire), from) {
                continue;
            }
            let contact = self.split(wire.ordinal(), from)?;
            match contact {
                WireEndpoint::Junction(_) => {
                    self.merge(contact, end)?;
                }
                other if other != end => {
                    fixed.insert(other);
                }
                _ => {}
            }
        }
        Ok(fixed)
    }
    /// Connect a moved contact back to fixed terminals without duplicate wire identities.
    pub(super) fn retain_fixed_contacts(
        &mut self,
        end: WireEndpoint,
        from: Point,
        to: Point,
        fixed: BTreeSet<WireEndpoint>,
    ) -> Result<(), CommandError> {
        for other in fixed {
            if !self.project.wires().iter().any(|wire| wire.endpoints().contains(&other) && wire.endpoints().contains(&end)) {
                let path = crate::schematic::orthogonal_path([from, to], true);
                self.draw([other, end], &path)?;
            }
        }
        self.touched.extend(self.project.wires().iter().filter(|w| w.endpoints().contains(&end)).map(|w| w.ordinal()));
        Ok(())
    }
}

impl Project {
    /// Set a current-sheet pose while preserving naming-terminal contacts. Existing explicit
    /// routes keep their endpoints and authored bends; no automatic route cleanup is performed.
    /// Invalid indices, coordinates, orientations or bounded contact allocations fail atomically.
    pub(crate) fn retained_symbol_pose(&self, index: usize, to: Point, rotation: u8, revision: u64) -> Result<Self, CommandError> {
        if !valid_point(&to) || rotation > 3 {
            return Err(CommandError::InvalidParams("invalid symbol pose".into()));
        }
        let mut edit = Edit::new(self, revision);
        edit.pose_symbol(index, to, rotation)?;
        if edit.touched.is_empty() { Ok(edit.project) } else { edit.finish(false) }
    }

    /// Move an existing named marker with its touching wires, independent of placement order.
    /// Free-end/branch contacts become explicit marker endpoints; interior routes split at
    /// the old anchor. Part instance pins and other named anchors stay fixed and gain a connecting
    /// leg. Preview and commit allocate at the same revision. Invalid geometry, exhausted IDs
    /// or wire limits reject the entire candidate; rotation alone never rewrites contacts.
    pub fn dragged_net_label(&self, id: u64, to: Point, rotation: u16, revision: u64, auto_heal: bool) -> Result<Self, CommandError> {
        let preview = self.preview_net_label(id, to, rotation)?;
        let end = WireEndpoint::NetLabel(id);
        let from = self.endpoint_position(end).ok_or_else(|| CommandError::InvalidParams("unknown net label on this sheet".into()))?;
        if from == to {
            return Ok(preview);
        }
        let mut edit = Edit::new(self, revision);
        let fixed = edit.capture_named_contact(end, from)?;
        edit.project = edit.project.preview_net_label(id, to, rotation)?;
        edit.retain_fixed_contacts(end, from, to, fixed)?;
        if edit.touched.is_empty() { Ok(edit.project) } else { edit.finish(auto_heal) }
    }
}
