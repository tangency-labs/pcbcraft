//! Load KiCad sheet sources and resolve occurrences before constructing a native project.

use super::{Imported, MAX_TEXT_BYTES, Placed, Report, SheetNets, symbols, wiring};
use crate::hierarchy::{Direction, Label, ParentPin, PinEdge, SheetSymbol};
use crate::{CommandError, Point, Project, Session, WireEndpoint};
use pcbcraft_kicad::schematic::{LabelKind, PlacedSymbol, ReferenceSource, Schematic};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// An occurrence of a source sheet. A repeated subsheet shares its parsed source, but has its
/// own path, parent symbol and resolved references.
#[derive(Debug)]
pub struct Occurrence {
    pub file: String,
    pub instance_path: String,
    pub source: Arc<Schematic>,
    /// The parent occurrence index and the index of the sheet symbol in its source.
    pub parent: Option<(usize, usize)>,
    /// Reference and unit in source placement order, resolved for this occurrence.
    pub placements: Vec<(String, u32)>,
}

/// Load one root and its referenced sheets in parent-first order. The caller supplies source
/// text by normalized path relative to the project directory and must bound each read to the
/// remaining byte allowance. The loader enforces that allowance again before parsing.
pub fn load(root: &str, read: impl FnMut(&str, usize) -> Result<String, String>) -> Result<Vec<Occurrence>, CommandError> {
    let root = resolve("", root).ok_or_else(|| CommandError::InvalidParams("Invalid KiCad root schematic path.".into()))?;
    let mut loader =
        Loader { read, files: BTreeMap::new(), active: BTreeSet::new(), paths: BTreeSet::new(), occurrences: Vec::new(), remaining: MAX_TEXT_BYTES };
    let source = loader.source(&root)?;
    let root_path = source.uuid.as_ref().map_or_else(String::new, |id| format!("/{id}"));
    if root_path.is_empty() && !source.sheets.is_empty() {
        return Err(CommandError::InvalidParams(format!("{root}: the root with subsheets needs a sheet UUID.")));
    }
    loader.visit(root, root_path, None)?;
    Ok(loader.occurrences)
}

