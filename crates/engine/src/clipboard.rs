//! Portable, bounded design fragments. Clipboard adapters carry text; only shared commands edit projects.
use crate::{
    CommandError, Project, Session,
    schematic::{Point, Terminal, WireEndpoint},
    selection::Selection,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Copy selected schematic objects and their pinned definitions without editing the project.
pub const COPY: &str = "schematic.copy";
/// Copy and remove selected objects, preserving unselected incident wires as dangling routes.
pub const CUT: &str = "schematic.cut";
/// Insert a portable schematic fragment with new identities in one undoable transaction.
pub const PASTE: &str = "schematic.paste";
/// Maximum UTF-8 clipboard size, checked before JSON parsing.
pub const MAX_BYTES: usize = 8 << 20;
const FORMAT: &str = "pcbcraft-clipboard/1";
const HIERARCHY_FORMAT: &str = "pcbcraft-clipboard/2";
mod hierarchy;
mod preview;
pub(crate) use hierarchy::hierarchy_objects;
pub use preview::SchematicPastePreview;

/// Wire format uses the native codecs for definition qualification and whole-fragment validation.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    format: String,
    kind: String,
    anchor: Point,
    documents: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_sheet: Option<uuid::Uuid>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    cut_sheets: bool,
}

/// An immutable validated fragment, independent of the source session, view and library catalog.
#[derive(Clone, Debug)]
pub struct Fragment {
    pub(crate) project: Arc<Project>,
    anchor: Point,
    cut_sheets: bool,
}

/// Paste policy distinguishes independent duplication from explicit Cut relocation. Invalid
/// references, names or ownership choices reject the whole batch.
#[derive(Clone, Debug, Default)]
pub struct PasteOptions {
    /// Keep copied references exactly, rejecting any collision; false allocates unused references.
    pub preserve_references: bool,
    /// Suffix for every copied net-label name and native power-symbol value.
    pub net_name_suffix: String,
    /// Counterclockwise quarter turns about the anchor; hierarchical objects retain orientation.
    pub quarter_turns: u8,
    /// Relocate eligible original child sheets after Cut, retaining their current logical content.
    pub reuse_sheets: bool,
}

/// Convert errors from portable codecs into a rejected command with no state change.
fn invalid(message: impl ToString) -> CommandError {
    CommandError::InvalidParams(message.to_string())
}

/// Allocate only inside a disposable candidate. The session reserves the high water on commit.
pub(crate) fn allocate(cursor: &mut u64) -> Result<u64, CommandError> {
    let id = *cursor;
    *cursor = cursor.checked_add(1).filter(|next| *next <= crate::MAX_OBJECT_ID).ok_or_else(|| invalid("clipboard exhausts object IDs"))?;
    Ok(id)
}

/// Translate/rotate using checked arithmetic; pasted geometry cannot escape native coordinate bounds.
pub(crate) fn transformed(point: Point, anchor: Point, at: Point, turns: u8) -> Result<Point, CommandError> {
    let (x, y) = crate::schematic::rotate(point.x_nm - anchor.x_nm, point.y_nm - anchor.y_nm, turns % 4);
    let point = Point {
        x_nm: at.x_nm.checked_add(x).ok_or_else(|| invalid("paste exceeds coordinate bounds"))?,
        y_nm: at.y_nm.checked_add(y).ok_or_else(|| invalid("paste exceeds coordinate bounds"))?,
    };
    if !crate::wire::valid_point(&point) {
        return Err(invalid("paste exceeds coordinate bounds"));
    }
    Ok(point)
}

