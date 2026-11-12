//! Closed sheet-tree capture and composition through the existing flat fragment remapper.
use super::*;
use crate::{SheetTarget, hierarchy as model, sheets::SchematicSheet};
use uuid::Uuid;

/// Parse canonical root hierarchy IDs without silently coalescing duplicates or accepting
/// another object's prefix. Omission preserves the original flat clipboard command contract.
pub(crate) fn hierarchy_objects(value: Option<&Value>) -> Option<BTreeSet<String>> {
    let Some(value) = value else { return Some(BTreeSet::new()) };
    let array = value.as_array()?;
    if array.len() > 128 {
        return None;
    }
    let mut objects = BTreeSet::new();
    for object in array {
        let object = object.as_str()?;
        if !["sheet-symbol-", "label-"].into_iter().any(|prefix| crate::schematic::wiring::parse_id(object, prefix).is_some())
            || !objects.insert(object.to_owned())
        {
            return None;
        }
    }
    Some(objects)
}

/// Follow only the selected block roots. Unrelated root documents and siblings never enter a
/// clipboard; the native single-parent/cycle rules remain authoritative.
fn descendants(project: &Project, blocks: &[model::SheetSymbol]) -> Result<BTreeSet<Uuid>, CommandError> {
    let mut todo: Vec<_> = blocks.iter().map(|b| b.child).collect();
    let mut included = BTreeSet::new();
    while let Some(id) = todo.pop() {
        if !included.insert(id) {
            return Err(invalid("copied sheet subtree repeats a child identity"));
        }
        let sheet = project.sheets.iter().find(|s| s.sheet_id == id).ok_or_else(|| invalid("copied child sheet is unavailable"))?;
        todo.extend(sheet.subsheets.iter().map(|b| b.child));
    }
    Ok(included)
}

/// Resolve root object ownership separately from complete descendant contents.
fn selected(project: &Project, objects: &BTreeSet<String>) -> Result<(Vec<model::SheetSymbol>, Vec<model::Label>), CommandError> {
    let blocks: Vec<_> = project.subsheets.iter().filter(|s| objects.contains(&format!("sheet-symbol-{}", s.id))).cloned().collect();
    let labels: Vec<_> = project.labels.iter().filter(|s| objects.contains(&format!("label-{}", s.id))).cloned().collect();
    if blocks.len() + labels.len() != objects.len() {
        return Err(invalid("hierarchy selection must belong to the explicitly addressed sheet"));
    }
    Ok((blocks, labels))
}

