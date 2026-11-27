//! Capture a named marker's actual contacts before dragging it away from their old position.
use super::*;

/// A naming terminal, its old point and the fixed contacts it keeps, when a pose moves it.
type Captured = Option<(WireEndpoint, Point, BTreeSet<WireEndpoint>)>;

impl Edit {
    /// Apply a validated pose while retaining exact contacts of an offset naming terminal.
    /// Capture against the old geometry, so fixed pins and unsplit interiors remain connected.
    /// Refresh marker attachment restrictions for the returned pose before subsequent cleanup.
    pub(super) fn pose_symbol(&mut self, index: usize, to: Point, rotation: u8, mirrored: bool) -> Result<(), CommandError> {
        let captured = self.capture_pose(index, to, rotation, mirrored, &BTreeSet::new(), &BTreeSet::new())?;
        self.place_pose(index, to, rotation, mirrored, captured)
    }

    /// Capture the exact contacts an offset naming terminal must keep when its symbol takes a
    /// pose, against the current geometry. Pins of the other `moving` symbols are not fixed:
    /// they take their own poses in the same rigid batch and stay in contact by geometry.
    fn capture_pose(
        &mut self,
        index: usize,
        to: Point,
        rotation: u8,
        mirrored: bool,
        moving: &BTreeSet<u64>,
        reflected_routes: &BTreeSet<u64>,
    ) -> Result<Captured, CommandError> {
        let symbol = self.project.symbols().get(index).ok_or_else(|| CommandError::NotFound("symbol does not exist".into()))?;
        let own = symbol.ordinal;
        let contact = self.project.power_terminals().find_map(|(end, _, from)| {
            let WireEndpoint::Terminal(pin) = end else { return None };
            if pin.symbol != symbol.ordinal {
                return None;
            }
            let index = symbol.unit_definition().pin_index(pin.number())?;
            let target = symbol.unit_definition().pin_positions(to, rotation, mirrored)[index];
            (target != from).then_some((end, from, target))
        });
        let Some((end, from, target)) = contact else { return Ok(None) };
        self.remove_returned_legs(end, target)?;
        let mut fixed = self.capture_named_contact_except(end, from, reflected_routes)?;
        fixed.retain(|other| !matches!(other, WireEndpoint::Terminal(pin) if pin.symbol != own && moving.contains(&pin.symbol)));
        Ok(Some((end, from, fixed)))
    }

    /// Set a pose captured by [`Self::capture_pose`] and connect its retained contacts.
    fn place_pose(&mut self, index: usize, to: Point, rotation: u8, mirrored: bool, captured: Captured) -> Result<(), CommandError> {
        let symbol = &mut Arc::make_mut(&mut self.project.sheet_mut().symbols)[index];
        symbol.position = to;
        symbol.rotation = rotation;
        symbol.mirrored = mirrored;
        if let Some((end, from, fixed)) = captured {
            let at = self.project.endpoint_position(end).ok_or_else(|| CommandError::InvalidParams("missing named terminal".into()))?;
            let generated = self.retain_fixed_contacts(end, from, at, fixed)?;
            if !generated.is_empty() {
                Arc::make_mut(&mut self.project.sheet_mut().generated_legs).extend(generated);
            }
        }
        self.project.named_graph = Arc::default();
        self.attachments = self.project.wire_attachment_policy();
        Ok(())
    }

    /// Drop exact legs recorded by an earlier retained pose when the named terminal returns to
    /// a fixed contact's point. Authored routes and legs edited after creation are preserved.
    fn remove_returned_legs(&mut self, end: WireEndpoint, target: Point) -> Result<(), CommandError> {
        let legs = self.project.generated_legs.clone();
        let mut remove = Vec::new();
        let mut stale = Vec::new();
        for leg in legs.iter() {
            if leg.end != end {
                continue;
            }
            let Some(wire) = self.project.wires().iter().find(|wire| wire.ordinal() == leg.wire) else {
                stale.push(leg.wire);
                continue;
            };
            if !leg.matches(wire) {
                stale.push(leg.wire);
                continue;
            }
            if self.project.endpoint_position(leg.other) == Some(target) {
                remove.push(leg.wire);
            }
        }
        if remove.is_empty() && stale.is_empty() {
            return Ok(());
        }
        let sheet = self.project.sheet_mut();
        Arc::make_mut(&mut sheet.wires).retain(|wire| !remove.contains(&wire.ordinal()));
        Arc::make_mut(&mut sheet.generated_legs).retain(|leg| !remove.contains(&leg.wire) && !stale.contains(&leg.wire));
        self.touched.extend(remove);
        Ok(())
    }