impl Fragment {
    /// Capture exactly the selected objects. Unselected endpoints become independent free endpoints;
    /// selected wire routes retain their full geometry and shared endpoint identities.
    pub fn capture(project: &Project, items: &Selection) -> Result<Self, CommandError> {
        let mut found = items.clone();
        found.retain(project);
        if items.is_empty() || &found != items {
            return Err(invalid("copy needs existing selected objects"));
        }
        let mut sheet = crate::sheets::SchematicSheet::empty(project.sheet_target().id(), "Clipboard".into());
        sheet.symbols = Arc::new(project.symbols().iter().filter(|s| items.symbols.contains(&s.id())).cloned().collect());
        sheet.net_labels = Arc::new(project.net_labels().iter().filter(|l| items.net_labels.contains(&l.object_id())).cloned().collect());
        let symbols: BTreeSet<_> = sheet.symbols.iter().map(|s| s.ordinal).collect();
        let labels: BTreeSet<_> = sheet.net_labels.iter().map(|l| l.id).collect();
        let mut detached = BTreeMap::new();
        let mut cursor = project.allocation_high_water().checked_add(1).ok_or_else(|| invalid("clipboard exhausts object IDs"))?;
        for source in project.wires().iter().filter(|w| items.wires.contains(&w.id())) {
            let mut wire = source.clone();
            let route = project.wire_path(source);
            wire.bends = Some(route[1..route.len() - 1].to_vec());
            for endpoint in &mut wire.endpoints {
                match *endpoint {
                    WireEndpoint::Terminal(pin) if symbols.contains(&pin.symbol) => {}
                    WireEndpoint::NetLabel(id) if labels.contains(&id) => {}
                    original => {
                        let id = if let Some(id) = detached.get(&original) {
                            *id
                        } else {
                            let id = allocate(&mut cursor)?;
                            let point = project.endpoint_position(original).ok_or_else(|| invalid("missing copied endpoint"))?;
                            Arc::make_mut(&mut sheet.junctions).insert(id, point);
                            detached.insert(original, id);
                            id
                        };
                        *endpoint = WireEndpoint::Junction(id);
                    }
                }
            }
            Arc::make_mut(&mut sheet.wires).push(wire);
        }
        Self::captured(project, vec![sheet], cursor, false)
    }

    /// Keep only captured sheets, their live components and exact package intent. This shared
    /// boundary excludes board placements and unrelated project assets for flat/tree captures.
    fn captured(project: &Project, sheets: Vec<crate::sheets::SchematicSheet>, cursor: u64, cut_sheets: bool) -> Result<Self, CommandError> {
        let components: BTreeSet<_> = sheets.iter().flat_map(|s| s.symbols.iter().map(|s| s.component)).collect();
        let symbols: BTreeSet<_> = sheets.iter().flat_map(|s| s.symbols.iter().map(|s| s.ordinal)).collect();
        let mut fragment = project.clone();
        fragment.sheets = Arc::new(sheets);
        fragment.sheet_index = 0;
        Arc::make_mut(&mut fragment.components).retain(|id, _| components.contains(id));
        Arc::make_mut(&mut fragment.units).retain(|_, id| symbols.contains(id));
        fragment.board = Arc::new(crate::board::Board::for_new_project(false)?);
        let assignments = project
            .board
            .assignments
            .iter()
            .chain(project.schematic_packages.assignments.iter())
            .filter(|(id, _)| components.contains(id))
            .map(|(id, assignment)| (*id, assignment.clone()))
            .collect::<BTreeMap<_, _>>();
        let definitions = assignments.values().filter_map(|a| project.assignment_definition(&a.key()).map(|d| (a.key(), d.clone()))).collect();
        fragment.schematic_packages = Arc::new(crate::board::assignment::SchematicPackages { assignments, definitions });
        fragment.named_graph = Arc::default();
        fragment.numeric_id_high_water = Some(cursor.saturating_sub(1));
        let anchor = fragment
            .symbols()
            .first()
            .map(|s| s.position)
            .or_else(|| fragment.wires().first().and_then(|w| fragment.endpoint_position(w.endpoints()[0])))
            .or_else(|| fragment.net_labels().first().map(|l| l.at))
            .or_else(|| fragment.hierarchical_labels().first().map(|l| l.at))
            .or_else(|| fragment.sheet_symbols().first().map(|s| s.at))
            .ok_or_else(|| invalid("nothing to copy"))?;
        Ok(Self { project: Arc::new(fragment), anchor, cut_sheets })
    }

