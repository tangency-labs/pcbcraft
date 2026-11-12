//! Part instance package choices are independent of board coordinates and placement IDs.
use super::{Footprint, definition};
use crate::{CommandError, Project, Session, schematic::Point};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

/// Pin a physical package and explicit logical mapping without creating a placement.
pub const ASSIGN_FOOTPRINT: &str = "pcb.assign_footprint";
/// Place a component's previously assigned package using its exact pinned definition.
pub const PLACE_ASSIGNED_FOOTPRINT: &str = "pcb.place_assigned_footprint";
/// Place a captured group of accepted assignments together at one canvas anchor.
pub const PLACE_ASSIGNED_FOOTPRINTS: &str = "pcb.place_assigned_footprints";

/// Project-owned package intent before a physical board exists. Explicit board creation
/// adopts these exact choices and pinned definitions; it never creates placements.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SchematicPackages {
    pub assignments: BTreeMap<u64, Assignment>,
    pub definitions: BTreeMap<definition::Key, Arc<definition::FootprintDefinition>>,
}

impl SchematicPackages {
    /// Retire a deleted component's choice and any definition no remaining choice references.
    pub(crate) fn remove_component(&mut self, component: u64) {
        if self.assignments.remove(&component).is_some() {
            let used: std::collections::BTreeSet<_> = self.assignments.values().map(Assignment::key).collect();
            self.definitions.retain(|key, _| used.contains(key));
        }
    }
}

/// Persist component identity and immutable package identity separately from board geometry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Assignment {
    pub component: u64,
    pub library: String,
    pub definition: String,
    pub definition_revision: u32,
    pub mapping: BTreeMap<String, crate::library::parts::PadIds>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unconnected_pads: Vec<String>,
}

impl Assignment {
    /// Address the exact pinned definition, never a current-library substitute.
    pub(crate) fn key(&self) -> definition::Key {
        (self.library.clone(), self.definition.clone(), self.definition_revision)
    }

    /// Construct placement vocabulary for the existing shared geometry/mapping validator.
    pub(crate) fn footprint(&self, id: u64, position: Point) -> Footprint {
        Footprint {
            unconnected_pads: self.unconnected_pads.clone(),
            id,
            component: self.component,
            library: Some(self.library.clone()),
            definition: self.definition.clone(),
            definition_revision: self.definition_revision,
            mapping: Some(self.mapping.clone()),
            pin_1_pad: 0,
            x_nm: position.x_nm,
            y_nm: position.y_nm,
            rotation: 0,
            bottom: false,
            reference_offset: None,
            reference_text: None,
        }
    }

    /// Resolve the component and its exact definition before accepting an offline record.
    pub(crate) fn valid(&self, project: &Project) -> bool {
        let footprint = self.footprint(1, Point { x_nm: 0, y_nm: 0 });
        let logical = project.components.get(&self.component).map_or_else(
            || project.board.baseline.as_ref().is_some_and(|baseline| baseline.validate_assignment(self)),
            |component| {
                project.assignment_definition(&self.key()).is_some_and(|definition| {
                    crate::library::parts::mapping::validate(&self.mapping, &self.unconnected_pads, &component.definition, definition).is_ok()
                })
            },
        );
        let part_matches = project.components.get(&self.component).and_then(|part| part.part.as_ref()).is_none_or(|selection| {
            selection.package.as_ref().and_then(|id| selection.resolved.definition.packages.get(id)).is_some_and(|package| {
                self.key() == (package.footprint.library.clone(), package.footprint.id.clone(), package.footprint.revision)
                    && self.mapping.iter().map(|(terminal, pads)| (terminal.clone(), pads.pads().to_vec())).collect::<BTreeMap<_, _>>()
                        == package.mapping
                    && self.unconnected_pads == package.unconnected_pads
            })
        });
        footprint.valid()
            && part_matches
            && logical
            && (project.board.baseline.is_some()
                || project
                    .board
                    .by_component
                    .get(&self.component)
                    .and_then(|i| project.board.footprints().get(*i))
                    .is_none_or(|f| f.key() == Some(self.key()) && f.mapping.as_ref() == Some(&self.mapping)))
    }

