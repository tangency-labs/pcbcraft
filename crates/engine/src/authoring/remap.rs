//! Explicit terminal renumbering and read-only current-project consequences. No placed copy is edited.
use super::{Action, Item, Kind, Primitive, Session};
use crate::{Project, library::Behavior, schematic::PinNumber};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
mod budget;
#[cfg(test)]
mod tests;
mod wire;
pub use budget::{MAX_PREVIEW_BYTES, MAX_PREVIEW_GRAPH_TEXT_BYTES, MAX_PREVIEW_GRAPH_WORK};
pub use wire::Scalar;

/// An explicit correspondence from one stable pin primitive to a new electrical terminal number.
/// The unit names the owning drawing. Primitive IDs belong to this authoring document;
/// opening a published revision reconstructs them rather than preserving them across publication.
/// Remap transport represents the positive u64 object ID as a canonical decimal string.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinRemap {
    pub unit: String,
    #[serde(deserialize_with = "wire::primitive_id", serialize_with = "wire::serialize_id")]
    pub object: u64,
    pub number: String,
}

/// One logical terminal change; other appearances of the same terminal follow this correspondence.
#[derive(Clone, Debug, Serialize)]
pub struct Change {
    pub unit: String,
    #[serde(serialize_with = "wire::serialize_id")]
    pub object: u64,
    pub before: String,
    pub after: String,
    pub name: Option<String>,
}

/// Reject unbounded or malformed transport before looking up primitives. Semantic collisions are
/// checked against the complete item, including every inactive unit, by the transaction itself.
pub fn decode(value: &Value) -> Result<Vec<PinRemap>, String> {
    let entries = value.as_array().ok_or("Pin remap must be an array.")?;
    if entries.is_empty() || entries.len() > 128 {
        return Err("Choose 1 to 128 logical pins to remap.".into());
    }
    let pins = entries
        .iter()
        .map(|entry| {
            let row = entry.as_object().ok_or("Each pin remap row must be an object.")?;
            if row.len() != 3 || row.keys().any(|key| !matches!(key.as_str(), "unit" | "object" | "number")) {
                return Err("Each pin remap row requires only unit, object and number.".into());
            }
            let unit =
                row.get("unit").and_then(Value::as_str).filter(|unit| crate::board::definition::valid_id(unit)).ok_or("Invalid pin remap unit.")?;
            let object = Scalar::Object.parse(row.get("object").unwrap_or(&Value::Null))?;
            let number = row
                .get("number")
                .and_then(Value::as_str)
                .filter(|number| PinNumber::new(number).is_some())
                .ok_or("Invalid destination terminal number.")?;
            Ok(PinRemap { unit: unit.into(), object, number: number.into() })
        })
        .collect::<Result<Vec<_>, String>>()?;
    validate_entries(&pins)?;
    Ok(pins)
}

/// Structural schema shared by both catalogs; exact repeated rows and unknown fields are refused.
/// Distinct rows reusing an identity or destination need shared validation; resolved logical
/// sources and unchanged-terminal collisions need the authoring transaction. Pinned terminal
/// existence is report evidence: an absent counterpart is reported, not an authored-edit refusal.
pub fn schema() -> Value {
    json!({"type":"array","minItems":1,"maxItems":128,"uniqueItems":true,"items":{"type":"object","additionalProperties":false,"required":["unit","object","number"],"properties":{"unit":{"type":"string","minLength":1,"maxLength":64,"pattern":"^[a-z0-9]([a-z0-9-]{0,62}[a-z0-9])?$","not":{"pattern":"[^a-z0-9-]"}},"object":Scalar::Object.schema(),"number":{"type":"string","minLength":1,"maxLength":8,"pattern":"^[A-Za-z0-9+_\\-]+$","not":{"pattern":"[^A-Za-z0-9+_\\-]"}}}}})
}

