//! Validate bounded persisted identity authority against the complete current electrical graph.

use super::{
    Anchor, AnchorKind, MAX_PAIRS, MAX_RECORDS, MAX_REGISTRY_BYTES, RecordEncoding, Registry, Retirement, ScopeKind, State,
    graph::{Graph, GraphBuilder},
};
use crate::Project;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Zero-allocation byte counter used to meter compact JSON size against [`MAX_REGISTRY_BYTES`].
struct ByteCounter<'a>(&'a mut usize);

impl std::io::Write for ByteCounter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        *self.0 = self.0.saturating_add(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn history_ids(state: &State) -> Option<&[Uuid]> {
    match state {
        State::Active {} => None,
        State::Ambiguous { branches, .. } => Some(branches),
        State::Retired { successors, .. } => Some(successors),
    }
}

/// Incremental validator and compact byte meter for [`Registry`] so native Save encoding can
/// yield between frames without allocating an 8 MiB buffer. Per-item stages step one record
/// ID/number check, differential pair, sheet anchor item, network index entry, record header,
/// class binding, lineage item, seen-lineage cleanup entry, branch/successor UUID, split
/// baseline check or index cleanup entry, or [`RecordEncoding`] byte-metering fragment per call.
/// Three whole-project units remain single isolated steps that yield before and after: warming
/// `project.borrowed_electrical_connectivity()` and indexing every net's label/port endpoints
/// (`GraphBuilder` phase 0, building connectivity once per snapshot if not already cached),
/// sorting derived networks (`GraphBuilder` phase 3), and `numbering::plan` (phase 5).
#[derive(Default)]
pub(crate) struct RegistryValidation {
    phase: u8,
    cursor: usize,
    substep: u8,
    item_index: usize,
    ids: BTreeSet<Uuid>,
    prev_id: Option<Uuid>,
    id_error: bool,
    numbers: BTreeSet<u32>,
    number_error: bool,
    pair_members: BTreeSet<Uuid>,
    prev_pair: Option<Uuid>,
    graph_builder: GraphBuilder,
    graph: Option<Graph>,
    assigned: BTreeSet<usize>,
    seen_lineage: BTreeSet<(Uuid, Option<Uuid>, Anchor)>,
    has_authored_intent: bool,
    // For each source net: its most recent lineage owner and the number of distinct owners.
    split_counts: BTreeMap<Uuid, (Uuid, u32)>,
    prev_successor: Option<Uuid>,
    meter_bytes: usize,
    meter_encoding: RecordEncoding,
}

impl RegistryValidation {
    /// Current validation phase name for incremental-stepping and cancellation tests.
    #[cfg(test)]
    pub(crate) fn stage(&self) -> &'static str {
        match self.phase {
            0 => "envelope",
            1 => "scan_records",
            2 => "validate_pairs",
            3 => "build_graph",
            4 => "validate_records",
            5 => "numbering_plan",
            6 => "meter_records",
            7 => "meter_pairs",
            8 => "validate_split_baselines",
            9 => "drain_split_counts",
            _ => "done",
        }
    }

    fn finish_id_checks(&self) -> Result<(), String> {
        if self.id_error {
            return Err("Net records require unique non-nil UUIDs in canonical order.".into());
        }
        if self.number_error {
            return Err("Generated net numbers must be positive and unique, including historical records.".into());
        }
        Ok(())
    }

    /// Advance one validation or byte-metering step, returning `Ok(true)` when complete.
    pub(crate) fn step(&mut self, registry: &Registry, project: &Project) -> Result<bool, String> {
        match self.phase {
            0 => {
                if registry.records.len() > MAX_RECORDS || (!registry.records.is_empty() && !project.has_sheet_occurrences()) {
                    return Err("Net registry requires adopted occurrences and a bounded record count.".into());
                }
                if registry.pairs.len() > MAX_PAIRS {
                    return Err("Differential pair count exceeds its limit.".into());
                }
                if registry.records.is_empty() {
                    if !registry.pairs.is_empty() {
                        registry.validate_pairs()?;
                    }
                    *self = Self::default();
                    return Ok(true);
                }
                self.phase = 1;
            }
            1 => {
                if let Some(record) = registry.records.get(self.cursor) {
                    self.has_authored_intent |= !record.authored_intent.is_empty();
                    let unique = self.ids.insert(record.id);
                    if record.id.is_nil() || self.prev_id.is_some_and(|prev| prev >= record.id) || !unique {
                        self.id_error = true;
                    }
                    self.prev_id = Some(record.id);
                    if let Some(number) = record.generated_number
                        && (number == 0 || !self.numbers.insert(number))
                    {
                        self.number_error = true;
                    }
                    self.cursor += 1;
                } else {
                    self.cursor = 0;
                    self.numbers.clear();
                    if registry.pairs.is_empty() {
                        self.finish_id_checks()?;
                        self.phase = 3;
                    } else {
                        self.phase = 2;
                    }
                }
            }
            2 => {
                if let Some(pair) = registry.pairs.get(self.cursor) {
                    super::pairs::validate_pair(pair, &self.ids, &mut self.pair_members, &mut self.prev_pair)?;
                    self.cursor += 1;
                } else {
                    self.cursor = 0;
                    self.pair_members.clear();
                    self.finish_id_checks()?;
                    self.phase = 3;
                }
            }
            3 => {
                if let Some(graph) = self.graph_builder.step(project)? {
                    self.graph = Some(graph);
                    self.cursor = 0;
                    self.substep = 0;
                    self.item_index = 0;
                    self.phase = 4;
                }
            }
            4 => {
                let Some(record) = registry.records.get(self.cursor) else {
                    self.cursor = 0;
                    self.assigned.clear();
                    self.seen_lineage.clear();
                    self.phase = if self.has_authored_intent { 8 } else { 5 };
                    return Ok(false);
                };
                match self.substep {
                    0 => {
                        record.authored_intent.validate_reference(Some(record.id), |id| self.ids.contains(&id))?;
                        let graph = self.graph.as_ref().ok_or("missing electrical graph during net registry validation")?;
                        if record.fallbacks.len() > 3
                            || record.lineage.len() > MAX_RECORDS
                            || record.anchors().collect::<BTreeSet<_>>().len() != record.fallbacks.len() + 1
                        {
                            return Err("Duplicate or excessive net anchors/lineage.".into());
                        }
                        if record.class_bindings.len() > super::classes::MAX_BINDINGS {
                            return Err("Invalid, dangling, duplicate or excessive net class bindings.".into());
                        }
                        for anchor in record.anchors() {
                            anchor_owner(anchor, project, !matches!(record.state, State::Active {}))?;
                        }
                        match &record.state {
                            State::Active {} => {
                                let index = graph.network(&record.primary).ok_or("Missing active net primary.")?;
                                let network = graph.networks.get(index).ok_or("Missing active net primary.")?;
                                if !network.meaningful()
                                    || !self.assigned.insert(index)
                                    || record.fallbacks.iter().any(|a| graph.network(a) != Some(index))
                                {
                                    return Err("Active net anchors do not uniquely identify one meaningful network.".into());
                                }
                            }
                            State::Ambiguous { snapshot, branches } => {
                                if branches.is_empty() || branches.len() > MAX_RECORDS {
                                    return Err("Ambiguity branches require distinct current net identities.".into());
                                }
                                snapshot.validate()?;
                            }
                            State::Retired { snapshot, reason, successors } => {
                                if successors.len() > MAX_RECORDS || (*reason == Retirement::Removed) != successors.is_empty() {
                                    return Err("Invalid retired net disposition.".into());
                                }
                                snapshot.validate()?;
                            }
                        }
                        self.item_index = 0;
                        self.prev_successor = None;
                        if !record.class_bindings.is_empty() {
                            self.substep = 1;
                        } else if !record.lineage.is_empty() {
                            self.substep = 2;
                        } else if history_ids(&record.state).is_some_and(|ids| !ids.is_empty()) {
                            self.substep = 4;
                        } else {
                            self.cursor += 1;
                        }
                    }
                    1 => {
                        if let Some(binding) = record.class_bindings.get(self.item_index) {
                            let ordered =
                                self.item_index.checked_sub(1).and_then(|prev| record.class_bindings.get(prev)).is_none_or(|prev| prev < binding);
                            if !ordered || project.net_classes.get(binding.class).is_none() || !self.ids.contains(&binding.origin) {
                                return Err("Invalid, dangling, duplicate or excessive net class bindings.".into());
                            }
                            self.item_index += 1;
                        }
                        if self.item_index >= record.class_bindings.len() {
                            self.item_index = 0;
                            if !record.lineage.is_empty() {
                                self.substep = 2;
                            } else if history_ids(&record.state).is_some_and(|ids| !ids.is_empty()) {
                                self.substep = 4;
                            } else {
                                self.substep = 0;
                                self.cursor += 1;
                            }
                        }
                    }
                    2 => {
                        if let Some(source) = record.lineage.get(self.item_index) {
                            source.snapshot.validate()?;
                            anchor_owner(&source.anchor, project, true)?;
                            if source.net.is_some_and(|id| id == record.id || !self.ids.contains(&id))
                                || !self.seen_lineage.insert((source.snapshot.capture_id, source.net, source.anchor.clone()))
                            {
                                return Err("Invalid or duplicate snapshot-qualified net lineage.".into());
                            }
                            if self.has_authored_intent
                                && let Some(net) = source.net
                            {
                                let (last, count) = self.split_counts.entry(net).or_insert((record.id, 1));
                                if *last != record.id {
                                    *last = record.id;
                                    *count += 1;
                                }
                            }
                            self.item_index += 1;
                        }
                        if self.item_index >= record.lineage.len() {
                            self.item_index = 0;
                            self.substep = 3;
                        }
                    }
                    3 => {
                        let _ = self.seen_lineage.pop_first();
                        if self.seen_lineage.is_empty() {
                            self.item_index = 0;
                            self.prev_successor = None;
                            if history_ids(&record.state).is_some_and(|ids| !ids.is_empty()) {
                                self.substep = 4;
                            } else {
                                self.substep = 0;
                                self.cursor += 1;
                            }
                        }
                    }
                    4 => {
                        let ids = history_ids(&record.state).unwrap_or(&[]);
                        if let Some(&id) = ids.get(self.item_index) {
                            let ordered = !self.prev_successor.is_some_and(|prev| prev >= id);
                            match &record.state {
                                State::Ambiguous { .. } => {
                                    let active = registry
                                        .records
                                        .binary_search_by_key(&id, |r| r.id)
                                        .ok()
                                        .and_then(|pos| registry.records.get(pos))
                                        .is_some_and(|r| matches!(r.state, State::Active {}));
                                    if !ordered || id == record.id || !active {
                                        return Err("Ambiguity branches require distinct current net identities.".into());
                                    }
                                }
                                State::Retired { .. } => {
                                    if !ordered || id == record.id || !self.ids.contains(&id) {
                                        return Err("Invalid retired net disposition.".into());
                                    }
                                }
                                State::Active {} => {}
                            }
                            self.prev_successor = Some(id);
                            self.item_index += 1;
                        }
                        if self.item_index >= ids.len() {
                            self.item_index = 0;
                            self.prev_successor = None;
                            self.substep = 0;
                            self.cursor += 1;
                        }
                    }
                    _ => return Err("invalid net registry validation substep".into()),
                }
            }
            5 => {
                let graph = self.graph.take().ok_or("missing electrical graph during net numbering validation")?;
                self.ids.clear();
                super::numbering::plan(registry, &graph.networks)?;
                self.meter_bytes = b"{\"records\":[".len();
                self.cursor = 0;
                self.meter_encoding = RecordEncoding::default();
                self.phase = 6;
            }
            6 => {
                if let Some(record) = registry.records.get(self.cursor) {
                    if self.meter_encoding.at_start() && self.cursor > 0 {
                        self.meter_bytes = self.meter_bytes.saturating_add(1);
                    }
                    let done = self.meter_encoding.step(&mut ByteCounter(&mut self.meter_bytes), record)?;
                    if self.meter_bytes > MAX_REGISTRY_BYTES {
                        return Err("Net registry exceeds its byte limit.".into());
                    }
                    if done {
                        self.cursor += 1;
                    }
                } else if !registry.pairs.is_empty() {
                    self.meter_bytes = self.meter_bytes.saturating_add(b"],\"pairs\":[".len());
                    if self.meter_bytes > MAX_REGISTRY_BYTES {
                        return Err("Net registry exceeds its byte limit.".into());
                    }
                    self.cursor = 0;
                    self.phase = 7;
                } else {
                    self.meter_bytes = self.meter_bytes.saturating_add(b"]}".len());
                    if self.meter_bytes > MAX_REGISTRY_BYTES {
                        return Err("Net registry exceeds its byte limit.".into());
                    }
                    *self = Self::default();
                    return Ok(true);
                }
            }
            7 => {
                if let Some(pair) = registry.pairs.get(self.cursor) {
                    if self.cursor > 0 {
                        self.meter_bytes = self.meter_bytes.saturating_add(1);
                    }
                    serde_json::to_writer(&mut ByteCounter(&mut self.meter_bytes), pair.as_ref()).map_err(|e| e.to_string())?;
                    if self.meter_bytes > MAX_REGISTRY_BYTES {
                        return Err("Net registry exceeds its byte limit.".into());
                    }
                    self.cursor += 1;
                } else {
                    self.meter_bytes = self.meter_bytes.saturating_add(b"]}".len());
                    if self.meter_bytes > MAX_REGISTRY_BYTES {
                        return Err("Net registry exceeds its byte limit.".into());
                    }
                    *self = Self::default();
                    return Ok(true);
                }
            }
            8 => {
                if let Some(record) = registry.records.get(self.cursor) {
                    record
                        .authored_intent
                        .validate_split_baselines_with(Some(record.id), |net| self.split_counts.get(&net).map_or(0, |&(_, count)| count))?;
                    self.cursor += 1;
                } else {
                    self.phase = 9;
                }
            }
            9 => {
                if self.split_counts.pop_first().is_none() {
                    self.phase = 5;
                }
            }
            _ => return Err("invalid net registry validation phase".into()),
        }
        Ok(false)
    }
}