    /// Retain all exact authored wire contacts of a globally naming terminal. Fixed pins stay
    /// put and receive a connecting leg; wire interiors split using the shared bounded allocator.
    pub(super) fn capture_named_contact(&mut self, end: WireEndpoint, from: Point) -> Result<BTreeSet<WireEndpoint>, CommandError> {
        self.capture_named_contact_except(end, from, &BTreeSet::new())
    }

    /// Capture stationary wire contacts. Routes reflecting together with the terminal keep
    /// their exact contact without a split or a generated leg.
    fn capture_named_contact_except(
        &mut self,
        end: WireEndpoint,
        from: Point,
        reflected_routes: &BTreeSet<u64>,
    ) -> Result<BTreeSet<WireEndpoint>, CommandError> {
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
            if reflected_routes.contains(&wire.ordinal())
                || wire.endpoints().contains(&end)
                || !super::super::path_contains_point(&self.project.wire_path(&wire), from)
            {
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
    /// Returns the exact legs it created so a later return can remove only those routes.
    pub(super) fn retain_fixed_contacts(
        &mut self,
        end: WireEndpoint,
        from: Point,
        to: Point,
        fixed: BTreeSet<WireEndpoint>,
    ) -> Result<Vec<crate::wire::GeneratedLeg>, CommandError> {
        let mut generated = Vec::new();
        for other in fixed {
            if !self.project.wires().iter().any(|wire| wire.endpoints().contains(&other) && wire.endpoints().contains(&end)) {
                let path = crate::schematic::orthogonal_path([from, to], true);
                self.draw([other, end], &path)?;
                let wire = self
                    .project
                    .wires()
                    .iter()
                    .find(|wire| wire.endpoints() == [other, end])
                    .ok_or_else(|| CommandError::Unavailable("generated contact leg is missing".into()))?;
                generated.push(crate::wire::GeneratedLeg { wire: wire.ordinal(), end, other, bends: wire.bends().map(|bends| bends.to_vec()) });
            }
        }
        self.touched.extend(self.project.wires().iter().filter(|w| w.endpoints().contains(&end)).map(|w| w.ordinal()));
        Ok(generated)
    }
}

impl Project {
    /// Set a current-sheet pose while preserving naming-terminal contacts. Existing explicit
    /// routes keep their endpoints and authored bends; no automatic route cleanup is performed.
    /// Invalid indices, coordinates, orientations or bounded contact allocations fail atomically.
    pub(crate) fn retained_symbol_pose(&self, index: usize, to: Point, rotation: u8, mirrored: bool, revision: u64) -> Result<Self, CommandError> {
        self.retained_symbol_poses(&[(index, to, rotation, mirrored)], revision, &BTreeSet::new())
    }

    /// Apply several current-sheet poses as one rigid batch with the same contact retention as
    /// [`Self::retained_symbol_pose`], cleaning up once after the last. Every contact is captured
    /// from the geometry before the first pose, so an earlier pose cannot move a pin into a later
    /// terminal's old contact point. Any invalid pose rejects all.
    pub(crate) fn retained_symbol_poses(
        &self,
        poses: &[(usize, Point, u8, bool)],
        revision: u64,
        reflected_routes: &BTreeSet<u64>,
    ) -> Result<Self, CommandError> {
        let mut edit = Edit::new(self, revision);
        let moving: BTreeSet<_> = poses.iter().filter_map(|&(index, ..)| self.symbols().get(index).map(|symbol| symbol.ordinal)).collect();
        let mut captured = Vec::with_capacity(poses.len());
        for &(index, to, rotation, mirrored) in poses {
            if !valid_point(&to) || rotation > 3 {
                return Err(CommandError::InvalidParams("invalid symbol pose".into()));
            }
            captured.push(edit.capture_pose(index, to, rotation, mirrored, &moving, reflected_routes)?);
        }
        for (&(index, to, rotation, mirrored), captured) in poses.iter().zip(captured) {
            edit.place_pose(index, to, rotation, mirrored, captured)?;
        }
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
        let from = self.endpoint_position(end).ok_or_else(|| CommandError::NotFound("unknown net label on this sheet".into()))?;
        if from == to {
            return Ok(preview);
        }
        let mut edit = Edit::new(self, revision);
        let fixed = edit.capture_named_contact(end, from)?;
        edit.project = edit.project.preview_net_label(id, to, rotation)?;
        let _ = edit.retain_fixed_contacts(end, from, to, fixed)?;
        if edit.touched.is_empty() { Ok(edit.project) } else { edit.finish(auto_heal) }
    }
}