/// Shared spelling and row-uniqueness validation for typed and transported callers.
fn validate_entries(pins: &[PinRemap]) -> Result<(), String> {
    if pins.is_empty() || pins.len() > 128 {
        return Err("Choose 1 to 128 logical pins to remap.".into());
    }
    let mut objects = BTreeSet::new();
    let mut numbers = BTreeSet::new();
    for pin in pins {
        if pin.object == 0 || !crate::board::definition::valid_id(&pin.unit) || !objects.insert((&pin.unit, pin.object)) {
            return Err("Unknown, duplicate or invalid pin identity.".into());
        }
        if PinNumber::new(&pin.number).is_none() || !numbers.insert(&pin.number) {
            return Err("Invalid or duplicate destination terminal number.".into());
        }
    }
    Ok(())
}

/// Resolve explicit identities, then validate the simultaneous map against unchanged terminals.
/// Neither pin positions nor display names ever establish correspondence.
fn changes(item: &Item, pins: &[PinRemap]) -> Result<Vec<Change>, String> {
    if item.kind != Kind::Symbol {
        return Err("Pin remap requires a symbol document.".into());
    }
    validate_entries(pins)?;
    let mut sources = BTreeSet::new();
    let mut changes = Vec::new();
    for pin in pins {
        let objects = if pin.unit == item.unit { Some(&item.objects) } else { item.other_units.get(&pin.unit) };
        let Some(Primitive::Pin { number, name, .. }) = objects.and_then(|objects| objects.get(&pin.object)) else {
            return Err("Unknown pin primitive or unit.".into());
        };
        if !sources.insert(number.clone()) {
            return Err("Choose each shared logical terminal only once.".into());
        }
        changes.push(Change { unit: pin.unit.clone(), object: pin.object, before: number.clone(), after: pin.number.clone(), name: name.clone() });
    }
    let map: BTreeMap<_, _> = changes.iter().map(|change| (change.before.clone(), change.after.clone())).collect();
    let all: BTreeSet<_> = item
        .objects
        .values()
        .chain(item.other_units.values().flat_map(|objects| objects.values()))
        .filter_map(|object| if let Primitive::Pin { number, .. } = object { Some(number) } else { None })
        .collect();
    let mut destinations = BTreeSet::new();
    for number in all {
        if !destinations.insert(map.get(number).unwrap_or(number)) {
            return Err("Destination collides with an unchanged terminal; include it in the simultaneous remap.".into());
        }
    }
    Ok(changes)
}

/// Apply the already explicit map to every appearance and power-role terminal on a private candidate.
/// Session validation and history publication remain the sole authored transaction authority.
pub(super) fn apply(item: &mut Item, pins: &[PinRemap]) -> Result<(), String> {
    let changes = changes(item, pins)?;
    let map: BTreeMap<_, _> = changes.iter().map(|change| (change.before.clone(), change.after.clone())).collect();
    for primitive in item.objects.values_mut().chain(item.other_units.values_mut().flat_map(|objects| objects.values_mut())) {
        if let Primitive::Pin { number, .. } = primitive
            && let Some(next) = map.get(&*number)
        {
            *number = next.clone();
        }
    }
    if let Some(Behavior::Power { terminal }) = &mut item.behavior
        && let Some(next) = map.get(&*terminal)
    {
        *terminal = next.clone();
    }
    Ok(())
}

/// A transient review budget, independent of the saved project/terminal limits. Refuse before
/// connectivity or report preparation rather than retaining a partial review that Apply could use.
pub const MAX_PREVIEW_ROWS: usize = 4096;

/// Count the expanded part × terminal work before graph/report/candidate allocation.
fn check_budget(parts: usize, pins: usize) -> Result<(), String> {
    if parts.checked_mul(pins).is_none_or(|rows| rows > MAX_PREVIEW_ROWS) {
        return Err(format!("Pin remap preview exceeds {MAX_PREVIEW_ROWS} affected rows; review fewer logical pins at once."));
    }
    Ok(())
}