/// Live/storage anchors use the sole owner; historical lineage may additionally retain the
/// schematic scope preceding first-board creation. Unsupported system/junction keys are refused.
fn anchor_owner(anchor: &Anchor, project: &Project, historical: bool) -> Result<(), String> {
    anchor.validate_form()?;
    if anchor.project != project.id || anchor.kind == AnchorKind::Junction || anchor.scope.kind == ScopeKind::System {
        return Err("Foreign or unsupported registry anchor.".into());
    }
    if anchor.scope != project.net_scope() && !(historical && anchor.scope.kind == ScopeKind::Schematic && anchor.scope.id == project.id) {
        return Err("Registry anchor names a foreign electrical owner.".into());
    }
    Ok(())
}

impl Registry {
    /// Reject malformed records, historical references, duplicate live authorities and dangling
    /// choices before adoption. Inactive source anchors are historical, never live references.
    /// Empty registries need no graph; nonempty registries require adopted occurrences.
    pub(crate) fn validate(&self, project: &Project) -> Result<(), String> {
        let mut validation = RegistryValidation::default();
        while !validation.step(self, project)? {}
        Ok(())
    }

    /// Check the registry byte bound without rebuilding connectivity for a metadata-only edit.
    pub(super) fn validate_encoded_size(&self) -> Result<(), String> {
        let mut bytes = 0;
        serde_json::to_writer(ByteCounter(&mut bytes), self).map_err(|error| error.to_string())?;
        if bytes > MAX_REGISTRY_BYTES {
            return Err("Net registry exceeds its byte limit.".into());
        }
        Ok(())
    }
}

