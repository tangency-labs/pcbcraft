//! Capture a named marker's actual contacts before dragging it away from their old position.
use super::*;

impl Edit {
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
    /// Move an existing named marker with its touching wires, independent of placement order.
    /// Free-end/branch contacts become explicit marker endpoints; interior routes split at
    /// the old anchor. Component pins and other named anchors stay fixed and gain a connecting
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
