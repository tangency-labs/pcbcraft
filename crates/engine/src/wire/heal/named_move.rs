//! Capture a named marker's actual contacts before dragging it away from their old position.
use super::*;

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
        let mut fixed: BTreeSet<_> = self
            .symbols()
            .iter()
            .flat_map(|symbol| symbol.terminals().into_iter().map(WireEndpoint::Terminal).zip(symbol.pins()))
            .chain(self.hierarchy_endpoints())
            .filter_map(|(other, at)| (other != end && at == from).then_some(other))
            .collect();
        for wire in self.wires() {
            edit.compare()?;
            if wire.endpoints().contains(&end) || !super::super::path_contains_point(&self.wire_path(wire), from) {
                continue;
            }
            let contact = edit.split(wire.ordinal(), from)?;
            match contact {
                WireEndpoint::Junction(_) => {
                    edit.merge(contact, end)?;
                }
                other if other != end => {
                    fixed.insert(other);
                }
                _ => {}
            }
        }
        edit.project = edit.project.preview_net_label(id, to, rotation)?;
        for other in fixed {
            if !edit.project.wires().iter().any(|wire| wire.endpoints().contains(&other) && wire.endpoints().contains(&end)) {
                let path = crate::schematic::orthogonal_path([from, to], true);
                edit.draw([other, end], &path)?;
            }
        }
        edit.touched.extend(edit.project.wires().iter().filter(|wire| wire.endpoints().contains(&end)).map(|wire| wire.ordinal()));
        if edit.touched.is_empty() { Ok(edit.project) } else { edit.finish(auto_heal) }
    }
}
