//! Named, board-local population choices and variant-aware manufacturing reports.

use super::{Board, view::BoardTarget};
use crate::{CommandError, Project, Session, schematic::PartInstance};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use uuid::Uuid;

#[cfg(test)]
mod tests;

/// Create a named assembly variant whose unset component choices inherit base DNP attributes.
pub const CREATE_ASSEMBLY_VARIANT: &str = "pcb.create_assembly_variant";
/// Rename a variant or set/clear one component's fitted override in one undoable edit.
pub const SET_ASSEMBLY_VARIANT: &str = "pcb.set_assembly_variant";
/// Remove a named assembly variant in one undoable edit.
pub const DELETE_ASSEMBLY_VARIANT: &str = "pcb.delete_assembly_variant";
/// Generate BOM and pick-and-place CSV files for an explicitly selected variant or the base assembly.
pub const EXPORT_ASSEMBLY: &str = "pcb.export_assembly";

/// Bound named variants independently of board geometry and project component limits.
pub const MAX_ASSEMBLY_VARIANTS: usize = 128;

/// One sparse board-local population choice. `true` means fitted; `false` means do not populate.
/// Missing component keys inherit that component's base `do_not_populate` setting.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssemblyVariant {
    /// Stable identity, independent of the variant's editable display name.
    pub id: Uuid,
    /// Stable physical board identity that owns this population alternative.
    pub board: Uuid,
    /// User-facing variant name, unique within the owning board.
    pub name: String,
    /// Sparse fitted overrides keyed by stable logical component ID.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub population: BTreeMap<u64, bool>,
}

/// Return whether a part is fitted in the selected variant, inheriting its base DNP attribute.
fn fitted(component: &PartInstance, overrides: &BTreeMap<u64, bool>) -> bool {
    overrides.get(&component.ordinal).copied().unwrap_or(!component.metadata.attributes.do_not_populate)
}

/// Validate board ownership, variant identities, names and every sparse component reference.
pub(crate) fn validate(project: &Project) -> Result<(), String> {
    let Some(target) = project.board_target() else {
        return if project.board.assembly_variants.is_empty() { Ok(()) } else { Err("assembly variants require a physical board".into()) };
    };
    if project.board.assembly_variants.len() > MAX_ASSEMBLY_VARIANTS {
        return Err(format!("at most {MAX_ASSEMBLY_VARIANTS} assembly variants are supported"));
    }
    let mut names = BTreeMap::<String, Uuid>::new();
    names.insert("base".into(), Uuid::nil());
    for (id, variant) in &project.board.assembly_variants {
        if id.is_nil() || variant.id != *id || variant.board != target.board {
            return Err(format!("assembly variant {id} has an invalid identity or foreign board owner"));
        }
        let name = checked_name(&variant.name).map_err(|error| format!("assembly variant {id}: {error}"))?;
        if names.insert(name.to_lowercase(), *id).is_some() {
            return Err(format!("assembly variant names must be unique and cannot be Base: {}", variant.name));
        }
        for component in variant.population.keys() {
            if !project.components.contains_key(component) {
                return Err(format!("assembly variant {} references missing component component-{component}", variant.id));
            }
        }
        if variant.population.len() > crate::MAX_SYMBOLS {
            return Err(format!("assembly variant {} has too many population overrides", variant.id));
        }
    }
    Ok(())
}

/// Normalize an authored variant name with the same bounds and control-character rules as project names.
fn checked_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("the name can't be empty".into());
    }
    if name.chars().count() > crate::MAX_NAME {
        return Err(format!("the name can't be longer than {} characters", crate::MAX_NAME));
    }
    if name.chars().any(char::is_control) {
        return Err("the name can't contain control characters".into());
    }
    if name.eq_ignore_ascii_case("base") {
        return Err("Base is reserved for the inherited assembly".into());
    }
    Ok(name.to_owned())
}