    /// Publish stable identities with explicit placement status for GUI and MCP.
    pub(crate) fn inspect(&self, project: &Project) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or(Value::Null);
        value["component"] = json!(format!("component-{}", self.component));
        value["board"] = json!(project.board_target().map(|target| target.board));
        value["placed"] = json!(project.board.by_component.contains_key(&self.component));
        if let Some(baseline) = &project.board.baseline {
            value["pending_update"] = json!(project.component_is_board_eligible(self.component) && !baseline.validate_assignment(self));
        }
        if project.board.baseline.as_ref().is_some_and(|baseline| baseline.components.contains_key(&self.component))
            && !project.component_is_board_eligible(self.component)
        {
            value["pending_removal"] = json!(true);
        }
        value
    }
}

/// A catalog-owned default identifies exact revisions instead of deriving electrical roles
/// from names, package geometry or the current set of starter parts.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MappingDefault {
    symbol: MappingItem,
    footprint: MappingItem,
    mapping: BTreeMap<String, crate::library::parts::PadIds>,
}

/// Revision-qualified item identities in the embedded starter mapping data.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MappingItem {
    library: String,
    id: String,
    revision: u32,
}

/// Accept a proposed default only when it is a complete bijection for the selected definitions.
fn mapping_covers(
    mapping: &BTreeMap<String, crate::library::parts::PadIds>,
    symbol: &crate::schematic::SymbolDefinition,
    footprint: &definition::FootprintDefinition,
) -> bool {
    crate::library::parts::mapping::validate(mapping, &[], symbol, footprint).is_ok()
}

impl Project {
    /// Desired package choices belong to the schematic project until explicitly adopted by
    /// its first board. Accepted placements and their baseline remain board-local.
    pub(crate) fn package_assignments(&self) -> &BTreeMap<u64, Assignment> {
        if self.board_target().is_some() { &self.board.assignments } else { &self.schematic_packages.assignments }
    }

    /// Resolve pinned package intent independently of whether a physical board exists.
    pub(crate) fn assignment_definition(&self, key: &definition::Key) -> Option<&Arc<definition::FootprintDefinition>> {
        self.schematic_packages.definitions.get(key).or_else(|| self.board.definitions.get(key))
    }

    /// List built-in and project-pinned physical packages by immutable identity. Old pinned
    /// revisions remain selectable offline; conflicting current-source content is never adopted.
    pub fn footprint_catalog(&self) -> Vec<Arc<definition::FootprintDefinition>> {
        let mut items = self.board.definitions.clone();
        items.extend(self.schematic_packages.definitions.clone());
        for id in definition::STARTER_IDS {
            if let Some(item) = definition::starter(id) {
                items.entry(item.key()).or_insert(item);
            }
        }
        items.into_values().collect()
    }

    /// Suggest editable pin-to-pad choices without assigning a package or modifying the project.
    /// An existing mapping for the exact package wins, followed by a revision-qualified catalog
    /// default, then matching terminal/pad IDs (including named terminals). Unknown components,
    /// incompatible numbering and invalid default data return None; positions and array order
    /// never establish correspondence. Assignment still validates the explicitly submitted map.
    pub fn default_footprint_mapping(
        &self,
        component: &str,
        footprint: &definition::FootprintDefinition,
    ) -> Option<BTreeMap<String, crate::library::parts::PadIds>> {
        let id = component.strip_prefix("component-")?.parse::<u64>().ok()?;
        if component != format!("component-{id}") {
            return None;
        }
        let symbol = &self.components.get(&id)?.definition;
        let key = footprint.key();
        let saved =
            self.package_assignments().get(&id).filter(|assignment| assignment.key() == key).map(|assignment| &assignment.mapping).or_else(|| {
                self.board
                    .footprints()
                    .iter()
                    .find(|placement| placement.component == id && placement.key().as_ref() == Some(&key))
                    .and_then(|placement| placement.mapping.as_ref())
            });
        if let Some(saved) = saved.filter(|mapping| mapping_covers(mapping, symbol, footprint)) {
            return Some(saved.clone());
        }
        let defaults: Vec<MappingDefault> =
            serde_json::from_str(include_str!("../../../../libraries/starter/footprint-mapping-defaults.json")).ok()?;
        if let Some(default) = defaults.into_iter().find(|default| {
            default.symbol.library == symbol.library
                && default.symbol.id == symbol.id
                && default.symbol.revision == symbol.revision()
                && default.footprint.library == footprint.library
                && default.footprint.id == footprint.id
                && default.footprint.revision == footprint.revision
        }) {
            return mapping_covers(&default.mapping, symbol, footprint).then_some(default.mapping);
        }
        let numbers = symbol.pins().iter().map(|pin| (pin.number.clone(), pin.number.clone().into())).collect();
        mapping_covers(&numbers, symbol, footprint).then_some(numbers)
    }

