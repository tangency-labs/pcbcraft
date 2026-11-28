//! Load KiCad sheet sources and resolve occurrences before constructing a native project.

use super::MAX_TEXT_BYTES;
use crate::CommandError;
use pcbcraft_kicad::schematic::{PlacedSymbol, ReferenceSource, Schematic};
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
    if !symbol.instances.is_empty() && (!root || symbol.reference_from == ReferenceSource::OtherRecord) {
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