    /// Decode bounded native text: version 1 is flat; version 2 contains a closed hierarchy tree
    /// or root interfaces. Unsupported versions, board owners, orphan scope and invalid records
    /// fail before any destination edit or identity allocation.
    pub fn from_text(text: &str) -> Result<Self, CommandError> {
        if text.len() > MAX_BYTES {
            return Err(invalid("clipboard exceeds the 8 MiB limit"));
        }
        let envelope: Envelope =
            serde_json::from_str(text).map_err(|_| invalid("Clipboard does not contain a supported PCBcraft design fragment."))?;
        if !matches!(envelope.format.as_str(), FORMAT | HIERARCHY_FORMAT)
            || envelope.kind != "schematic"
            || !crate::wire::valid_point(&envelope.anchor)
        {
            return Err(invalid("Unsupported clipboard format or editor kind."));
        }
        let documents = envelope.documents.into_iter().map(|(path, text)| (path, text.into_bytes())).collect();
        let project = crate::persistence::decode_documents(&documents).map_err(invalid)?;
        let project = if envelope.format == HIERARCHY_FORMAT {
            let root =
                envelope.root_sheet.filter(|id| !id.is_nil()).ok_or_else(|| invalid("hierarchy clipboard needs an explicit root sheet UUID"))?;
            Arc::new(
                project
                    .for_sheet(crate::SheetTarget { project: project.id(), sheet: root })
                    .ok_or_else(|| invalid("clipboard root sheet is unavailable"))?,
            )
        } else {
            if envelope.root_sheet.is_some() {
                return Err(invalid("legacy flat clipboard cannot carry a hierarchy root"));
            }
            project
        };
        let fragment = Self { project, anchor: envelope.anchor, cut_sheets: envelope.cut_sheets };
        fragment.validate_scope(envelope.format == HIERARCHY_FORMAT)?;
        Ok(fragment)
    }

    /// Serialize just the fragment; no paths, credentials, unused definitions or source files travel.
    pub fn to_text(&self) -> Result<String, CommandError> {
        let documents = crate::persistence::encode_documents(self.project.clone())
            .map_err(invalid)?
            .into_iter()
            .map(|(path, bytes)| String::from_utf8(bytes).map(|text| (path, text)).map_err(invalid))
            .collect::<Result<_, _>>()?;
        let text = serde_json::to_string(&Envelope {
            format: if self.has_hierarchy() { HIERARCHY_FORMAT } else { FORMAT }.into(),
            kind: "schematic".into(),
            anchor: self.anchor,
            documents,
            cut_sheets: self.cut_sheets,
            root_sheet: self.has_hierarchy().then(|| self.project.sheet_target().id()),
        })
        .map_err(invalid)?;
        if text.len() > MAX_BYTES {
            return Err(invalid("clipboard exceeds the 8 MiB limit"));
        }
        Ok(text)
    }

    /// Original anchor for Paste at Original Position, independent of the target camera.
    pub fn anchor(&self) -> Point {
        self.anchor
    }

    /// Counts shown in Paste Options without resolving source files or a library catalog.
    pub fn counts(&self) -> (usize, usize, usize) {
        (
            self.project.components.len(),
            self.project.sheets.iter().map(|s| s.wires.len()).sum(),
            self.project.sheets.iter().map(|s| s.net_labels.len()).sum(),
        )
    }

    /// Build a cancellable placement candidate and its new selection. Copied connectivity remains
    /// separate from existing geometry except for explicitly retained named-net semantics.
    pub fn placed(&self, destination: &Project, at: Point, first_id: u64, options: &PasteOptions) -> Result<(Project, Selection), CommandError> {
        if self.has_hierarchy() {
            return self.placed_hierarchy(destination, at, first_id, options);
        }
        self.placed_with_parts(destination, at, first_id, options, &mut BTreeMap::new())
    }

