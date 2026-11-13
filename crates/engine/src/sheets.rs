//! Sheet ownership, immutable schematic projections and atomic sheet transactions.

use super::*;
use std::collections::BTreeSet;
use std::ops::Deref;

/// Bound metadata and navigation as well as per-sheet geometry.
pub const MAX_SHEETS: usize = 64;
/// Create a named empty flat sheet as one project undo step.
pub const CREATE_SHEET: &str = "sheet.create";
/// Rename an existing sheet without changing its identity or file path.
pub const RENAME_SHEET: &str = "sheet.rename";
/// Remove a sheet and its owned placements, retaining at least one sheet.
pub const DELETE_SHEET: &str = "sheet.delete";
pub(super) const SHEET: Param = Param { required: true, name: "sheet", summary: "Sheet UUID from project inspection", kind: ParamKind::SheetId };
pub(super) const SHEET_NAME: Param =
    Param { required: true, name: "name", summary: "Sheet name, unique without case within the project", kind: ParamKind::Name };

impl SheetTarget {
    /// UUID used by sheet-scoped commands. The containing project remains part of this address.
    pub fn id(self) -> uuid::Uuid {
        self.sheet
    }
}

/// Authored geometry and hierarchy interfaces of one schematic sheet. Logical components and board placements remain project-owned.
#[derive(Clone, Debug)]
pub struct SchematicSheet {
    pub(crate) sheet_id: uuid::Uuid,
    pub(crate) sheet_name: String,
    /// Occurrence identity in the adopted single-use format; legacy IDs are derived on inspection.
    pub(crate) occurrence_id: Option<uuid::Uuid>,
    pub(crate) symbols: Arc<Vec<SymbolPlacement>>,
    pub(crate) wires: Arc<Vec<Wire>>,
    pub(crate) junctions: Arc<BTreeMap<u64, Point>>,
    pub(crate) labels: Arc<Vec<crate::hierarchy::Label>>,
    pub(crate) net_labels: Arc<Vec<crate::net_labels::Label>>,
    pub(crate) subsheets: Arc<Vec<crate::hierarchy::SheetSymbol>>,
    pub(crate) connectivity: Arc<OnceLock<Connectivity>>,
    /// Provenance for legs drawn by retained named-contact poses; live records persist in sheet/7.
    pub(crate) generated_legs: Arc<Vec<crate::wire::GeneratedLeg>>,
    /// Explicit no-connect intent on project-unique wire-end junction identities.
    pub(crate) no_connect_ends: Arc<BTreeSet<u64>>,
}

impl SchematicSheet {
    /// Create an empty sheet after identity/name validation by the owning command or codec.
    /// Occurrence adoption is separate; new authored sheets use `Project::new_sheet_record`.
    pub(crate) fn empty(sheet_id: uuid::Uuid, sheet_name: String) -> Self {
        Self {
            sheet_id,
            sheet_name,
            occurrence_id: None,
            labels: Arc::default(),
            net_labels: Arc::default(),
            subsheets: Arc::default(),
            symbols: Arc::default(),
            wires: Arc::default(),
            junctions: Arc::default(),
            connectivity: Arc::default(),
            generated_legs: Arc::default(),
            no_connect_ends: Arc::default(),
        }
    }

    /// Whether a recorded generated leg still exactly describes a wire on this sheet.
    pub(crate) fn has_live_generated_legs(&self) -> bool {
        self.generated_legs.iter().any(|leg| self.wires.iter().any(|wire| leg.matches(wire)))
    }
}

/// Stable document identity and presentation metadata in project order.
#[derive(Clone, Copy, Debug)]
pub struct SheetInfo<'a> {
    pub target: SheetTarget,
    pub name: &'a str,
}

impl Deref for Project {
    type Target = SchematicSheet;
    /// Geometry helpers operate on this explicitly scoped immutable projection.
    fn deref(&self) -> &Self::Target {
        &self.sheets[self.sheet_index]
    }
}

impl Project {
    /// Highest live or retired numeric identity reserved by this saved lineage.
    pub(crate) fn allocation_high_water(&self) -> u64 {
        self.highest_id().max(self.numeric_id_high_water.unwrap_or(0))
    }