/// Resolve one registered variant, reporting dangling references instead of silently dropping them.
fn selected<'a>(project: &'a Project, target: BoardTarget, value: &Value) -> Result<Option<&'a AssemblyVariant>, CommandError> {
    if project.resolve_board(Some(&target.board.to_string()))? != target {
        return Err(CommandError::InvalidParams("the assembly export board changed".into()));
    }
    if value.is_null() {
        return Ok(None);
    }
    let id = value
        .as_str()
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| CommandError::InvalidParams("`variant` must be null for Base or a variant UUID from project inspection".into()))?;
    let variant = project.board.assembly_variants.get(&id).ok_or_else(|| CommandError::InvalidParams(format!("unknown assembly variant {id}")))?;
    if variant.board != target.board {
        return Err(CommandError::InvalidParams(format!("assembly variant {id} belongs to a different board")));
    }
    for component in variant.population.keys() {
        if !project.components.contains_key(component) {
            return Err(CommandError::InvalidParams(format!("assembly variant {id} references missing component component-{component}")));
        }
    }
    Ok(Some(variant))
}

impl Board {
    /// Remove population overrides with the component's other live board state; Undo restores both.
    pub(crate) fn retire_assembly_component(&mut self, component: u64) {
        for variant in self.assembly_variants.values_mut() {
            variant.population.remove(&component);
        }
    }
}

impl Session {
    /// Create a stable board-owned variant and return its identity through ordinary project inspection.
    pub(crate) fn create_assembly_variant(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let mut project = self.open_project()?.clone();
        let target = project.resolve_board(params["board"].as_str())?;
        if project.board.assembly_variants.len() >= MAX_ASSEMBLY_VARIANTS {
            return Err(CommandError::Unavailable(format!("at most {MAX_ASSEMBLY_VARIANTS} assembly variants are supported")));
        }
        let name = checked_name(params["name"].as_str().unwrap_or_default()).map_err(CommandError::InvalidParams)?;
        let normalized_name = name.to_lowercase();
        if project.board.assembly_variants.values().any(|variant| variant.name.to_lowercase() == normalized_name) {
            return Err(CommandError::InvalidParams("an assembly variant already uses that name".into()));
        }
        let id = crate::fresh_id()?;
        let variant = AssemblyVariant { id, board: target.board, name, population: BTreeMap::new() };
        Arc::make_mut(&mut project.board).assembly_variants.insert(id, variant);
        project.numeric_id_high_water = Some(project.numeric_id_high_water.unwrap_or(0).max(project.allocation_high_water()));
        self.change(project)
    }

    /// Atomically rename a variant or edit one stable component's fitted override.
    pub(crate) fn set_assembly_variant(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let mut project = self.open_project()?.clone();
        let target = project.resolve_board(params["board"].as_str())?;
        let id = Uuid::parse_str(params["variant"].as_str().unwrap_or_default())
            .map_err(|_| CommandError::InvalidParams("`variant` must be a variant UUID from project inspection".into()))?;
        let mut candidate =
            project.board.assembly_variants.get(&id).cloned().ok_or_else(|| CommandError::InvalidParams(format!("unknown assembly variant {id}")))?;
        if candidate.board != target.board {
            return Err(CommandError::InvalidParams(format!("assembly variant {id} belongs to a different board")));
        }
        let has_name = params.get("name").is_some();
        let has_component = params.get("component").is_some();
        let has_fitted = params.get("fitted").is_some();
        if !has_name && !has_component || has_component != has_fitted {
            return Err(CommandError::InvalidParams("provide a name, or provide both component and fitted".into()));
        }
        if has_name {
            let name = checked_name(params["name"].as_str().unwrap_or_default()).map_err(CommandError::InvalidParams)?;
            let normalized_name = name.to_lowercase();
            if project.board.assembly_variants.values().any(|other| other.id != id && other.name.to_lowercase() == normalized_name) {
                return Err(CommandError::InvalidParams("an assembly variant already uses that name".into()));
            }
            candidate.name = name;
        }
        if has_component {
            let component = params["component"]
                .as_str()
                .and_then(|value| crate::schematic::wiring::parse_id(value, "component-"))
                .ok_or_else(|| CommandError::InvalidParams("`component` must identify a component from project inspection".into()))?;
            if !project.components.contains_key(&component) {
                return Err(CommandError::InvalidParams(format!("unknown component component-{component}")));
            }
            if params["fitted"].is_null() {
                candidate.population.remove(&component);
            } else {
                let fitted = params["fitted"].as_bool().ok_or_else(|| CommandError::InvalidParams("`fitted` must be true, false or null".into()))?;
                candidate.population.insert(component, fitted);
            }
        }
        if project.board.assembly_variants.get(&id) == Some(&candidate) {
            return Ok(());
        }
        Arc::make_mut(&mut project.board).assembly_variants.insert(id, candidate);
        self.change(project)
    }