/// Build the native sheet tree and its interface anchors before placing symbols or routing
/// wires. The returned project's sheet order matches the source occurrences.
pub fn build_sheet_structure(occurrences: &[Occurrence], name: &str) -> Result<Project, CommandError> {
    let Some((root, children)) = occurrences.split_first() else {
        return Err(CommandError::InvalidParams("The KiCad hierarchy has no root sheet.".into()));
    };
    if root.parent.is_some() || occurrences.len() > crate::sheets::MAX_SHEETS {
        return Err(CommandError::InvalidParams("Invalid KiCad hierarchy root or sheet count.".into()));
    }
    let mut session = Session::default();
    let name = super::symbols::clip(name, 64).unwrap_or_else(|| "Imported".into());
    session.apply("project.new", &json!({"name":name}))?;
    let mut project = session.project().ok_or_else(|| CommandError::InvalidParams("Missing import project.".into()))?.clone();
    let mut names = BTreeSet::from([project.sheet_name().to_lowercase()]);
    let mut ordinal = project.highest_id();
    let mut allocate = || {
        ordinal += 1;
        ordinal
    };
    for (index, occurrence) in children.iter().enumerate() {
        let (parent, block_index) = occurrence
            .parent
            .filter(|(parent, _)| *parent <= index)
            .ok_or_else(|| CommandError::InvalidParams(format!("{}: missing parent of {}.", occurrence.file, occurrence.instance_path)))?;
        let block = occurrences[parent].source.sheets.get(block_index).ok_or_else(|| {
            CommandError::InvalidParams(format!("{}: missing parent sheet symbol for {}.", occurrence.file, occurrence.instance_path))
        })?;
        let at = native_point(block.at);
        let size = native_point(block.size);
        let fallback = occurrence.file.rsplit('/').next().unwrap_or("Sheet");
        let base = super::symbols::clip(&block.name, 60).or_else(|| super::symbols::clip(fallback, 60)).unwrap_or_else(|| "Sheet".into());
        let name = (1..=crate::sheets::MAX_SHEETS + 1)
            .map(|n| if n == 1 { base.clone() } else { format!("{base} ({n})") })
            .find(|name| names.insert(name.to_lowercase()))
            .ok_or_else(|| CommandError::InvalidParams(format!("{}: cannot assign a unique sheet name.", occurrence.file)))?;
        let id = crate::fresh_id()?;
        let mut child = project.new_sheet_record(id, name)?;
        let refused = |message| CommandError::InvalidParams(format!("{} ({}): {message}", occurrence.file, occurrence.instance_path));
        let mut ports = BTreeMap::<_, Vec<_>>::new();
        for label in occurrence.source.labels.iter().filter(|label| label.kind == LabelKind::Hierarchical) {
            ports.entry(label.name.as_str()).or_default().push(label);
        }
        let mut native_labels = Vec::new();
        let mut aliases = Vec::new();
        let mut assigned = BTreeSet::new();
        for (slot, pin) in block.pins.iter().enumerate() {
            let slot = u8::try_from(slot)
                .ok()
                .filter(|slot| usize::from(*slot) < crate::hierarchy::MAX_LABELS)
                .ok_or_else(|| refused("The sheet has too many interface pins.".into()))?;
            if !assigned.insert(pin.name.as_str()) {
                return Err(refused(format!("Duplicate parent pin {:?}.", pin.name)));
            }
            let Some((primary, extra)) =
                ports.remove(pin.name.as_str()).and_then(|labels| labels.split_first().map(|(first, rest)| (*first, rest.to_vec())))
            else {
                return Err(refused(format!("Parent pin {:?} has no matching hierarchical label.", pin.name)));
            };
            let parent_pin = parent_pin(at, size, native_point(pin.at))
                .ok_or_else(|| refused(format!("Pin {:?} is outside the sheet symbol border.", pin.name)))?;
            let direction = direction(pin.shape.as_deref().or(primary.shape.as_deref()))
                .ok_or_else(|| refused(format!("Unrecognized direction for parent pin {:?}.", pin.name)))?;
            native_labels.push(Label {
                id: allocate(),
                name: primary.name.clone(),
                direction,
                at: native_point(primary.at),
                slot,
                parent_pin: Some(parent_pin),
            });
            for alias in extra {
                // Repeated occurrences of a child interface name are additional local anchors.
                // The named graph binds these to the same child port rather than adding parent pins.
                aliases.push(crate::net_labels::Label {
                    id: allocate(),
                    name: alias.name.clone(),
                    style: crate::net_labels::Style::Local,
                    at: native_point(alias.at),
                    rotation: if alias.rotation_mdeg % 90_000 == 0 { (alias.rotation_mdeg / 1000) as u16 } else { 0 },
                    text: None,
                });
            }
        }
        if let Some((unmatched, _)) = ports.first_key_value() {
            return Err(refused(format!("Hierarchical label {unmatched:?} has no corresponding parent pin.")));
        }
        child.labels = Arc::new(native_labels);
        child.net_labels = Arc::new(aliases);
        let sheets = Arc::make_mut(&mut project.sheets);
        Arc::make_mut(&mut sheets[parent].subsheets).push(SheetSymbol { id: allocate(), child: id, at, size: Some(size) });
        sheets.push(child);
        let authored = project.sheets[parent].subsheets.last().ok_or_else(|| refused("Missing constructed sheet symbol.".into()))?;
        if project.sheet_symbol_size(authored) != size {
            return Err(refused("The sheet symbol is smaller than the native size limit.".into()));
        }
    }
    if root.source.labels.iter().any(|label| label.kind == LabelKind::Hierarchical) {
        return Err(CommandError::InvalidParams(format!("{}: the root sheet has hierarchical labels with no parent.", root.file)));
    }
    project.numeric_id_high_water = Some(ordinal);
    project.validate_hierarchy().map_err(CommandError::InvalidParams)?;
    project.validate_net_labels().map_err(CommandError::InvalidParams)?;
    Ok(project)
}

fn native_point(point: pcbcraft_kicad::Point) -> Point {
    Point { x_nm: point.x_nm, y_nm: point.y_nm }
}