    /// Overlay monotonic metadata on historical content without mutating a shared snapshot.
    /// Keep the original pointer when neither adoption nor a larger watermark is needed.
    pub(crate) fn with_allocation_watermark(project: Arc<Self>, previous: Option<u64>) -> Arc<Self> {
        let watermark = match (project.numeric_id_high_water, previous) {
            (None, None) => None,
            (current, previous) => Some(current.unwrap_or(0).max(previous.unwrap_or(0)).max(project.highest_id())),
        };
        if watermark == project.numeric_id_high_water {
            project
        } else {
            Arc::new(Self { numeric_id_high_water: watermark, ..Self::clone(&project) })
        }
    }

    /// Internal commands clone only the addressed sheet's metadata and touched geometry.
    pub(crate) fn sheet_mut(&mut self) -> &mut SchematicSheet {
        // Labels can contact geometry and join other sheets, including in pure previews.
        if self.requires_project_graph() {
            self.named_graph = Arc::default();
            for sheet in Arc::make_mut(&mut self.sheets) {
                sheet.connectivity = Arc::default();
            }
        }
        &mut Arc::make_mut(&mut self.sheets)[self.sheet_index]
    }

    /// Enumerate stable sheet addresses and names without making a view's active sheet global.
    pub fn sheets(&self) -> impl ExactSizeIterator<Item = SheetInfo<'_>> {
        self.sheets.iter().map(|sheet| SheetInfo { target: SheetTarget { project: self.id, sheet: sheet.sheet_id }, name: &sheet.sheet_name })
    }

    /// Project this immutable snapshot onto exactly one sheet. Every other document and shared
    /// component remains available; encoding a projection still writes the complete project.
    pub fn for_sheet(&self, target: SheetTarget) -> Option<Self> {
        let sheet_index = self.sheets().position(|sheet| sheet.target == target)?;
        Some(Self { sheet_index, ..self.clone() })
    }

    /// Resolve explicit UUIDs. Omission is accepted only for legacy single-sheet callers.
    pub fn resolve_sheet(&self, id: Option<&str>) -> Result<SheetTarget, CommandError> {
        match id {
            Some(id) => self
                .sheets()
                .find(|sheet| sheet.target.sheet.to_string() == id)
                .map(|sheet| sheet.target)
                .ok_or_else(|| CommandError::InvalidParams("unknown sheet ID".into())),
            None if self.sheets.len() == 1 => Ok(self.sheet_target()),
            None => Err(CommandError::InvalidParams("sheet is required when a project has multiple sheets".into())),
        }
    }

    /// All units in project order, used for project-wide physical correspondence and validation.
    pub fn all_symbols(&self) -> impl Iterator<Item = &SymbolPlacement> {
        self.sheets.iter().flat_map(|sheet| sheet.symbols.iter())
    }

    /// The current sheet's display name, independent of identity and file path.
    pub fn sheet_name(&self) -> &str {
        &self.sheet_name
    }

    /// Reserve IDs across every document, including inactive sheets and board placements.
    pub(super) fn highest_id(&self) -> u64 {
        self.sheets
            .iter()
            .map(|sheet| {
                sheet
                    .labels
                    .iter()
                    .map(|l| l.id)
                    .chain(sheet.net_labels.iter().map(|l| l.id))
                    .chain(sheet.subsheets.iter().map(|s| s.id))
                    .max()
                    .unwrap_or(0)
                    .max(
                        sheet
                            .symbols
                            .last()
                            .map_or(0, |s| s.ordinal)
                            .max(sheet.wires.last().map_or(0, |w| w.ordinal))
                            .max(sheet.junctions.last_key_value().map_or(0, |(&id, _)| id)),
                    )
            })
            .max()
            .unwrap_or_default()
            .max(self.components.last_key_value().map_or(0, |(&id, _)| id))
            .max(self.net_registry.highest_object_id())
            .max(self.board.baseline.as_ref().and_then(|baseline| baseline.components.keys().next_back().copied()).unwrap_or(0))
            .max(self.board.footprints().last().map_or(0, |f| f.id))
    }
}

/// Authored schematic geometry takes a sheet target; portable text preparation is independent
/// of any project/sheet. Logical no-connect intent, sheet lifecycle and board setup remain
/// project operations; their targets do not depend on the current sheet projection.
pub fn scoped_command(id: &str) -> bool {
    (id.starts_with("schematic.") && !matches!(id, crate::clipboard_text::LABELS | crate::no_connect::SET | crate::no_connect::CHECK))
        || id == PLACE_FOOTPRINT
}