    /// Publish source identities, pad numbers and provenance without copying full drawings into
    /// every inspection response. Source registration/search can extend this bounded catalog later.
    pub(crate) fn inspect_footprint_catalog(&self) -> Value {
        json!(
            self.footprint_catalog()
                .iter()
                .map(|d| json!({
                    "library": d.library, "id": d.id, "revision": d.revision,
                    "name": d.name, "pads": d.pads.iter().map(|pad| &pad.number).collect::<Vec<_>>(),
                    "license": d.license, "source": d.source
                }))
                .collect::<Vec<_>>()
        )
    }

    /// Report explicit choices and physical legacy placements as assignments without migrating
    /// their saved bytes. Marker-only legacy placements remain outside the physical catalog.
    pub fn footprint_assignments(&self) -> Vec<Value> {
        let mut assignments = self.package_assignments().clone();
        for f in self.board.footprints() {
            if let (Some(library), Some(mapping)) = (&f.library, &f.mapping) {
                assignments.entry(f.component).or_insert_with(|| Assignment {
                    unconnected_pads: f.unconnected_pads.clone(),
                    component: f.component,
                    library: library.clone(),
                    definition: f.definition.clone(),
                    definition_revision: f.definition_revision,
                    mapping: mapping.clone(),
                });
            }
        }
        assignments.values().map(|a| a.inspect(self)).collect()
    }
}

/// Canonical component identities are independent of a sheet's symbol placement IDs.
fn component_id(params: &Value, project: &Project) -> Result<u64, CommandError> {
    let text = params["component"].as_str().unwrap_or_default();
    text.strip_prefix("component-")
        .and_then(|id| id.parse::<u64>().ok())
        .filter(|id| text == format!("component-{id}") && project.components.contains_key(id))
        .ok_or_else(|| CommandError::InvalidParams("unknown component ID".into()))
}