    /// A hierarchy paste shares this remap across sheets so a multi-unit part is copied once.
    fn placed_with_parts(
        &self,
        destination: &Project,
        at: Point,
        first_id: u64,
        options: &PasteOptions,
        shared_parts: &mut BTreeMap<u64, u64>,
    ) -> Result<(Project, Selection), CommandError> {
        if options.reuse_sheets {
            return Err(invalid("flat fragments have no child sheets to relocate"));
        }

        if destination.all_symbols().count() + self.project.all_symbols().count() > crate::schematic::MAX_SYMBOLS
            || destination.sheets.iter().map(|s| s.wires.len()).sum::<usize>() + self.project.wires().len() > crate::schematic::wiring::MAX_WIRES
            || destination.sheets.iter().map(|s| s.net_labels.len()).sum::<usize>() + self.project.net_labels().len() > crate::net_labels::MAX_LABELS
        {
            return Err(invalid("pasted objects exceed project limits"));
        }
        if options.net_name_suffix.len() > 200 || options.net_name_suffix.chars().any(char::is_control) {
            return Err(invalid("invalid net-name suffix"));
        }
        let mut next = destination.clone();
        let mut selected = Selection::default();
        let mut cursor = first_id.max(destination.allocation_high_water().saturating_add(1));
        let mut components = BTreeMap::new();
        let mut symbols = BTreeMap::new();
        let mut junctions = BTreeMap::new();
        let mut labels = BTreeMap::new();
        let mut used_references: BTreeSet<_> = destination.components.values().map(|c| c.reference.clone()).collect();
        let mut next_references = BTreeMap::<String, usize>::new();
        let mut pinned = BTreeMap::new();
        for component in destination.components.values() {
            let prefix = component.definition.reference_prefix.clone();
            let maximum = next_references.entry(prefix).or_default();
            *maximum = (*maximum).max(component.reference_number);
            let d = &component.definition;
            pinned.insert((d.library.clone(), d.id.clone(), d.revision()), d);
        }
        let retained_parts: BTreeSet<_> = shared_parts.keys().copied().collect();
        for component in self.project.components.values() {
            if let Some(id) = shared_parts.get(&component.ordinal) {
                if !next.components.contains_key(id) {
                    return Err(invalid("missing shared pasted part"));
                }
                components.insert(component.ordinal, *id);
                continue;
            }
            let definition = &component.definition;
            if pinned.get(&(definition.library.clone(), definition.id.clone(), definition.revision())).is_some_and(|existing| *existing != definition)
            {
                return Err(invalid("copied symbol conflicts with a pinned definition of the same identity/revision"));
            }
            let id = allocate(&mut cursor)?;
            components.insert(component.ordinal, id);
            shared_parts.insert(component.ordinal, id);
            let mut copy = component.clone();
            copy.ordinal = id;
            if options.preserve_references {
                if !used_references.insert(copy.reference.clone()) {
                    return Err(invalid(format!("reference {} already exists", copy.reference)));
                }
            } else {
                let prefix = &copy.definition.reference_prefix;
                let number = next_references.entry(prefix.clone()).or_default();
                *number += 1;
                while used_references.contains(&format!("{prefix}{number}")) {
                    *number += 1;
                }
                if *number > crate::MAX_REFERENCE_NUMBER {
                    return Err(invalid("no unused references remain"));
                }
                copy.reference_number = *number;
                copy.reference = format!("{prefix}{number}");
                used_references.insert(copy.reference.clone());
            }
            if copy.definition.power_terminal().is_some() && !options.net_name_suffix.is_empty() {
                copy.value.push_str(&options.net_name_suffix);
                crate::schematic::check_value(&copy.value).map_err(invalid)?;
            }
            Arc::make_mut(&mut next.components).insert(id, copy);
        }
        for assignment in self.project.schematic_packages.assignments.values() {
            if retained_parts.contains(&assignment.component) {
                continue;
            }
            let mut copy = assignment.clone();
            copy.component = components[&assignment.component];
            let definition = self.project.assignment_definition(&copy.key()).ok_or_else(|| invalid("missing copied package"))?;
            if next.assignment_definition(&copy.key()).is_some_and(|existing| existing != definition) {
                return Err(invalid("copied footprint conflicts with a pinned definition of the same identity/revision"));
            }
            if next.board_target().is_some() {
                let board = Arc::make_mut(&mut next.board);
                board.definitions.insert(copy.key(), definition.clone());
                board.assignments.insert(copy.component, copy);
            } else {
                let packages = Arc::make_mut(&mut next.schematic_packages);
                packages.definitions.insert(copy.key(), definition.clone());
                packages.assignments.insert(copy.component, copy);
            }
        }
        let mut new_symbols = Vec::new();
        let mut new_junctions = BTreeMap::new();
        let mut new_labels = Vec::new();
        let mut new_wires = Vec::new();
        for source in self.project.symbols() {
            let mut copy = source.clone();
            copy.ordinal = allocate(&mut cursor)?;
            copy.component = components[&source.component];
            copy.position = transformed(source.position, self.anchor, at, options.quarter_turns)?;
            if copy.position.x_nm.abs() > crate::schematic::MAX_CENTER_NM || copy.position.y_nm.abs() > crate::schematic::MAX_CENTER_NM {
                return Err(invalid("pasted symbol exceeds sheet bounds"));
            }
            copy.rotation = (copy.rotation + options.quarter_turns % 4) % 4;
            symbols.insert(source.ordinal, copy.ordinal);
            selected.symbols.insert(copy.id());
            Arc::make_mut(&mut next.units).insert((copy.component, copy.unit.clone()), copy.ordinal);
            new_symbols.push(copy);
        }
        for (id, point) in self.project.junctions() {
            let fresh = allocate(&mut cursor)?;
            junctions.insert(id, fresh);
            new_junctions.insert(fresh, transformed(point, self.anchor, at, options.quarter_turns)?);
        }
        for source in self.project.net_labels() {
            let mut copy = source.clone();
            copy.id = allocate(&mut cursor)?;
            labels.insert(source.id, copy.id);
            copy.at = transformed(source.at, self.anchor, at, options.quarter_turns)?;
            copy.rotation = (copy.rotation + u16::from(options.quarter_turns % 4) * 90) % 360;
            copy.name.push_str(&options.net_name_suffix);
            selected.net_labels.insert(copy.object_id());
            new_labels.push(copy);
        }
        for source in self.project.wires() {
            let mut copy = source.clone();
            copy.ordinal = allocate(&mut cursor)?;
            for endpoint in &mut copy.endpoints {
                *endpoint = match *endpoint {
                    WireEndpoint::Terminal(pin) => {
                        let old = self.project.terminal_symbol(pin).ok_or_else(|| invalid("missing copied pin"))?;
                        WireEndpoint::Terminal(
                            Terminal::from_number(symbols[&pin.symbol], old.unit_definition(), pin.number())
                                .ok_or_else(|| invalid("missing copied terminal"))?,
                        )
                    }
                    WireEndpoint::Junction(id) => WireEndpoint::Junction(junctions[&id]),
                    WireEndpoint::NetLabel(id) => WireEndpoint::NetLabel(labels[&id]),
                    _ => return Err(invalid("unsupported hierarchical clipboard endpoint")),
                };
            }
            copy.bends = source
                .bends
                .as_ref()
                .map(|points| points.iter().map(|p| transformed(*p, self.anchor, at, options.quarter_turns)).collect())
                .transpose()?;
            selected.wires.insert(copy.id());
            new_wires.push(copy);
        }
        let sheet = next.sheet_mut();
        Arc::make_mut(&mut sheet.symbols).extend(new_symbols);
        Arc::make_mut(&mut sheet.junctions).extend(new_junctions);
        Arc::make_mut(&mut sheet.net_labels).extend(new_labels);
        Arc::make_mut(&mut sheet.wires).extend(new_wires);
        sheet.connectivity = Arc::default();
        next.numeric_id_high_water = Some(next.allocation_high_water().max(cursor.saturating_sub(1)));
        next.named_graph = Arc::default();
        Ok((next, selected))
    }
}