impl Session {
    /// Execute against an immutable sheet projection in an isolated transaction. A failure or
    /// no-op preserves the exact canonical snapshot and history; success commits once project-wide.
    pub(super) fn apply_to_sheet(&mut self, id: &str, params: &Value) -> Result<u64, CommandError> {
        let command = command(id).ok_or_else(|| CommandError::Unknown(id.into()))?;
        check_params(command, params)?;
        let project = self.open_project()?;
        let target = project.resolve_sheet(params.get("sheet").and_then(Value::as_str))?;
        let projected = project.for_sheet(target).ok_or_else(|| CommandError::InvalidParams("unknown sheet ID".into()))?;
        let mut transaction = Self {
            libraries: self.libraries.clone(),
            project: Some(Arc::new(projected)),
            revision: self.revision,
            history_root_revision: self.history_root_revision,
            defer_net_reconciliation: true,
            ..Self::default()
        };
        transaction.apply_inner(id, params)?;
        if transaction.revision != self.revision {
            let mut project = transaction.open_project()?.clone();
            if project.components.len() > MAX_SYMBOLS || project.sheets.iter().map(|s| s.wires.len()).sum::<usize>() > MAX_WIRES {
                return Err(CommandError::Unavailable("project exceeds the symbol or wire limit".into()));
            }
            project.sheet_index = 0;
            self.change(project)?;
        }
        Ok(self.revision)
    }

    /// Create, rename or delete a sheet in one project undo step. Names are unique without case;
    /// deleting a populated sheet removes its live components and retains accepted
    /// footprints when reviewed baseline mode is enabled, otherwise removing them.
    pub(super) fn edit_sheet(&mut self, id: &str, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let mut project = self.open_project()?.clone();
        let name = if id == DELETE_SHEET { None } else { Some(name_param(params)?) };
        let index = if id == CREATE_SHEET {
            None
        } else {
            let target = project.resolve_sheet(params["sheet"].as_str())?;
            project.sheets().position(|sheet| sheet.target == target)
        };
        if let Some(name) = &name
            && project.sheets().enumerate().any(|(i, sheet)| Some(i) != index && sheet.name.to_lowercase() == name.to_lowercase())
        {
            return Err(CommandError::InvalidParams("a sheet with that name already exists".into()));
        }
        match (id, index) {
            (CREATE_SHEET, _) => {
                if project.sheets.len() >= MAX_SHEETS {
                    return Err(CommandError::Unavailable(format!("project already has {MAX_SHEETS} sheets")));
                }
                let name = name.ok_or_else(|| CommandError::InvalidParams("missing sheet name".into()))?;
                let sheet = project.new_sheet_record(fresh_id()?, name)?;
                Arc::make_mut(&mut project.sheets).push(sheet);
            }
            (RENAME_SHEET, Some(index)) => {
                let name = name.ok_or_else(|| CommandError::InvalidParams("missing sheet name".into()))?;
                if project.sheets[index].sheet_name == name {
                    return Ok(());
                }
                Arc::make_mut(&mut project.sheets)[index].sheet_name = name;
            }
            (DELETE_SHEET, Some(index)) => {
                if project.sheets.len() == 1 {
                    return Err(CommandError::Unavailable("a project must retain at least one sheet".into()));
                }
                let deleting = project.sheets[index].sheet_id;
                let pins: std::collections::BTreeSet<_> = project.sheets[index].labels.iter().map(|l| WireEndpoint::SheetPin(l.id)).collect();
                for sheet in Arc::make_mut(&mut project.sheets) {
                    Arc::make_mut(&mut sheet.subsheets).retain(|block| block.child != deleting);
                    Arc::make_mut(&mut sheet.wires).retain(|wire| !wire.endpoints.iter().any(|end| pins.contains(end)));
                    let used: std::collections::BTreeSet<_> = sheet
                        .wires
                        .iter()
                        .flat_map(|w| w.endpoints)
                        .filter_map(|end| if let WireEndpoint::Junction(id) = end { Some(id) } else { None })
                        .collect();
                    Arc::make_mut(&mut sheet.junctions).retain(|id, _| used.contains(id));
                    sheet.connectivity = Arc::default();
                }
                let removed = Arc::make_mut(&mut project.sheets).remove(index);
                for symbol in removed.symbols.iter() {
                    if !project.all_symbols().any(|other| other.component == symbol.component) {
                        project.retire_component(symbol.component);
                    }
                    Arc::make_mut(&mut project.units).remove(&(symbol.component, symbol.unit.clone()));
                }
            }
            _ => return Err(CommandError::InvalidParams("unknown sheet operation or target".into())),
        }
        project.sheet_index = 0;
        self.change(project)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