    /// Delete a variant with ordinary revision, undo and redo semantics.
    pub(crate) fn delete_assembly_variant(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let mut project = self.open_project()?.clone();
        let target = project.resolve_board(params["board"].as_str())?;
        let id = Uuid::parse_str(params["variant"].as_str().unwrap_or_default())
            .map_err(|_| CommandError::InvalidParams("`variant` must be a variant UUID from project inspection".into()))?;
        let variant =
            project.board.assembly_variants.get(&id).ok_or_else(|| CommandError::InvalidParams(format!("unknown assembly variant {id}")))?;
        if variant.board != target.board {
            return Err(CommandError::InvalidParams(format!("assembly variant {id} belongs to a different board")));
        }
        Arc::make_mut(&mut project.board).assembly_variants.remove(&id);
        self.change(project)
    }

    /// Generate an immutable BOM and placement export for an explicit board and variant selection.
    pub fn export_assembly(&self, params: &Value) -> Result<Vec<crate::board::fabrication::Output>, CommandError> {
        let project = self.open_project()?;
        let target = project.resolve_board(params["board"].as_str())?;
        let variant = selected(project, target, &params["variant"])?;
        let empty_overrides = BTreeMap::new();
        let overrides = variant.map_or(&empty_overrides, |variant| &variant.population);
        let mut bom = String::from("\"Reference\",\"Value\",\"Footprint\",\"Component ID\",\"Quantity\"\r\n");
        for component in project.components.values() {
            if component.metadata.attributes.exclude_from_board || component.metadata.attributes.exclude_from_bom || !fitted(component, overrides) {
                continue;
            }
            let footprint = project.board.footprints().iter().find(|footprint| footprint.component == component.ordinal);
            csv_row(
                &mut bom,
                &[&component.reference, &component.value, footprint.map_or("", |footprint| footprint.definition.as_str()), &component.id(), "1"],
            );
        }
        let mut placement =
            String::from("\"Reference\",\"Value\",\"Footprint\",\"X (mm)\",\"Y (mm)\",\"Rotation (deg)\",\"Side\",\"Component ID\"\r\n");
        for footprint in project.board.footprints() {
            let Some(component) = project.components.get(&footprint.component) else { continue };
            if component.metadata.attributes.exclude_from_board
                || component.metadata.attributes.exclude_from_position_files
                || !fitted(component, overrides)
            {
                continue;
            }
            let x = millimeters(footprint.x_nm);
            let y = millimeters(-footprint.y_nm);
            // Board coordinates grow down, but placement files use Y-up; the board's
            // persisted counterclockwise angle is unchanged on the top side. A bottom
            // footprint's local X axis is mirrored before rotation, adding 180 degrees.
            let rotation = if footprint.bottom { (u32::from(footprint.rotation) + 180) % 360 } else { u32::from(footprint.rotation) };
            let rotation = rotation.to_string();
            let side = if footprint.bottom { "bottom" } else { "top" };
            let id = component.id();
            csv_row(&mut placement, &[&component.reference, &component.value, &footprint.definition, &x, &y, &rotation, side, &id]);
        }
        Ok(vec![
            crate::board::fabrication::Output { name: "BOM.csv".into(), contents: bom },
            crate::board::fabrication::Output { name: "Placement.csv".into(), contents: placement },
        ])
    }

    /// Publish the query result for MCP clients without changing the project or revision.
    pub(crate) fn assembly_report(&self, params: &Value) -> Result<Value, CommandError> {
        let files = self.export_assembly(params)?;
        let variant = if params["variant"].is_null() { Value::Null } else { params["variant"].clone() };
        Ok(json!({"board":params["board"],"variant":variant,"files":files}))
    }
}

/// Quote CSV cells and terminate each record with the RFC 4180 CRLF sequence.
fn csv_row(output: &mut String, values: &[&str]) {
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push('"');
        for character in value.chars() {
            if character == '"' {
                output.push('"');
            }
            output.push(character);
        }
        output.push('"');
    }
    output.push_str("\r\n");
}

/// Format bounded board nanometers as exact, six-decimal millimeters without floating-point rounding.
fn millimeters(nanometers: i64) -> String {
    let sign = if nanometers < 0 { "-" } else { "" };
    let magnitude = nanometers.unsigned_abs();
    format!("{sign}{}.{:06}", magnitude / 1_000_000, magnitude % 1_000_000)
}