impl Session {
    /// Export a revision-bound selection through the same command used by GUI and MCP.
    pub(crate) fn copy_schematic(&self, params: &Value, cut: bool) -> Result<String, CommandError> {
        crate::check_params(crate::command(COPY).ok_or_else(|| invalid("missing copy command"))?, params)?;
        self.check_revision(params)?;
        let project = self.open_project()?;
        let project =
            project.for_sheet(project.resolve_sheet(params.get("sheet").and_then(Value::as_str))?).ok_or_else(|| invalid("missing sheet"))?;
        let items = Selection::parse(&params["items"]).ok_or_else(|| invalid("invalid selection"))?;
        let objects = hierarchy_objects(params.get("objects")).ok_or_else(|| invalid("invalid hierarchy object selection"))?;
        Fragment::capture_with_hierarchy(&project, &items, &objects, cut)?.to_text()
    }

    /// Cut only selected objects. Unselected incident routes remain at their old coordinates with
    /// shared free endpoints, so moving a component through the clipboard cannot delete other wiring.
    pub(crate) fn cut_schematic(&mut self, params: &Value) -> Result<(), CommandError> {
        let objects = hierarchy_objects(params.get("objects")).ok_or_else(|| invalid("invalid hierarchy object selection"))?;
        if !objects.is_empty() {
            return self.cut_schematic_hierarchy(params, &objects);
        }

        self.check_revision(params)?;
        let items = Selection::parse(&params["items"]).ok_or_else(|| invalid("invalid selection"))?;
        let project = self.open_project()?;
        Fragment::capture(project, &items)?;
        let removed_symbols: BTreeSet<_> = project.symbols().iter().filter(|s| items.symbols.contains(&s.id())).map(|s| s.ordinal).collect();
        let mut next = project.clone();
        let mut cursor = self.next_object_id()?;
        let mut detached = BTreeMap::new();
        for wire in Arc::make_mut(&mut next.sheet_mut().wires) {
            if items.wires.contains(&wire.id()) {
                continue;
            }
            for endpoint in &mut wire.endpoints {
                let remove = match *endpoint {
                    WireEndpoint::Terminal(pin) => removed_symbols.contains(&pin.symbol),
                    WireEndpoint::NetLabel(id) => items.net_labels.contains(&format!("net-label-{id}")),
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
        for (endpoint, id) in detached {
            Arc::make_mut(&mut next.sheet_mut().junctions)
                .insert(id, project.endpoint_position(endpoint).ok_or_else(|| invalid("missing endpoint"))?);
        }
        Arc::make_mut(&mut next.sheet_mut().wires).retain(|w| !items.wires.contains(&w.id()));
        Arc::make_mut(&mut next.sheet_mut().symbols).retain(|s| !items.symbols.contains(&s.id()));
        Arc::make_mut(&mut next.sheet_mut().net_labels).retain(|l| !items.net_labels.contains(&l.object_id()));
        Arc::make_mut(&mut next.units).retain(|_, s| !removed_symbols.contains(s));
        for component in project.symbols().iter().filter(|s| removed_symbols.contains(&s.ordinal)).map(|s| s.component) {
            if !next.all_symbols().any(|symbol| symbol.component == component) {
                next.retire_component(component);
            }
        }
        next.prune_junctions();
        next.numeric_id_high_water = Some(next.allocation_high_water().max(cursor.saturating_sub(1)));
        validate_candidate(&next)?;
        self.change(next);
        Ok(())
    }

    /// Validate and publish one complete paste against the captured revision; failed validation
    /// consumes neither IDs nor undo history. Native codecs check the whole resulting project.
    pub(crate) fn paste_schematic(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let fragment = Fragment::from_text(params["text"].as_str().unwrap_or_default())?;
        let options = PasteOptions {
            preserve_references: params["references"].as_str() == Some("preserve"),
            net_name_suffix: params["net_name_suffix"].as_str().unwrap_or_default().into(),
            quarter_turns: (params["rotation"].as_u64().unwrap_or(0) / 90) as u8,
            reuse_sheets: params["reuse_sheets"].as_bool().unwrap_or(false),
        };
        let at = Point { x_nm: params["x_nm"].as_i64().unwrap_or_default(), y_nm: params["y_nm"].as_i64().unwrap_or_default() };
        let (next, _) = fragment.placed(self.open_project()?, at, self.next_object_id()?, &options)?;
        validate_candidate(&next)?;
        self.change(next);
        Ok(())
    }
}

/// Reuse production validation, including pinned definitions, field text and cross-document links.
pub(crate) fn validate_candidate(project: &Project) -> Result<(), CommandError> {
    let documents = crate::persistence::encode_documents(Arc::new(project.clone())).map_err(invalid)?;
    crate::persistence::decode_documents(&documents).map_err(invalid)?;
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod hierarchy_tests;