impl Session {
    /// Validate identity and complete mappings before pinning intent. Store project-owned
    /// choices without a board, or stage desired choices independently of accepted geometry
    /// under an existing baseline. Legacy placed
    /// changes remain refused, and identical choices create no history.
    pub(crate) fn assign_footprint(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let component = component_id(params, project)?;
        let assignment = Assignment {
            unconnected_pads: params
                .get("unconnected_pads")
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()
                .map_err(|error| CommandError::InvalidParams(error.to_string()))?
                .unwrap_or_default(),
            component,
            library: params["library"].as_str().unwrap_or_default().into(),
            definition: params["definition"].as_str().unwrap_or_default().into(),
            definition_revision: params["definition_revision"].as_u64().and_then(|r| r.try_into().ok()).unwrap_or(0),
            mapping: super::parse_mapping(&params["mapping"]).ok_or_else(|| CommandError::InvalidParams("invalid terminal-to-pad mapping".into()))?,
        };
        let key = assignment.key();
        let definition = project
            .assignment_definition(&key)
            .cloned()
            .or_else(|| definition::starter(&assignment.definition).filter(|d| d.key() == key))
            .or_else(|| self.libraries.footprint(&key.0, &key.1, key.2))
            .ok_or_else(|| CommandError::InvalidParams("unavailable footprint identity/revision".into()))?;
        let mut next = project.clone();
        if project.board_target().is_some() {
            Arc::make_mut(&mut next.board).definitions.insert(key, definition);
        } else {
            Arc::make_mut(&mut next.schematic_packages).definitions.insert(key, definition);
        }
        if !assignment.valid(&next) {
            let message = if next.board.baseline.is_none() && next.board.by_component.contains_key(&component) {
                "Changing a placed package or mapping requires a reviewed board update."
            } else {
                "Map every terminal and pad exactly once."
            };
            return Err(CommandError::InvalidParams(message.into()));
        }
        if next.package_assignments().get(&component) == Some(&assignment)
            || (!next.package_assignments().contains_key(&component)
                && next
                    .board
                    .footprints()
                    .iter()
                    .any(|f| f.component == component && f.key() == Some(assignment.key()) && f.mapping.as_ref() == Some(&assignment.mapping)))
        {
            return Ok(());
        }
        if next.board_target().is_some() {
            Arc::make_mut(&mut next.board).assignments.insert(component, assignment);
            let used = next.board.used_package_keys();
            Arc::make_mut(&mut next.board).definitions.retain(|key, _| used.contains(key));
        } else {
            let packages = Arc::make_mut(&mut next.schematic_packages);
            packages.assignments.insert(component, assignment);
            let used: std::collections::BTreeSet<_> = packages.assignments.values().map(Assignment::key).collect();
            packages.definitions.retain(|key, _| used.contains(key));
        }
        if next.board.baseline.is_some() {
            if next.numeric_id_high_water.is_some() {
                next.numeric_id_high_water = Some(next.allocation_high_water());
            }
            crate::persistence::encode_documents(Arc::new(next.clone())).map_err(CommandError::InvalidParams)?;
        }
        self.change(next);
        Ok(())
    }

    /// Create one placement from a pinned assignment in one undo step. Source availability is
    /// irrelevant; duplicate placement, stale revisions and out-of-range positions leave it intact.
    /// Pending desired choices must be accepted before they can create physical geometry.
    pub(crate) fn place_assigned_footprint(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let component = component_id(params, project)?;
        if project.components.get(&component).is_some_and(|c| c.metadata.attributes.exclude_from_board) {
            return Err(CommandError::InvalidParams("This component is excluded from the board.".into()));
        }
        let assignment = project
            .board
            .assignments
            .get(&component)
            .ok_or_else(|| CommandError::InvalidParams("component has no unplaced footprint assignment".into()))?;
        if project.board.baseline.as_ref().is_some_and(|baseline| !baseline.validate_assignment(assignment)) {
            return Err(CommandError::Unavailable("Apply the reviewed package update before placing this component.".into()));
        }
        let position = Point { x_nm: params["x_nm"].as_i64().unwrap_or_default(), y_nm: params["y_nm"].as_i64().unwrap_or_default() };
        let footprint = assignment.footprint(self.next_object_id()?, position);
        let mut next = project.clone();
        Arc::make_mut(&mut next.board).append(footprint).map_err(|e| CommandError::InvalidParams(e.into()))?;
        self.change(next);
        Ok(())
    }
}

#[cfg(test)]
mod tests;

/// Parse a bounded ordered set of canonical component addresses without using reference text.
pub(crate) fn component_ids(value: &Value) -> Option<Vec<u64>> {
    let values = value.as_array()?;
    if values.is_empty() || values.len() > crate::MAX_SYMBOLS {
        return None;
    }
    let ids: Vec<_> = values
        .iter()
        .map(|value| {
            let text = value.as_str()?;
            let id = text.strip_prefix("component-")?.parse::<u64>().ok()?;
            (id > 0 && text == format!("component-{id}")).then_some(id)
        })
        .collect::<Option<_>>()?;
    (ids.iter().collect::<std::collections::BTreeSet<_>>().len() == ids.len()).then_some(ids)
}