fn parent_pin(at: Point, size: Point, pin: Point) -> Option<ParentPin> {
    let x = pin.x_nm.checked_sub(at.x_nm)?;
    let y = pin.y_nm.checked_sub(at.y_nm)?;
    if !(0..=size.x_nm).contains(&x) || !(0..=size.y_nm).contains(&y) {
        return None;
    }
    let (edge, offset_nm) = if x == 0 {
        (PinEdge::Left, y)
    } else if x == size.x_nm {
        (PinEdge::Right, y)
    } else if y == 0 {
        (PinEdge::Top, x)
    } else if y == size.y_nm {
        (PinEdge::Bottom, x)
    } else {
        return None;
    };
    Some(ParentPin { edge, offset_nm })
}

fn direction(shape: Option<&str>) -> Option<Direction> {
    match shape {
        Some("input") => Some(Direction::Input),
        Some("output") => Some(Direction::Output),
        Some("bidirectional") => Some(Direction::Bidirectional),
        Some("tri_state") => Some(Direction::TriState),
        None | Some("passive") => Some(Direction::Passive),
        _ => None,
    }
}

/// Convert the loaded occurrences and verify the complete terminal partition before returning
/// the native project.
pub fn import(occurrences: &[Occurrence], name: &str) -> Result<Imported, CommandError> {
    if let [root] = occurrences
        && root.source.sheets.is_empty()
    {
        return super::import_sheet((*root.source).clone(), name, None).map(|(imported, _)| imported);
    }
    let mut project = Arc::new(build_sheet_structure(occurrences, name)?);
    let sheets = project.sheets().map(|sheet| sheet.target).collect::<Vec<_>>();
    let mut report = Report::default();
    let mut source = Vec::new();
    for (occurrence, sheet) in occurrences.iter().zip(sheets) {
        let mut schematic = (*occurrence.source).clone();
        for (symbol, (reference, unit)) in schematic.symbols.iter_mut().zip(&occurrence.placements) {
            symbol.reference.clone_from(reference);
            symbol.unit = *unit;
            if symbol.reference_from == ReferenceSource::OtherRecord {
                symbol.reference_from = ReferenceSource::OwnRecord;
            }
        }
        let (imported, groups) = super::import_sheet(schematic, name, Some((project, sheet))).map_err(|error| {
            let message = format!("{} ({}): {error}", occurrence.file, occurrence.instance_path);
            match error {
                CommandError::CapExceeded(_) => CommandError::CapExceeded(message),
                _ => CommandError::InvalidParams(message),
            }
        })?;
        for (note, count) in imported.report.notes() {
            report.add(note, count);
        }
        project = imported.project;
        source.extend(groups);
    }
    verify_nets(source, &project)?;
    Ok(Imported { project, report })
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Contact {
    Terminal(u64, String),
    Interface(u64),
    Global(String),
    Power(String),
}

/// Resolve KiCad's wired components separately on each source sheet. Explicit interface IDs and
/// global names are the only keys that can connect nets across sheet boundaries.
pub(super) fn source_nets(
    schematic: &Schematic,
    segments: &[wiring::Segment],
    placed: &[Placed],
    converted: &BTreeMap<&str, symbols::Converted>,
    sheet: &Project,
    labels: &[(String, pcbcraft_kicad::Point)],
) -> Result<SheetNets, CommandError> {
    let mut contacts = wiring::Contacts { pins: Vec::new(), labels: Vec::new(), power: Vec::new() };
    let mut cuts: BTreeSet<_> = schematic.junctions.iter().map(|marker| marker.at).collect();
    for part in placed {
        let symbol = &schematic.symbols[part.source];
        let power = converted.get(symbol.lib_symbol.as_str()).and_then(|converted| converted.power_terminal.as_deref());
        for (number, point) in &part.pins {
            let key = Contact::Terminal(part.component, number.clone());
            if power == Some(number.as_str())
                && let Some(name) = symbol.property("Value")
            {
                let local = schematic.lib_symbols.get(&symbol.lib_symbol).is_some_and(|lib| lib.power_local);
                if local {
                    contacts.labels.push(("local power", name.to_owned(), *point));
                } else {
                    contacts.pins.push((Contact::Power(name.to_owned()), *point));
                }
            }
            contacts.pins.push((key, *point));
        }
    }
    for label in &schematic.labels {
        if label.kind == LabelKind::Global {
            contacts.pins.push((Contact::Global(label.name.clone()), label.at));
        } else {
            contacts.labels.push(("local", label.name.clone(), label.at));
            cuts.insert(label.at);
        }
    }
    for (end, point) in sheet.hierarchy_endpoints() {
        if let WireEndpoint::SheetPin(id) = end {
            contacts.pins.push((Contact::Interface(id), pcbcraft_kicad::Point { x_nm: point.x_nm, y_nm: point.y_nm }));
        }
    }
    for (name, point) in labels {
        let label = sheet
            .hierarchical_labels()
            .iter()
            .find(|label| label.name == *name)
            .ok_or_else(|| CommandError::InvalidParams(format!("Missing native port for {name:?}.")))?;
        contacts.pins.push((Contact::Interface(label.id), *point));
        contacts.labels.push(("local", name.clone(), *point));
    }
    cuts.extend(contacts.pins.iter().map(|(_, point)| *point));
    Ok(wiring::kicad_nets(&wiring::split(segments, &cuts), &contacts))
}

fn verify_nets(source: Vec<BTreeSet<Contact>>, project: &Project) -> Result<(), CommandError> {
    let mut neighbors = BTreeMap::<Contact, Vec<Contact>>::new();
    for group in source {
        if let Some(first) = group.first().cloned() {
            neighbors.entry(first.clone()).or_default();
            for contact in group.into_iter().skip(1) {
                neighbors.entry(first.clone()).or_default().push(contact.clone());
                neighbors.entry(contact).or_default().push(first.clone());
            }
        }
    }
    let mut expected = BTreeSet::new();
    while let Some(first) = neighbors.first_key_value().map(|(key, _)| key.clone()) {
        let mut pending = vec![first];
        let mut group = BTreeSet::new();
        while let Some(key) = pending.pop() {
            if let Some(next) = neighbors.remove(&key) {
                if let Contact::Terminal(part, number) = key {
                    group.insert((part, number));
                }
                pending.extend(next);
            }
        }
        if !group.is_empty() {
            expected.insert(group);
        }
    }
    let graph = project.electrical_connectivity();
    let mut connected = BTreeMap::<_, BTreeSet<_>>::new();
    let mut bare = BTreeSet::new();
    for symbol in project.all_symbols() {
        for terminal in symbol.terminals() {
            let key = (symbol.component, terminal.number().to_owned());
            if let Some(index) = graph.net_index_for(terminal) {
                connected.entry(index).or_default().insert(key);
            } else {
                bare.insert(key);
            }
        }
    }
    let grouped: BTreeSet<_> = connected.values().flatten().cloned().collect();
    let actual: BTreeSet<_> = connected.into_values().chain(bare.difference(&grouped).map(|key| BTreeSet::from([key.clone()]))).collect();
    if actual == expected {
        return Ok(());
    }
    let display = |group: &BTreeSet<(u64, String)>| {
        let mut labels: Vec<_> = group
            .iter()
            .take(8)
            .filter_map(|(part, pin)| project.components.get(part).map(|component| format!("{}.{}", component.reference(), pin)))
            .collect();
        if group.len() > 8 {
            labels.push(format!("and {} more", group.len() - 8));
        }
        labels.join(", ")
    };
    let detail = expected
        .difference(&actual)
        .next()
        .and_then(|group| {
            let first = group.first()?;
            let found = actual.iter().find(|other| other.contains(first))?;
            Some(format!(" (source: {}; native: {})", display(group), display(found)))
        })
        .unwrap_or_default();
    Err(CommandError::InvalidParams(format!("The imported hierarchy's terminal connections differ from the KiCad sources{detail}.")))
}

struct Loader<F> {
    read: F,
    files: BTreeMap<String, Arc<Schematic>>,
    active: BTreeSet<String>,
    paths: BTreeSet<String>,
    occurrences: Vec<Occurrence>,
    remaining: usize,
}

impl<F: FnMut(&str, usize) -> Result<String, String>> Loader<F> {
    fn source(&mut self, file: &str) -> Result<Arc<Schematic>, CommandError> {
        if let Some(source) = self.files.get(file) {
            return Ok(Arc::clone(source));
        }
        let text = (self.read)(file, self.remaining).map_err(|error| CommandError::InvalidParams(format!("{file}: {error}")))?;
        self.remaining = self
            .remaining
            .checked_sub(text.len())
            .ok_or_else(|| CommandError::CapExceeded(format!("{file}: the KiCad hierarchy exceeds {} MiB of source text.", MAX_TEXT_BYTES >> 20)))?;
        let source = Arc::new(pcbcraft_kicad::read_schematic(&text).map_err(|error| CommandError::InvalidParams(format!("{file}: {error}")))?);
        if let Some((reason, _)) = source.lost.iter().next() {
            return Err(CommandError::InvalidParams(format!("{file}: {reason}.")));
        }
        if source.symbols.len() > super::MAX_SYMBOLS {
            return Err(CommandError::CapExceeded(format!("{file}: more than {} placed symbols.", super::MAX_SYMBOLS)));
        }
        self.files.insert(file.to_owned(), Arc::clone(&source));
        Ok(source)
    }

    fn visit(&mut self, file: String, path: String, parent: Option<(usize, usize)>) -> Result<(), CommandError> {
        if !self.active.insert(file.clone()) {
            return Err(CommandError::InvalidParams(format!("{file}: a sheet cycle reaches this file again at {path}.")));
        }
        if !self.paths.insert(path.clone()) {
            return Err(CommandError::InvalidParams(format!("{file}: duplicate sheet occurrence {path}.")));
        }
        let index = self.occurrences.len();
        if index >= crate::sheets::MAX_SHEETS {
            return Err(CommandError::CapExceeded(format!("{file}: the hierarchy exceeds {} sheet occurrences.", crate::sheets::MAX_SHEETS)));
        }
        let source = self.source(&file)?;
        let placements = source
            .symbols
            .iter()
            .map(|symbol| placement(symbol, &path, parent.is_none()))
            .collect::<Result<_, _>>()
            .map_err(|error| CommandError::InvalidParams(format!("{file} ({path}): {error}")))?;
        self.occurrences.push(Occurrence { file: file.clone(), instance_path: path.clone(), source: Arc::clone(&source), parent, placements });
        for (sheet, child) in source.sheets.iter().enumerate() {
            let child_file = resolve(&file, &child.file)
                .ok_or_else(|| CommandError::InvalidParams(format!("{file}: invalid child path {:?} in sheet {:?}.", child.file, child.name)))?;
            let id = child.uuid.as_deref().ok_or_else(|| {
                CommandError::InvalidParams(format!("{file}: sheet {:?} pointing to {child_file} has no sheet symbol UUID.", child.name))
            })?;
            self.visit(child_file, format!("{path}/{id}"), Some((index, sheet)))?;
        }
        self.active.remove(&file);
        Ok(())
    }
}

/// Select the reference for the actual sheet occurrence, never another project's first record.
fn placement(symbol: &PlacedSymbol, path: &str, root: bool) -> Result<(String, u32), String> {
    let occurrence_record = |record: &&pcbcraft_kicad::schematic::Instance| record.path == path;
    let mut matches = symbol.instances.iter().filter(occurrence_record);
    if let Some(first) = matches.next() {
        let unit = first.unit.unwrap_or(symbol.declared_unit);
        if matches.any(|other| other.reference != first.reference || other.unit.unwrap_or(symbol.declared_unit) != unit) {
            return Err(format!("conflicting instance records for {}.", symbol.lib_symbol));
        }
        return Ok((first.reference.clone(), unit));
    }
    if !root && !symbol.instances.is_empty() {
        return Err(format!("no matching instance record for {}.", symbol.lib_symbol));
    }
    Ok((symbol.reference.clone(), symbol.unit))
}

/// Resolve slash-delimited paths relative to the parent source's directory. A child may use
/// ".." within the project directory. Keep absolute paths and drive prefixes out of the map.
fn resolve(parent: &str, child: &str) -> Option<String> {
    if child.is_empty() || child.starts_with('/') || child.contains(['\\', ':', '\0']) {
        return None;
    }
    let mut parts: Vec<_> = parent.rsplit_once('/').map_or_else(Vec::new, |(folder, _)| folder.split('/').collect());
    for part in child.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

#[cfg(test)]
mod tests;