impl Project {
    /// Inspect persistent and revision-derived addresses without promotion, allocation or edits.
    /// Derived membership remains in the ordinary netlist; each row here supplies a typed key.
    /// Explicit differential relationships appear only when stored.
    pub fn inspect_net_registry(&self) -> Result<serde_json::Value, String> {
        let graph = Graph::build(self)?;
        let electrical = self.electrical_connectivity();
        let mut active_ids = vec![None; graph.networks.len()];
        for record in &self.net_registry.records {
            if matches!(record.state, State::Active {})
                && let Some(index) = graph.network(&record.primary)
            {
                active_ids[index].get_or_insert(record.id);
            }
        }
        let nets: Vec<_> = graph
            .networks
            .iter()
            .enumerate()
            .filter(|(_, n)| n.meaningful())
            .map(|(index, network)| {
                let id = active_ids[index];
                let display_name = network
                    .anchors
                    .iter()
                    .find_map(|anchor| electrical.net_index_for_endpoint(self.anchor_endpoint(anchor)?).map(|index| electrical.net_name(index)))
                    .unwrap_or_else(|| "Unresolved net".into());
                serde_json::json!({"id":id,"anchor":network.anchors.first(),"names":network.names,"name":display_name})
            })
            .collect();
        let mut result = serde_json::json!({"scope":self.net_scope(),"records":self.net_registry.records,"nets":nets});
        if self.net_registry.has_pairs() {
            result["pairs"] = serde_json::to_value(&self.net_registry.pairs).map_err(|error| error.to_string())?;
        }
        Ok(result)
    }