impl Fragment {
    /// Whether this fragment owns child sheets, sheet symbols or hierarchical interface labels.
    /// Flat board-circuit fragments remain deliberately outside this extended contract.
    pub fn has_hierarchy(&self) -> bool {
        self.project.sheets.len() != 1 || !self.project.subsheets.is_empty() || !self.project.labels.is_empty()
    }
    /// Root sheet symbols, root interface labels and included child documents for presentation.
    pub fn hierarchy_counts(&self) -> (usize, usize, usize) {
        (self.project.subsheets.len(), self.project.labels.len(), self.project.sheets.len().saturating_sub(1))
    }
    /// Suggest relocation only for a Cut tree from this saved project. Actual existence, parent
    /// ownership and cycle checks are performed again by placement, never authorized by this flag.
    pub fn offers_sheet_reuse(&self, project: &Project) -> bool {
        self.cut_sheets && self.project.id() == project.id() && !self.project.subsheets.is_empty()
    }
    /// Capture root objects plus complete selected child subtrees, retaining explicit interface
    /// endpoint identities. Unselected root endpoint owners become shared free endpoints.
    /// Root and descendant occurrence IDs remain attached to the retained source capture.
    pub fn capture_with_hierarchy(project: &Project, items: &Selection, objects: &BTreeSet<String>, cut: bool) -> Result<Self, CommandError> {
        if objects.is_empty() {
            return Self::capture(project, items);
        }
        let mut found = items.clone();
        found.retain(project);
        if &found != items {
            return Err(invalid("copy needs existing selected objects"));
        }
        let (blocks, labels) = selected(project, objects)?;
        let included = descendants(project, &blocks)?;
        let mut root = SchematicSheet::empty(project.sheet_target().id(), project.sheet_name().into());
        root.occurrence_id = project.sheet_occurrence(project.sheet_target()).map(|record| record.id);
        root.symbols = Arc::new(project.symbols().iter().filter(|s| items.symbols.contains(&s.id())).cloned().collect());
        root.net_labels = Arc::new(project.net_labels().iter().filter(|l| items.net_labels.contains(&l.object_id())).cloned().collect());
        root.subsheets = Arc::new(blocks);
        root.labels = Arc::new(labels);
        let symbols: BTreeSet<_> = root.symbols.iter().map(|s| s.ordinal).collect();
        let net_labels: BTreeSet<_> = root.net_labels.iter().map(|l| l.id).collect();
        let labels: BTreeSet<_> = root.labels.iter().map(|l| l.id).collect();
        let pins: BTreeSet<_> = root.subsheets.iter().flat_map(|b| project.sheet_pins(b).map(|p| p.label.id)).collect();
        let mut cursor = project.allocation_high_water().checked_add(1).ok_or_else(|| invalid("clipboard exhausts IDs"))?;
        let mut detached = BTreeMap::new();
        for source in project.wires().iter().filter(|w| items.wires.contains(&w.id())) {
            let mut copy = source.clone();
            let path = project.wire_path(source);
            copy.bends = Some(path[1..path.len() - 1].to_vec());
            for endpoint in &mut copy.endpoints {
                let keep = match endpoint {
                    WireEndpoint::Terminal(pin) => symbols.contains(&pin.symbol),
                    WireEndpoint::NetLabel(id) => net_labels.contains(id),
                    WireEndpoint::Label(id) => labels.contains(id),
                    WireEndpoint::SheetPin(id) => pins.contains(id),
                    _ => false,
                };
                if !keep {
                    let original = *endpoint;
                    let id = if let Some(id) = detached.get(&original) {
                        *id
                    } else {
                        let id = allocate(&mut cursor)?;
                        Arc::make_mut(&mut root.junctions)
                            .insert(id, project.endpoint_position(original).ok_or_else(|| invalid("missing copied endpoint"))?);
                        detached.insert(original, id);
                        id
                    };
                    *endpoint = WireEndpoint::Junction(id);
                }
            }
            Arc::make_mut(&mut root.wires).push(copy);
        }
        let cut_sheets = cut && !root.subsheets.is_empty();
        let mut sheets = vec![root];
        sheets.extend(project.sheets.iter().filter(|s| included.contains(&s.sheet_id)).cloned());
        let fragment = Self::captured(project, sheets, cursor, cut_sheets)?;
        fragment.validate_scope(true)?;
        Ok(fragment)
    }
    /// Reject board owners, orphan payload sheets/components, legacy extended payloads and empty
    /// roots. The native decoder has already checked geometry, libraries and hierarchy topology.
    pub(super) fn validate_scope(&self, extended: bool) -> Result<(), CommandError> {
        let p = &self.project;
        if p.board_target().is_some()
            || self.has_hierarchy() != extended
            || p.symbols().is_empty() && p.wires().is_empty() && p.net_labels().is_empty() && p.labels.is_empty() && p.subsheets.is_empty()
            || self.cut_sheets && p.subsheets.is_empty()
        {
            return Err(invalid("clipboard must contain a nonempty schematic fragment of the declared version"));
        }
        let included = descendants(p, &p.subsheets)?;
        let all: BTreeSet<_> = p.sheets.iter().filter(|s| s.sheet_id != p.sheet_target().id()).map(|s| s.sheet_id).collect();
        let components: BTreeSet<_> = p.sheets.iter().flat_map(|s| s.symbols.iter().map(|s| s.component)).collect();
        if included != all || components != p.components.keys().copied().collect() {
            return Err(invalid("clipboard contains objects outside its selected sheet subtree"));
        }
        Ok(())
    }
    /// Compose a new independent subtree, or relocate eligible original unparented children after
    /// Cut. Flat contents reuse the established identity/reference/package remapper; hierarchy
    /// endpoints are rebound by wire order before validation, retaining exact shared connectivity.
    /// Duplicated sheets receive new occurrence IDs in the destination use; Cut reuse keeps IDs.
    pub(super) fn placed_hierarchy(
        &self,
        destination: &Project,
        at: Point,
        first_id: u64,
        options: &PasteOptions,
    ) -> Result<(Project, Selection), CommandError> {
        if options.net_name_suffix.len() > 200 || options.net_name_suffix.chars().any(char::is_control) {
            return Err(invalid("invalid net-name suffix"));
        }
        if self.has_hierarchy() && !options.quarter_turns.is_multiple_of(4) {
            return Err(invalid("hierarchical sheet symbols and interfaces retain their orientation; paste without group rotation"));
        }
        if options.reuse_sheets && (!self.offers_sheet_reuse(destination) || !options.net_name_suffix.is_empty()) {
            return Err(invalid("sheet relocation needs an original-project Cut and cannot rename its current nets"));
        }
        // Root identity comes from the versioned envelope, independently of native display order.
        let source_root = self.project.sheet_target().id();
        let sources: Vec<_> = std::iter::once(&self.project.sheets[self.project.sheet_index])
            .chain(self.project.sheets.iter().filter(|s| s.sheet_id != source_root))
            .collect();
        let mut next = destination.clone();
        let root = destination.sheet_target();
        let mut sheet_map = BTreeMap::from([(self.project.sheet_target().id(), root.id())]);
        let descendants = descendants(&self.project, &self.project.subsheets)?;
        let mut labels = BTreeMap::new();
        let mut cursor = first_id.max(destination.allocation_high_water().saturating_add(1));
        if options.reuse_sheets {
            for block in self.project.subsheets.iter() {
                let target = SheetTarget { project: destination.id(), sheet: block.child };
                if destination.for_sheet(target).is_none() || destination.parent_sheet(target).is_some() {
                    return Err(invalid("original child sheets must still exist and be unparented; choose duplication instead"));
                }
                let current = destination.for_sheet(target).ok_or_else(|| invalid("source child unavailable"))?;
                for label in current.labels.iter() {
                    labels.insert(label.id, label.id);
                }
            }
        } else {
            if next.sheets.len() + descendants.len() > crate::sheets::MAX_SHEETS {
                return Err(invalid("pasted subtree exceeds the 64-sheet limit"));
            }
            let mut names: BTreeSet<_> = next.sheets.iter().map(|s| s.sheet_name.to_lowercase()).collect();
            for source in sources.iter().skip(1) {
                let id = crate::fresh_id()?;
                sheet_map.insert(source.sheet_id, id);
                let sheet = next.new_sheet_record(id, unique_sheet_name(&source.sheet_name, &mut names)?)?;
                Arc::make_mut(&mut next.sheets).push(sheet);
            }
        }
        // Allocate interfaces and blocks before importing wires, so every pin address is known.
        for (index, source) in sources.iter().enumerate() {
            if index != 0 && options.reuse_sheets {
                continue;
            }
            let target = sheet_map[&source.sheet_id];
            let dest_index = next.sheets.iter().position(|s| s.sheet_id == target).ok_or_else(|| invalid("destination sheet unavailable"))?;
            for source in source.labels.iter() {
                let mut copy = source.clone();
                copy.id = allocate(&mut cursor)?;
                labels.insert(source.id, copy.id);
                if index == 0 {
                    copy.at = transformed(source.at, self.anchor, at, options.quarter_turns)?;
                    let occupied = &next.sheets[dest_index].labels;
                    copy.slot = (0..model::MAX_LABELS as u8)
                        .find(|slot| !occupied.iter().any(|l| l.direction.right() == copy.direction.right() && l.slot == *slot))
                        .ok_or_else(|| invalid("no free parent pin slot"))?;
                }
                copy.name.push_str(&options.net_name_suffix);
                Arc::make_mut(&mut Arc::make_mut(&mut next.sheets)[dest_index].labels).push(copy);
            }
            for source in source.subsheets.iter() {
                let mut copy = source.clone();
                copy.id = allocate(&mut cursor)?;
                copy.child = if options.reuse_sheets { source.child } else { sheet_map[&source.child] };
                if index == 0 {
                    copy.at = transformed(source.at, self.anchor, at, 0)?;
                }
                Arc::make_mut(&mut Arc::make_mut(&mut next.sheets)[dest_index].subsheets).push(copy);
            }
        }
        next.numeric_id_high_water = Some(next.allocation_high_water().max(cursor.saturating_sub(1)));
        let mut shared_parts = BTreeMap::new();
        if options.reuse_sheets {
            for source in &sources {
                for symbol in source.symbols.iter() {
                    if destination.components.contains_key(&symbol.component) {
                        shared_parts.insert(symbol.component, symbol.component);
                    }
                }
            }
        }
        let mut selected = Selection::default();
        for (index, source) in sources.iter().enumerate() {
            if index != 0 && options.reuse_sheets {
                continue;
            }
            let source_target = SheetTarget { project: self.project.id(), sheet: source.sheet_id };
            let projection = self.project.for_sheet(source_target).ok_or_else(|| invalid("source sheet unavailable"))?;
            let items = Selection {
                symbols: projection.symbols().iter().map(|s| s.id()).collect(),
                wires: projection.wires().iter().map(|w| w.id()).collect(),
                net_labels: projection.net_labels().iter().map(|l| l.object_id()).collect(),
            };
            if items.is_empty() {
                continue;
            }
            let flat = Self::capture(&projection, &items)?;
            let target = SheetTarget { project: next.id(), sheet: sheet_map[&source.sheet_id] };
            let destination = next.for_sheet(target).ok_or_else(|| invalid("destination sheet unavailable"))?;
            let before = destination.wires().len();
            let mut policy = options.clone();
            policy.reuse_sheets = false;
            if index != 0 {
                policy.quarter_turns = 0;
            }
            let position = if index == 0 { transformed(flat.anchor(), self.anchor, at, options.quarter_turns)? } else { flat.anchor() };
            let (mut candidate, added) = flat.placed_with_parts(&destination, position, cursor, &policy, &mut shared_parts)?;
            for (offset, source) in source.wires.iter().enumerate() {
                for (end, original) in source.endpoints.iter().enumerate() {
                    let replacement = match original {
                        WireEndpoint::Label(id) => {
                            Some(WireEndpoint::Label(*labels.get(id).ok_or_else(|| invalid("copied interface is no longer available"))?))
                        }
                        WireEndpoint::SheetPin(id) => {
                            Some(WireEndpoint::SheetPin(*labels.get(id).ok_or_else(|| invalid("copied parent pin is no longer available"))?))
                        }
                        _ => None,
                    };
                    if let Some(replacement) = replacement {
                        Arc::make_mut(&mut candidate.sheet_mut().wires)[before + offset].endpoints[end] = replacement;
                    }
                }
            }
            candidate.prune_junctions();
            cursor = candidate.allocation_high_water().checked_add(1).ok_or_else(|| invalid("clipboard exhausts IDs"))?;
            if index == 0 {
                selected = added;
            }
            next = candidate;
        }
        next = next.for_sheet(root).ok_or_else(|| invalid("root destination unavailable"))?;
        next.named_graph = Arc::default();
        for sheet in Arc::make_mut(&mut next.sheets) {
            sheet.connectivity = Arc::default();
        }
        next.validate_hierarchy().map_err(invalid)?;
        validate_candidate(&next)?;
        Ok((next, selected))
    }
}