/// Borrow evidence until its encoded row fits the remaining common report budget.
#[derive(Serialize)]
struct Evidence<'a> {
    #[serde(flatten)]
    change: &'a Change,
    current_pads: Option<&'a Vec<String>>,
    current_name: Option<&'a str>,
    current_nets: BTreeSet<&'a str>,
    status: &'static str,
}

/// Exact live publication and the catalog state consulted for that address. Unrelated catalog
/// replacements do not affect a review; a changed body, availability, or quarantine does.
#[derive(Debug, PartialEq, Eq)]
pub struct LiveSource {
    published: std::sync::Arc<crate::schematic::SymbolDefinition>,
    loaded: Option<std::sync::Arc<crate::schematic::SymbolDefinition>>,
    conflicted: bool,
}

impl LiveSource {
    /// Capture the editor's immutable publication, never a reconstruction of the draft.
    pub fn capture(session: &Session, catalog: &crate::library::sources::Catalog) -> Option<Self> {
        use crate::library::sources::{Key, Kind};
        let published = session.published_symbol()?.clone();
        let key = Key { library: published.library.clone(), kind: Kind::Symbol, id: published.id.clone(), revision: published.revision() };
        let loaded = catalog.symbol(&key.library, &key.id, key.revision);
        Some(Self { published, loaded, conflicted: catalog.conflicting(&key) })
    }
}

/// Explain current nets using the immutable publication actually opened by the live editor.
pub fn preview_live(item: &Item, pins: &[PinRemap], project: Option<&Project>, source: Option<&LiveSource>) -> Result<Value, String> {
    let source = source.filter(|source| source.published.library == item.library && source.published.id == item.id);
    preview_with_source(item, pins, source.map(|source| source.published.revision()), project, None, source)
}

/// Prepare a validated headless edit and explain current nets without predicting a project update.
/// Only an explicitly supplied published revision associates the draft with placed copies. Other
/// pinned revisions and missing terminals are reported, never inferred from geometry or labels.
/// Native part/package mappings are current read-only evidence, not a proposed replacement map.
pub fn preview(
    item: &Item,
    pins: &[PinRemap],
    source_revision: Option<u32>,
    project: Option<&Project>,
    catalog: Option<&crate::library::sources::Catalog>,
) -> Result<Value, String> {
    preview_with_source(item, pins, source_revision, project, catalog, None)
}