impl Session {
    /// Build the same validated candidate used by group placement without changing the live
    /// snapshot, allocation watermark, approval capability or history. Only accepted, eligible,
    /// unplaced assignments are included. The preview is disposable and grants no commit authority.
    pub fn preview_footprint_placement(&self, params: &Value) -> Result<Arc<Project>, CommandError> {
        let command = crate::command(PLACE_ASSIGNED_FOOTPRINTS).ok_or_else(|| CommandError::Unknown(PLACE_ASSIGNED_FOOTPRINTS.into()))?;
        crate::check_params(command, params)?;
        self.check_revision(params)?;
        let project = self.open_project()?;
        let target = project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let components = component_ids(&params["components"]).ok_or_else(|| CommandError::InvalidParams("invalid component IDs".into()))?;
        let mut next = project.clone();
        let start = next.board.footprints.len();
        let first_id = self.next_object_id()?;
        for (index, component) in components.iter().enumerate() {
            if !project.component_is_board_eligible(*component) || !project.components.contains_key(component) {
                return Err(CommandError::InvalidParams("unknown or excluded component".into()));
            }
            let assignment =
                project.board.assignments.get(component).ok_or_else(|| CommandError::InvalidParams("component has no assigned footprint".into()))?;
            if project.board.baseline.as_ref().is_some_and(|baseline| !baseline.validate_assignment(assignment)) {
                return Err(CommandError::Unavailable("Apply the reviewed package update before placing these components.".into()));
            }
            let id = first_id
                .checked_add(index as u64)
                .filter(|id| *id <= crate::MAX_OBJECT_ID)
                .ok_or_else(|| CommandError::Unavailable("project has exhausted its object IDs".into()))?;
            Arc::make_mut(&mut next.board)
                .append(assignment.footprint(id, Point { x_nm: 0, y_nm: 0 }))
                .map_err(|error| CommandError::InvalidParams(error.into()))?;
        }
        // Conservative centered cells cover every resolved pad and technical graphic, including
        // off-center origins. This is an initial arrangement, not an electrical clearance check.
        let mut extent = [0i64; 2];
        if let Some(board) = next.board_view(target) {
            for footprint in board.footprints().skip(start) {
                let points = footprint
                    .pads
                    .iter()
                    .flat_map(|pad| pad.copper.into_iter().flatten().chain(std::iter::once(pad.center)))
                    .chain(footprint.graphics.iter().flat_map(|graphic| graphic.points.iter().copied()));
                for point in points {
                    extent[0] = extent[0].max(point.x_nm.abs());
                    extent[1] = extent[1].max(point.y_nm.abs());
                }
            }
        }
        let cell = [extent[0] * 2 + 2_000_000, extent[1] * 2 + 2_000_000];
        let mut columns = 1usize;
        while columns * columns < components.len() {
            columns += 1;
        }
        let rotation = params["rotation"].as_u64().unwrap_or(0) as u16;
        let anchor = Point { x_nm: params["x_nm"].as_i64().unwrap_or_default(), y_nm: params["y_nm"].as_i64().unwrap_or_default() };
        for (index, footprint) in Arc::make_mut(&mut next.board).footprints.iter_mut().skip(start).enumerate() {
            let (x, y) = super::rotate((index % columns) as i64 * cell[0], (index / columns) as i64 * cell[1], rotation);
            footprint.x_nm = anchor.x_nm + x;
            footprint.y_nm = anchor.y_nm + y;
            footprint.rotation = rotation;
            if !footprint.valid() {
                return Err(CommandError::InvalidParams("footprint group exceeds the board coordinate range".into()));
            }
        }
        if next.numeric_id_high_water.is_some() {
            next.numeric_id_high_water = Some(next.allocation_high_water());
        }
        crate::persistence::encode_documents(Arc::new(next.clone())).map_err(CommandError::InvalidParams)?;
        Ok(Arc::new(next))
    }
}