/// Retain source names when free, otherwise allocate an independent bounded copy name. Names
/// are presentation only; UUID mapping above is the sole source of ownership identity.
fn unique_sheet_name(source: &str, used: &mut BTreeSet<String>) -> Result<String, CommandError> {
    if used.insert(source.to_lowercase()) {
        return Ok(source.into());
    }
    for number in 1..=crate::sheets::MAX_SHEETS {
        let suffix = format!(" copy {number}");
        let limit = 200_usize.saturating_sub(suffix.chars().count());
        let name = format!("{}{suffix}", source.chars().take(limit).collect::<String>());
        if used.insert(name.to_lowercase()) {
            return Ok(name);
        }
    }
    Err(invalid("no unused child sheet name is available"))
}

impl Session {
    /// Cut root hierarchy objects while retaining unselected routes on both sides of interfaces.
    /// Child sheets become roots; their live content and board acceptance are retained for reuse.
    pub(super) fn cut_schematic_hierarchy(&mut self, params: &Value, objects: &BTreeSet<String>) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let items = Selection::parse(&params["items"]).ok_or_else(|| invalid("invalid selection"))?;
        let project = self.open_project()?;
        Fragment::capture_with_hierarchy(project, &items, objects, true)?;
        let (blocks, labels) = selected(project, objects)?;
        let label_ids: BTreeSet<_> = labels.iter().map(|l| l.id).collect();
        let pins: BTreeSet<_> = blocks.iter().flat_map(|b| project.sheet_pins(b).map(|p| p.label.id)).collect();
        let symbols: BTreeSet<_> = project.symbols().iter().filter(|s| items.symbols.contains(&s.id())).map(|s| s.ordinal).collect();
        let root = project.sheet_index;
        let mut next = project.clone();
        let mut cursor = self.next_object_id()?;
        for index in 0..project.sheets.len() {
            let mut projection = project.clone();
            projection.sheet_index = index;
            let mut detached = BTreeMap::new();
            let sheet = &mut Arc::make_mut(&mut next.sheets)[index];
            Arc::make_mut(&mut sheet.wires).retain(|w| index != root || !items.wires.contains(&w.id()));
            for wire in Arc::make_mut(&mut sheet.wires) {
                for endpoint in &mut wire.endpoints {
                    let remove = match *endpoint {
                        WireEndpoint::Terminal(pin) => symbols.contains(&pin.symbol),
                        WireEndpoint::NetLabel(id) => items.net_labels.contains(&format!("net-label-{id}")),
                        WireEndpoint::Label(id) => label_ids.contains(&id),
                        WireEndpoint::SheetPin(id) => label_ids.contains(&id) || pins.contains(&id),
                        _ => false,
                    };
                    if remove {
                        let original = *endpoint;
                        let id = if let Some(id) = detached.get(&original) {
                            *id
                        } else {
                            let id = allocate(&mut cursor)?;
                            detached.insert(original, id);
                            id
                        };
                        *endpoint = WireEndpoint::Junction(id);
                    }
                }
            }
            for (original, id) in detached {
                Arc::make_mut(&mut sheet.junctions)
                    .insert(id, projection.endpoint_position(original).ok_or_else(|| invalid("missing cut interface position"))?);
            }
            sheet.connectivity = Arc::default();
        }
        let sheet = &mut Arc::make_mut(&mut next.sheets)[root];
        Arc::make_mut(&mut sheet.symbols).retain(|s| !items.symbols.contains(&s.id()));
        Arc::make_mut(&mut sheet.net_labels).retain(|l| !items.net_labels.contains(&l.object_id()));
        Arc::make_mut(&mut sheet.labels).retain(|l| !label_ids.contains(&l.id));
        let block_ids: BTreeSet<_> = blocks.iter().map(|b| b.id).collect();
        Arc::make_mut(&mut sheet.subsheets).retain(|b| !block_ids.contains(&b.id));
        Arc::make_mut(&mut next.units).retain(|_, id| !symbols.contains(id));
        for component in project.symbols().iter().filter(|s| symbols.contains(&s.ordinal)).map(|s| s.component) {
            if !next.all_symbols().any(|symbol| symbol.component == component) {
                next.retire_component(component);
            }
        }
        for index in 0..next.sheets.len() {
            next.sheet_index = index;
            next.prune_junctions();
        }
        next.sheet_index = root;
        next.named_graph = Arc::default();
        next.numeric_id_high_water = Some(next.allocation_high_water().max(cursor.saturating_sub(1)));
        validate_candidate(&next)?;
        self.change(next)?;
        Ok(())
    }
}