/// Prepare a private candidate and its current-project evidence. The live route uses the
/// captured published symbol and its catalog status. Without a base revision, no placed copy
/// is associated, and pinned project definitions do not stand in for a missing revision.
/// With an explicit headless base revision, use the optional catalog to check for conflicts
/// and compare bodies when available. If the catalog supplies neither a body nor a conflict,
/// the project's pinned copies at that revision alone provide the terminal evidence.
/// Check the expanded row budget before preparing the candidate. Invalid edits and row, graph,
/// or encoded report budget overruns return no partial report. A late byte-budget refusal can
/// leave derived connectivity cached, but neither document nor its history changes.
fn preview_with_source(
    item: &Item,
    pins: &[PinRemap],
    source_revision: Option<u32>,
    project: Option<&Project>,
    catalog: Option<&crate::library::sources::Catalog>,
    live: Option<&LiveSource>,
) -> Result<Value, String> {
    let parts = project
        .map_or(0, |project| project.components().filter(|part| part.definition.library == item.library && part.definition.id == item.id).count());
    check_budget(parts, pins.len())?;
    let changes = changes(item, pins)?;
    let mut candidate = Session::new(item.clone(), false)?;
    candidate.execute(0, Action::RemapPins { pins: pins.to_vec() })?;
    let mut report = json!({"item":candidate.snapshot().as_ref(),"changes":changes,"source_revision":source_revision.map(|revision| revision.to_string()),"instances":[],
        "project":project.map(Project::id),"saved":false,"placed_instances_changed":false,
        "notice":"Current nets and native part/package maps only. Publishing does not update placed instances; reviewed project adoption is separate work."});
    let mut bytes = budget::Budget::new();
    bytes.charge(&report, false)?;
    let mut instances = Vec::new();
    if let Some(project) = project {
        use crate::library::sources::{Key, Kind as SourceKind, matches_pinned};
        // The catalog is independent of the authored draft. A missing offline source is not a
        // known conflict; a quarantined base or known unequal revision body is.
        let key = source_revision.map(|revision| Key { library: item.library.clone(), kind: SourceKind::Symbol, id: item.id.clone(), revision });
        let source = catalog.zip(key.as_ref());
        let matches = |pinned: &std::sync::Arc<crate::schematic::SymbolDefinition>, selected: &std::sync::Arc<crate::schematic::SymbolDefinition>| {
            std::sync::Arc::ptr_eq(pinned, selected) || matches_pinned(pinned, selected)
        };
        let (conflicted, base) = if let Some(live) = live {
            let catalog_mismatch = live.loaded.as_ref().is_some_and(|loaded| !matches(&live.published, loaded));
            (live.conflicted || catalog_mismatch, Some(live.published.clone()))
        } else {
            (
                source.is_some_and(|(catalog, key)| catalog.conflicting(key)),
                source.and_then(|(catalog, key)| catalog.symbol(&key.library, &key.id, key.revision)),
            )
        };
        let mut graph = None;
        // Project components own these Arcs for the whole preview. Key by allocation, not
        // published address: distinct pinned bodies at the same address can disagree.
        let mut pinned_matches = BTreeMap::new();
        for part in project.components().filter(|part| part.definition.library == item.library && part.definition.id == item.id) {
            let mut instance = json!({"part_instance":part.id(),"reference":part.reference(),"pinned_revision":part.definition.revision().to_string(),
                "part":part.part.as_ref().map(|selection| json!({"definition":selection.resolved.definition.reference(),"package":selection.package})),"pins":[]});
            bytes.charge(&instance, !instances.is_empty())?;
            let same_revision = source_revision == Some(part.definition.revision());
            let conflict = same_revision
                && (conflicted
                    || base.as_ref().is_some_and(|base| {
                        !*pinned_matches.entry(std::sync::Arc::as_ptr(&part.definition)).or_insert_with(|| matches(&part.definition, base))
                    }));
            let associated = same_revision && !conflict;
            let mut rows = Vec::new();
            for change in &changes {
                let current = associated.then(|| part.definition.pins().iter().find(|pin| pin.number == change.before)).flatten();
                let terminals: Vec<_> = current.into_iter().flat_map(|_| project.component_terminals(part.ordinal, &change.before)).collect();
                if !terminals.is_empty() && graph.is_none() {
                    budget::check_graph(project)?;
                    graph = Some(project.borrowed_electrical_connectivity());
                }
                let nets: BTreeSet<_> = terminals
                    .iter()
                    .filter_map(|terminal| {
                        let graph = graph?;
                        graph.borrowed_net_name(graph.net_index_for(*terminal)?)
                    })
                    .collect();
                let pads = associated
                    .then_some(part.part.as_ref())
                    .flatten()
                    .and_then(|selection| selection.package.as_ref().and_then(|id| selection.resolved.definition.packages.get(id)))
                    .and_then(|package| package.mapping.get(&change.before));
                let row = Evidence {
                    change,
                    current_pads: pads,
                    current_name: current.and_then(|pin| pin.name.as_deref()),
                    current_nets: nets,
                    status: if conflict {
                        "conflicting_definition"
                    } else if !same_revision {
                        "different_revision"
                    } else if current.is_none() {
                        "missing_terminal"
                    } else if terminals.is_empty() {
                        "unplaced_terminal"
                    } else {
                        "current"
                    },
                };
                bytes.charge(&row, !rows.is_empty())?;
                rows.push(serde_json::to_value(&row).map_err(|error| error.to_string())?);
            }
            instance["pins"] = Value::Array(rows);
            instances.push(instance);
        }
    }
    report["instances"] = Value::Array(instances);
    Ok(report)
}
