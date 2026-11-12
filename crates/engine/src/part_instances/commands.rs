//! Shared revision-checked transactions pin native parts, package choices and optional manufacturer metadata.
use super::Selection;
use crate::{
    Command, CommandError, Param, ParamKind, Session,
    library::{
        parts::Reference,
        sources::{manufacturer_digest, part_digest},
    },
    schematic,
};
use serde_json::{Value, json};
use std::sync::Arc;
/// Place one reusable part with its exact native dependency closure.
pub const PLACE_PART: &str = "schematic.place_part";
/// Update a part's package, requirements or manufacturer selection without changing its identity.
pub const SET_PART: &str = "part.set_selection";
const PACKAGE: Param =
    Param { name: "package", required: false, summary: "Package variant ID; null leaves a schematic-only part", kind: ParamKind::OptionalNativeId };
const MANUFACTURER: Param = Param {
    name: "manufacturer",
    required: false,
    summary: "Exact native manufacturer-part reference; null clears it",
    kind: ParamKind::NativeReference,
};
const MANUFACTURER_DIGEST: Param = Param {
    name: "manufacturer_digest",
    required: false,
    summary: "Digest of the selected manufacturer record from library.search",
    kind: ParamKind::LibraryDigest,
};
const REQUIREMENTS: Param = Param {
    name: "requirements",
    required: false,
    summary: "Typed design requirements keyed by category field ID",
    kind: ParamKind::PartRequirements,
};
/// Commands are discoverable by GUI and MCP through the ordinary engine catalog.
pub(crate) const COMMANDS: [Command; 2] = [
    Command {
        id: PLACE_PART,
        title: "Place Part",
        summary: "Pin an exact native part, its dependencies, package and optional manufacturer selection in one undoable placement.",
        params: &[
            Param { name: "library", required: true, summary: "Native part library", kind: ParamKind::LibrarySource },
            Param { name: "definition", required: true, summary: "Reusable part ID", kind: ParamKind::LibrarySource },
            Param { name: "definition_revision", required: true, summary: "Exact part revision", kind: ParamKind::DefinitionRevision },
            Param {
                name: "definition_digest",
                required: true,
                summary: "Dependency digest from library.resolve_part or library.search",
                kind: ParamKind::LibraryDigest,
            },
            PACKAGE,
            MANUFACTURER,
            MANUFACTURER_DIGEST,
            REQUIREMENTS,
            Param {
                name: "unit",
                required: false,
                summary: "Schematic unit ID; defaults to the definition's initial unit",
                kind: ParamKind::LibrarySource,
            },
            Param {
                name: "value",
                required: false,
                summary: "Initial Value; normalized when the part declares a primary quantity",
                kind: ParamKind::Text,
            },
            Param { name: "rotation", required: false, summary: "Counterclockwise degrees", kind: ParamKind::Rotation },
            crate::SYMBOL_X,
            crate::SYMBOL_Y,
            crate::EXPECTED_REVISION,
            crate::AUTO_HEAL,
        ],
    },
    Command {
        id: SET_PART,
        title: "Edit Part Selection",
        summary: "Edit package, typed requirements or MPN in one transaction. Later requirement/package changes retain an incompatible MPN with explicit review findings.",
        params: &[
            Param { name: "component", required: true, summary: "Existing part instance ID", kind: ParamKind::Text },
            PACKAGE,
            MANUFACTURER,
            MANUFACTURER_DIGEST,
            REQUIREMENTS,
            crate::EXPECTED_REVISION,
        ],
    },
];
/// Read an optional native manufacturer reference without coercing missing, null and malformed records.
fn manufacturer(session: &Session, params: &Value, selection: &mut Selection) -> Result<bool, CommandError> {
    let Some(value) = params.get("manufacturer") else {
        return Ok(false);
    };
    if value.is_null() {
        selection.manufacturer = None;
        return Ok(false);
    }
    let reference: Reference = serde_json::from_value(value.clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
    let record = session.libraries.manufacturer_part(&reference).map_err(CommandError::InvalidParams)?;
    if params["manufacturer_digest"].as_str() != Some(manufacturer_digest(&record).as_str()) {
        return Err(CommandError::InvalidParams("Manufacturer record changed; inspect the native catalog again.".into()));
    }
    if record.part != selection.resolved.definition.reference() {
        return Err(CommandError::InvalidParams("Manufacturer part belongs to another reusable part.".into()));
    }
    if params.get("package").is_none() {
        selection.package = Some(record.package.clone());
    }
    if selection.manufacturer.as_ref().is_some_and(|current| current.reference() == reference && current.as_ref() != record.as_ref()) {
        return Err(CommandError::InvalidParams("Manufacturer content conflicts with the pinned record at the same revision.".into()));
    }
    selection.manufacturer = Some(record);
    selection.manufacturer_primary_default();
    Ok(true)
}
/// Apply only supplied selection fields; omission preserves existing package, MPN and requirements.
fn edited(session: &Session, params: &Value, mut selection: Selection) -> Result<Selection, CommandError> {
    if let Some(value) = params.get("package") {
        selection.package = value.as_str().map(str::to_owned);
    }
    if let Some(value) = params.get("requirements") {
        selection.requirements = serde_json::from_value(value.clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
    }
    let choosing_manufacturer = manufacturer(session, params, &mut selection)?;
    selection.validate().map_err(CommandError::InvalidParams)?;
    if choosing_manufacturer && selection.compatibility()["status"] != "meets_requirements" {
        return Err(CommandError::InvalidParams(format!(
            "Manufacturer specifications do not meet the selected package and requirements: {}",
            selection.compatibility()["findings"]
        )));
    }
    Ok(selection)
}
impl Session {
    /// Build an isolated ordinary symbol/assignment transaction, then publish its complete result once.
    pub(crate) fn place_library_part(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let reference = Reference {
            library: params["library"].as_str().unwrap_or_default().into(),
            id: params["definition"].as_str().unwrap_or_default().into(),
            revision: params["definition_revision"].as_u64().and_then(|v| u32::try_from(v).ok()).unwrap_or(0),
        };
        let resolved = self.libraries.resolve_part(&reference).map_err(CommandError::InvalidParams)?.clone();
        if params["definition_digest"].as_str() != Some(part_digest(&resolved).as_str()) {
            return Err(CommandError::InvalidParams("Part or one of its dependencies changed; inspect the catalog again.".into()));
        }
        let package = (resolved.definition.packages.len() == 1).then(|| resolved.definition.packages.keys().next().cloned()).flatten();
        let mut selection =
            edited(self, params, Selection { resolved: Arc::new(resolved), package, manufacturer: None, requirements: Default::default() })?;
        if let Some(value) = params.get("value").and_then(Value::as_str) {
            selection = selection.with_primary_value(value).map_err(CommandError::InvalidParams)?;
        }
        selection.manufacturer_primary_default();
        if selection.manufacturer.is_some() && selection.compatibility()["status"] != "meets_requirements" {
            return Err(CommandError::InvalidParams("Manufacturer specifications do not meet the initial requirements.".into()));
        }
        // Catalog and pinned project snapshots remain immutable; failed work never reaches the live session.
        let mut transaction = Self {
            project: self.project.clone(),
            libraries: self.libraries.clone(),
            revision: self.revision,
            history_root_revision: self.history_root_revision,
            ..Self::default()
        };
        let mut symbol_params = params.clone();
        if let Some(value) = selection.primary_value() {
            symbol_params["value"] = json!(value);
        }
        transaction.place_symbol(selection.resolved.symbol.clone(), &symbol_params)?;
        let mut candidate = transaction.open_project()?.clone();
        let id = candidate.symbols().last().ok_or_else(|| CommandError::InvalidParams("Part placement produced no unit.".into()))?.component;
        // Reuse the project's accepted attribution for an otherwise identical legacy pinned symbol.
        let symbol =
            candidate.components.get(&id).ok_or_else(|| CommandError::InvalidParams("Part instance disappeared.".into()))?.definition.clone();
        Arc::make_mut(&mut selection.resolved).symbol = symbol;
        Arc::make_mut(&mut candidate.components).get_mut(&id).ok_or_else(|| CommandError::InvalidParams("Unknown new part.".into()))?.part =
            Some(selection);
        transaction.project = Some(Arc::new(candidate));
        transaction.assign_selected_package(id)?;
        let candidate = transaction.open_project()?.clone();
        super::storage::validate_project(&candidate).map_err(CommandError::InvalidParams)?;
        crate::persistence::encode_documents(Arc::new(candidate.clone())).map_err(CommandError::InvalidParams)?;
        self.change(candidate);
        Ok(())
    }
    /// Pin and assign a selected package through the ordinary assignment validator, without creating geometry.
    fn assign_selected_package(&mut self, id: u64) -> Result<(), CommandError> {
        let project = self.open_project()?;
        let selection = project
            .components
            .get(&id)
            .and_then(|part| part.part.clone())
            .ok_or_else(|| CommandError::InvalidParams("No reusable part selection.".into()))?;
        let mut candidate = project.clone();
        let Some(package_id) = &selection.package else {
            if candidate.board.footprints().iter().any(|footprint| footprint.component == id) {
                return Err(CommandError::InvalidParams("Remove the physical footprint before clearing its package choice.".into()));
            }
            if candidate.board_target().is_some() {
                Arc::make_mut(&mut candidate.board).assignments.remove(&id);
            } else {
                Arc::make_mut(&mut candidate.schematic_packages).assignments.remove(&id);
            }
            self.project = Some(Arc::new(candidate));
            return Ok(());
        };
        let package =
            selection.resolved.definition.packages.get(package_id).ok_or_else(|| CommandError::InvalidParams("Unknown package variant.".into()))?;
        let footprint = selection
            .resolved
            .footprints
            .get(package_id)
            .ok_or_else(|| CommandError::InvalidParams("Missing pinned package footprint.".into()))?
            .clone();
        if candidate.assignment_definition(&footprint.key()).is_some_and(|existing| existing.as_ref() != footprint.as_ref()) {
            return Err(CommandError::InvalidParams("Footprint conflicts with the project's pinned content at the same identity/revision.".into()));
        }
        if candidate.board_target().is_some() {
            Arc::make_mut(&mut candidate.board).definitions.insert(footprint.key(), footprint);
        } else {
            Arc::make_mut(&mut candidate.schematic_packages).definitions.insert(footprint.key(), footprint);
        }
        self.project = Some(Arc::new(candidate));
        self.assign_footprint(&json!({"component":format!("component-{id}"),"library":package.footprint.library,"definition":package.footprint.id,
            "definition_revision":package.footprint.revision,"mapping":package.mapping,"unconnected_pads":package.unconnected_pads,"expected_revision":self.revision()}))
    }
    /// Retain stable part identity while editing selection data. Invalid edits leave project and history intact.
    pub(crate) fn edit_library_part(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let id = params["component"]
            .as_str()
            .and_then(|id| schematic::wiring::parse_id(id, "component-"))
            .ok_or_else(|| CommandError::InvalidParams("Invalid part instance ID.".into()))?;
        let project = self.open_project()?;
        let part = project.components.get(&id).ok_or_else(|| CommandError::InvalidParams("Unknown part instance.".into()))?;
        let previous = part.part.clone().ok_or_else(|| CommandError::InvalidParams("This instance has no reusable part definition.".into()))?;
        let selection = edited(self, params, previous.clone())?;
        if selection == previous {
            return Ok(());
        }
        let mut candidate = project.clone();
        let part =
            Arc::make_mut(&mut candidate.components).get_mut(&id).ok_or_else(|| CommandError::InvalidParams("Unknown part instance.".into()))?;
        if let Some(value) = selection.primary_value() {
            part.value = value;
        }
        part.part = Some(selection);
        let mut transaction = Self {
            project: Some(Arc::new(candidate)),
            libraries: self.libraries.clone(),
            revision: self.revision,
            history_root_revision: self.history_root_revision,
            ..Self::default()
        };
        transaction.assign_selected_package(id)?;
        let candidate = transaction.open_project()?.clone();
        super::storage::validate_project(&candidate).map_err(CommandError::InvalidParams)?;
        crate::persistence::encode_documents(Arc::new(candidate.clone())).map_err(CommandError::InvalidParams)?;
        self.change(candidate);
        Ok(())
    }
}