    /// Return the active persistent ID of each row of [`Project::electrical_connectivity`], by
    /// row index; rows without one are `None`. Reads only, like [`Self::inspect_net_registry`].
    pub(crate) fn persistent_net_ids(&self) -> Result<Vec<Option<uuid::Uuid>>, String> {
        let electrical = self.electrical_connectivity();
        let mut ids = vec![None; electrical.nets.len()];
        if self.net_registry.records.is_empty() {
            return Ok(ids);
        }
        let graph = Graph::build(self)?;
        for record in &self.net_registry.records {
            if !matches!(record.state, State::Active {}) {
                continue;
            }
            let Some(network) = graph.network(&record.primary) else { continue };
            let row = graph.networks[network].anchors.iter().find_map(|anchor| electrical.net_index_for_endpoint(self.anchor_endpoint(anchor)?));
            if let Some(slot) = row.and_then(|row| ids.get_mut(row)) {
                slot.get_or_insert(record.id);
            }
        }
        Ok(ids)
    }

    /// The current schematic endpoint of a live anchor; junction anchors have none.
    fn anchor_endpoint(&self, anchor: &Anchor) -> Option<crate::WireEndpoint> {
        match anchor.kind {
            AnchorKind::Terminal => self.component_terminal(anchor.object, &anchor.terminal).map(crate::WireEndpoint::Terminal),
            AnchorKind::Label => Some(crate::WireEndpoint::NetLabel(anchor.object)),
            AnchorKind::Port => Some(crate::WireEndpoint::Label(anchor.object)),
            AnchorKind::Junction => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Session,
        board::update::SourceCapture,
        net_classes,
        nets::{ClassBinding, Lineage, Pair, PairState},
    };
    use std::sync::Arc;

    /// Build a project with two promoted active nets so validation tests can mutate records,
    /// pairs, class bindings, and lineage without hand-building schematic fixtures.
    fn two_net_project() -> Project {
        let mut session = Session::default();
        session.apply("project.new", &serde_json::json!({})).unwrap();
        let mut pins = Vec::new();
        for index in 0..4 {
            let revision = session.revision();
            session
                .apply(crate::PLACE_RESISTOR, &serde_json::json!({"x_nm": index as i64 * 25_400_000, "y_nm": 0, "expected_revision": revision}))
                .unwrap();
            let id = session.project().unwrap().symbols().last().unwrap().id();
            pins.push(serde_json::json!({"symbol": id, "terminal": "1"}));
        }
        for (from, to) in [(&pins[0], &pins[1]), (&pins[2], &pins[3])] {
            let revision = session.revision();
            session.apply(crate::CONNECT_PINS, &serde_json::json!({"from": from, "to": to, "expected_revision": revision})).unwrap();
        }
        for pin in [&pins[0], &pins[2]] {
            let revision = session.revision();
            session.apply(crate::nets::PROMOTE, &serde_json::json!({"anchor": pin, "expected_revision": revision})).unwrap();
        }
        (*session.snapshot().unwrap()).clone()
    }

    /// Out-of-order records `[2, 1]` still populate the pair membership set so a valid pair
    /// `(1, 2)` passes pair membership checks and reports the UUID canonical-order error rather
    /// than a false missing-member error.
    #[test]
    fn out_of_order_records_with_valid_pair_report_uuid_order_error() {
        let mut project = two_net_project();
        let registry = Arc::make_mut(&mut project.net_registry);
        assert_eq!(registry.records.len(), 2);
        let first_id = registry.records[0].id;
        let second_id = registry.records[1].id;
        assert!(first_id < second_id);
        registry.records.swap(0, 1);

        let mut pair_id = Uuid::from_u128(1);
        while pair_id == first_id || pair_id == second_id {
            pair_id = Uuid::from_u128(pair_id.as_u128() + 1);
        }
        registry.pairs = Arc::new(vec![Arc::new(Pair {
            id: pair_id,
            positive: first_id,
            negative: second_id,
            name: "DIFF_0".into(),
            state: PairState::Active {},
        })]);

        let error = project.net_registry.validate(&project).unwrap_err();
        assert_eq!(error, "Net records require unique non-nil UUIDs in canonical order.");
    }

    /// Large `class_bindings` lists validate one binding per [`RegistryValidation::step`] before
    /// lineage/state substeps and reject out-of-order, dangling-class, and dangling-origin entries.
    #[test]
    fn class_bindings_validate_one_item_per_step_and_reject_invalid_entries() {
        const BINDING_COUNT: usize = 64;
        let mut project = two_net_project();
        let first_id = project.net_registry.records[0].id;
        let second_id = project.net_registry.records[1].id;

        let catalog = Arc::make_mut(&mut project.net_classes);
        catalog.definitions = (0..BINDING_COUNT)
            .map(|index| net_classes::Definition {
                id: Uuid::from_u128(1000 + index as u128),
                revision: 1,
                name: format!("Class{index}"),
                rules: Default::default(),
                authored_intent: Default::default(),
            })
            .collect();

        let bindings: Vec<_> =
            (0..BINDING_COUNT).map(|index| ClassBinding { class: Uuid::from_u128(1000 + index as u128), origin: first_id }).collect();
        Arc::make_mut(&mut project.net_registry).records[0].class_bindings = bindings;

        let mut validation = RegistryValidation::default();
        while !(validation.phase == 4 && validation.cursor == 0 && validation.substep == 1) {
            assert!(!validation.step(&project.net_registry, &project).unwrap());
        }
        assert_eq!(validation.item_index, 0);
        for expected_index in 1..=BINDING_COUNT {
            assert!(!validation.step(&project.net_registry, &project).unwrap());
            if expected_index < BINDING_COUNT {
                assert_eq!((validation.phase, validation.cursor, validation.substep, validation.item_index), (4, 0, 1, expected_index));
            } else {
                assert_eq!((validation.phase, validation.cursor, validation.substep, validation.item_index), (4, 1, 0, 0));
            }
        }
        while !validation.step(&project.net_registry, &project).unwrap() {}

        let mut out_of_order = project.clone();
        Arc::make_mut(&mut out_of_order.net_registry).records[0].class_bindings.swap(10, 11);
        assert_eq!(out_of_order.net_registry.validate(&out_of_order).unwrap_err(), "Invalid, dangling, duplicate or excessive net class bindings.");

        let mut dangling_class = project.clone();
        Arc::make_mut(&mut dangling_class.net_registry).records[0].class_bindings[BINDING_COUNT - 1].class = Uuid::from_u128(999_999);
        assert_eq!(
            dangling_class.net_registry.validate(&dangling_class).unwrap_err(),
            "Invalid, dangling, duplicate or excessive net class bindings."
        );

        let mut missing_origin = Uuid::from_u128(2);
        while missing_origin == first_id || missing_origin == second_id {
            missing_origin = Uuid::from_u128(missing_origin.as_u128() + 1);
        }
        let mut dangling_origin = project.clone();
        Arc::make_mut(&mut dangling_origin.net_registry).records[0].class_bindings[BINDING_COUNT - 1].origin = missing_origin;
        assert_eq!(
            dangling_origin.net_registry.validate(&dangling_origin).unwrap_err(),
            "Invalid, dangling, duplicate or excessive net class bindings."
        );
    }

    /// After validating a record's lineage items, [`RegistryValidation::step`] drains
    /// `seen_lineage` one entry per step rather than clearing the whole set on the last lineage
    /// item.
    #[test]
    fn seen_lineage_drains_one_entry_per_step_after_lineage_validation() {
        const LINEAGE_COUNT: usize = 64;
        let mut project = two_net_project();
        let registry = Arc::make_mut(&mut project.net_registry);
        let second_id = registry.records[1].id;
        let anchor = registry.records[0].primary.clone();
        registry.records[0].lineage = (0..LINEAGE_COUNT)
            .map(|index| Lineage {
                snapshot: SourceCapture { capture_id: Uuid::from_u128(100 + index as u128), content_digest: "a".repeat(64) },
                net: (index % 2 == 0).then_some(second_id),
                anchor: anchor.clone(),
            })
            .collect();

        let mut validation = RegistryValidation::default();
        while !(validation.phase == 4 && validation.cursor == 0 && validation.substep == 2) {
            assert!(!validation.step(&project.net_registry, &project).unwrap());
        }
        assert!(validation.seen_lineage.is_empty());
        for expected_len in 1..=LINEAGE_COUNT {
            assert!(!validation.step(&project.net_registry, &project).unwrap());
            assert_eq!(validation.seen_lineage.len(), expected_len);
        }
        assert_eq!((validation.phase, validation.cursor, validation.substep), (4, 0, 3));
        assert_eq!(validation.seen_lineage.len(), LINEAGE_COUNT);

        for remaining in (0..LINEAGE_COUNT).rev() {
            assert!(!validation.step(&project.net_registry, &project).unwrap());
            assert_eq!(validation.seen_lineage.len(), remaining);
            if remaining > 0 {
                assert_eq!((validation.phase, validation.cursor, validation.substep), (4, 0, 3));
            } else {
                assert_eq!((validation.phase, validation.cursor, validation.substep), (4, 1, 0));
            }
        }
        while !validation.step(&project.net_registry, &project).unwrap() {}
    }
}
