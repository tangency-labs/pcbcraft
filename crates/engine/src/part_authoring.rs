//! Session-only reusable part documents; publication reuses immutable native records.
use crate::{
    authoring::publishing::{self, Publication},
    board::definition::Source,
    library::{
        parts::{ManufacturerPart, PartDefinition, Reference},
        sources::{Catalog, Documents, Kind, MAX_FILE_BYTES, NativeSource},
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

/// Native records remain the authoring vocabulary; no persistent draft format is introduced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Record {
    Part(PartDefinition),
    Orderable(ManufacturerPart),
}
impl Record {
    /// Create a schematic-only generic draft with an exact, explicitly supplied symbol.
    pub fn new_part(symbol: Reference) -> Result<Self, String> {
        Ok(Self::Part(PartDefinition {
            format: crate::library::parts::PART_FORMAT.into(),
            library: "user".into(),
            id: format!("part-{}", new_identity()?),
            revision: 1,
            name: "New generic part".into(),
            license: "MIT OR Apache-2.0".into(),
            source: original_source(),
            symbol,
            category: None,
            fixed_parameters: BTreeMap::new(),
            packages: BTreeMap::new(),
        }))
    }
    /// Orderable drafts require a resolved published generic part and an explicit package.
    pub fn new_orderable(catalog: &Catalog, part: Reference) -> Result<Self, String> {
        let resolved = catalog.resolve_part(&part)?;
        let package = resolved.definition.packages.keys().next().ok_or("Publish a package before adding an orderable part.")?.clone();
        Ok(Self::Orderable(ManufacturerPart {
            format: crate::library::parts::MANUFACTURER_PART_FORMAT.into(),
            library: "user".into(),
            id: format!("mpn-{}", new_identity()?),
            revision: 1,
            name: "New orderable part".into(),
            license: resolved.definition.license.clone(),
            source: original_source(),
            manufacturer: String::new(),
            mpn: String::new(),
            part,
            package,
            datasheet: None,
            specifications: BTreeMap::new(),
        }))
    }
    /// Exact item identity is distinct from a runtime document revision.
    pub fn reference(&self) -> Reference {
        match self {
            Self::Part(part) => part.reference(),
            Self::Orderable(part) => part.reference(),
        }
    }
    /// Catalog kind determines the native directory and publication identity.
    pub fn kind(&self) -> Kind {
        match self {
            Self::Part(_) => Kind::Part,
            Self::Orderable(_) => Kind::ManufacturerPart,
        }
    }
    /// Human-readable metadata stays independent of native identity.
    pub fn name(&self) -> &str {
        match self {
            Self::Part(p) => &p.name,
            Self::Orderable(p) => &p.name,
        }
    }
    /// Rebinding changes only the exact publication address, never dependencies or geometry.
    fn rebind(&mut self, reference: &Reference) {
        match self {
            Self::Part(p) => {
                p.library = reference.library.clone();
                p.id = reference.id.clone();
                p.revision = reference.revision;
            }
            Self::Orderable(p) => {
                p.library = reference.library.clone();
                p.id = reference.id.clone();
                p.revision = reference.revision;
            }
        }
    }
    /// Complete validation resolves all declared packages and rejects unavailable/conflicting data.
    pub fn validate(&self, catalog: &Catalog) -> Result<(), String> {
        match self {
            Self::Part(p) => {
                if p.symbol.library == "user" && p.symbol.id == "choose-symbol" {
                    return Err("Choose a symbol before publishing.".into());
                }
                catalog.validate_part(Arc::new(p.clone()))?;
            }
            Self::Orderable(p) => p.validate_part(catalog.resolve_part(&p.part)?)?,
        }
        Ok(())
    }
    /// A derivative retains the original record's licence and portable provenance.
    pub fn copy(&self) -> Result<Self, String> {
        let mut copy = self.clone();
        let reference = self.reference();
        let address = Reference { library: "user".into(), id: format!("part-{}", new_identity()?), revision: 1 };
        copy.rebind(&address);
        let source = match &mut copy {
            Self::Part(p) => &mut p.source,
            Self::Orderable(p) => &mut p.source,
        };
        source.reference = format!("Fork of {}/{} r{}; {}", reference.library, reference.id, reference.revision, source.reference);
        if source.reference.len() > 2048 {
            let mut end = 2048;
            while !source.reference.is_char_boundary(end) {
                end -= 1;
            }
            source.reference.truncate(end);
        }
        Ok(copy)
    }
    /// Incomplete fields are allowed in bounded session drafts, never in a publication.
    fn bounded(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err("Part draft exceeds 1 MiB.".into());
        }
        match self {
            Self::Part(p) if p.packages.len() > 64 || p.fixed_parameters.len() > 64 => {
                return Err("A part supports at most 64 packages and fixed parameters.".into());
            }
            Self::Orderable(p) if p.specifications.len() > 64 => return Err("At most 64 specifications are supported.".into()),
            _ => {}
        }
        Ok(())
    }
}
/// Generate portable identities using the engine's already qualified randomness source.
fn new_identity() -> Result<uuid::Uuid, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(uuid::Builder::from_random_bytes(bytes).into_uuid())
}
/// New authored records carry original provenance rather than an invented provider address.
fn original_source() -> Source {
    Source { kind: "original".into(), reference: "Original PCBcraft part".into(), locator: "PCBcraft part document".into() }
}

/// A captured part document has its own history and never mutates a project or source.
#[derive(Clone, Debug)]
pub struct Session {
    record: Record,
    saved: Option<Record>,
    revision: u64,
    undo: Vec<Record>,
    redo: Vec<Record>,
}
impl Session {
    /// Admit a bounded draft or already published record without allocating a content revision.
    pub fn new(record: Record, published: bool) -> Result<Self, String> {
        record.bounded()?;
        Ok(Self { saved: published.then(|| record.clone()), record, revision: 1, undo: Vec::new(), redo: Vec::new() })
    }
    /// Current session draft, which may be incomplete until publication validation.
    pub fn record(&self) -> &Record {
        &self.record
    }
    /// Monotonic edit authority for GUI and MCP captures.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Whether the current native content differs from the last confirmed publication.
    pub fn dirty(&self) -> bool {
        self.saved.as_ref() != Some(&self.record)
    }
    /// Last durably confirmed immutable address, independent of draft edits.
    pub fn published(&self) -> Option<Reference> {
        self.saved.as_ref().map(Record::reference)
    }
    /// Replace one captured draft atomically. Stale clients and identity changes preserve history.
    pub fn replace(&mut self, expected: u64, record: Record) -> Result<(), String> {
        self.check(expected)?;
        if self.record.kind() != record.kind() || self.record.reference() != record.reference() {
            return Err("Part document identity cannot change during editing.".into());
        }
        record.bounded()?;
        if record == self.record {
            return Ok(());
        }
        self.advance()?;
        self.undo.push(std::mem::replace(&mut self.record, record));
        if self.undo.len() > 64 {
            self.undo.remove(0);
        }
        self.redo.clear();
        Ok(())
    }
    /// Undo and redo address this session, independently of project history.
    pub fn history(&mut self, expected: u64, redo: bool) -> Result<(), String> {
        self.check(expected)?;
        if if redo { self.redo.is_empty() } else { self.undo.is_empty() } {
            return Ok(());
        }
        self.advance()?;
        let (from, to) = if redo { (&mut self.redo, &mut self.undo) } else { (&mut self.undo, &mut self.redo) };
        if let Some(record) = from.pop() {
            to.push(std::mem::replace(&mut self.record, record));
        }
        Ok(())
    }
    /// Reject stale edit authority without changing draft content or history.
    fn check(&self, expected: u64) -> Result<(), String> {
        if expected == self.revision { Ok(()) } else { Err("Part document changed; inspect its current revision.".into()) }
    }
    /// Allocate edit authority before changing any draft or history entry.
    fn advance(&mut self) -> Result<(), String> {
        self.revision = self.revision.checked_add(1).ok_or("Part document revision exhausted.")?;
        Ok(())
    }
    /// Prepare a later immutable record after checking all dependencies, with explicit older-base consent.
    pub fn prepare(&self, catalog: &Catalog, manifest: &str, latest: u32, allow_older: bool, copy: bool) -> Result<Prepared, String> {
        let library = publishing::destination(manifest)?;
        let address = self.record.reference();
        if !copy && address.library == library && !self.dirty() {
            return Err("This revision is already published.".into());
        }
        prepare_record(&self.record, catalog, manifest, latest, self.published().map(|base| base.revision), allow_older, copy)
    }

    /// Durable completion rebinds history, while edits made during I/O remain unpublished.
    pub fn confirm(&mut self, prepared: &Prepared) {
        let unchanged = self.record == prepared.captured;
        let address = prepared.published.reference();
        let captured_source = match &prepared.captured {
            Record::Part(p) => &p.source,
            Record::Orderable(p) => &p.source,
        };
        let published_source = match &prepared.published {
            Record::Part(p) => &p.source,
            Record::Orderable(p) => &p.source,
        };
        for record in self.undo.iter_mut().chain(self.redo.iter_mut()).chain(std::iter::once(&mut self.record)) {
            record.rebind(&address);
            let source = match record {
                Record::Part(p) => &mut p.source,
                Record::Orderable(p) => &mut p.source,
            };
            if source == captured_source {
                *source = published_source.clone();
            }
        }
        if unchanged {
            self.record = prepared.published.clone();
        }
        self.saved = Some(prepared.published.clone());
        // Rebinding invalidates captures even when there were newer local field edits.
        self.revision = self.revision.saturating_add(1);
    }
    /// Inspect portable native content and document authority without host paths.
    pub fn inspect(&self) -> Value {
        json!({"record":self.record,"revision":self.revision,"dirty":self.dirty(),"published":self.published(),"can_undo":!self.undo.is_empty(),"can_redo":!self.redo.is_empty()})
    }
}
/// Validated bytes are private so callers cannot replace a prepared record after validation.
#[derive(Clone, Debug)]
pub struct Prepared {
    captured: Record,
    published: Record,
    documents: Documents,
    manifest: String,
    latest: u32,
}
impl Prepared {
    /// Current session draft, which may be incomplete until publication validation.
    /// Validated native content captured for durable publication.
    pub fn record(&self) -> &Record {
        &self.published
    }
    /// Return native documents for a host to write, without claiming a completed save.
    pub fn inspect(&self) -> Value {
        json!({"record":self.published,"documents":self.documents,"saved":false})
    }
}
impl Publication for Prepared {
    fn documents(&self) -> &Documents {
        &self.documents
    }
    fn manifest(&self) -> &str {
        &self.manifest
    }
    fn latest(&self) -> u32 {
        self.latest
    }
    fn reference(&self) -> Reference {
        self.published.reference()
    }
    fn kind(&self) -> Kind {
        self.published.kind()
    }
}

/// Filesystem-free preparation shared by session documents and headless MCP captures.
/// Every dependency resolves before bytes are returned; callers must still confirm a durable save.
pub fn prepare_record(
    record: &Record,
    catalog: &Catalog,
    manifest: &str,
    latest: u32,
    base: Option<u32>,
    allow_older: bool,
    copy: bool,
) -> Result<Prepared, String> {
    let library = publishing::destination(manifest)?;
    let current = record.reference();
    let copy = copy || (current.library != "user" && current.library != library);
    if !copy && latest > 0 && base.is_none_or(|base| latest > base) && !allow_older {
        return Err(format!("Revision {latest} is available; confirm publishing without merging."));
    }
    let mut published = if copy { record.copy()? } else { record.clone() };
    let next = if copy { 1 } else { latest.checked_add(1).ok_or("Library revision limit reached.")? };
    published.rebind(&Reference { library, id: published.reference().id, revision: next });
    published.validate(catalog)?;
    let (directory, text) = match &published {
        Record::Part(p) => ("parts", p.encode()?),
        Record::Orderable(p) => ("manufacturer-parts", p.encode()?),
    };
    let reference = published.reference();
    let documents = Documents::from([(format!("{directory}/{}-r{}.json", reference.id, reference.revision), text)]);
    let mut complete = documents.clone();
    complete.insert("library.json".into(), manifest.into());
    let source = NativeSource::load(&complete)?;
    if !source.problems().is_empty() {
        return Err(source.problems().join("\n"));
    }
    Ok(Prepared { captured: record.clone(), published, documents, manifest: manifest.into(), latest: if copy { 0 } else { latest } })
}

impl Record {
    /// Advance an exact symbol or footprint reference in a derived draft, preserving every
    /// mapping and authored value. Complete validation remains required before publication.
    pub fn advance_dependency(&self, kind: Kind, old: &Reference, new: &Reference) -> Result<Self, String> {
        if old.library != new.library || old.id != new.id || new.revision <= old.revision {
            return Err("Dependency update requires a later revision of the same item.".into());
        }
        let Self::Part(part) = self else {
            return Err("Only generic parts own drawing dependencies.".into());
        };
        let mut part = part.clone();
        let mut changed = false;
        if kind == Kind::Symbol && part.symbol == *old {
            part.symbol = new.clone();
            changed = true;
        }
        if kind == Kind::Footprint {
            for package in part.packages.values_mut() {
                if package.footprint == *old {
                    package.footprint = new.clone();
                    changed = true;
                }
            }
        }
        if !changed {
            return Err("The part does not reference this exact earlier definition.".into());
        }
        Ok(Self::Part(part))
    }
}

#[cfg(test)]
mod tests;

/// A native association field, addressed by meaning and stable package identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "field", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReferenceField {
    Symbol,
    Footprint { package: String },
    Category,
    Part,
}
impl ReferenceField {
    /// A field's catalog kind does not depend on its label or position in the form.
    pub fn kind(&self) -> Kind {
        match self {
            Self::Symbol => Kind::Symbol,
            Self::Footprint { .. } => Kind::Footprint,
            Self::Category => Kind::Category,
            Self::Part => Kind::Part,
        }
    }
}
impl Record {
    /// Resolve a field on the current draft; wrong document kinds and removed packages fail.
    pub fn reference_field(&self, field: &ReferenceField) -> Result<Option<Reference>, String> {
        match (self, field) {
            (Self::Part(p), ReferenceField::Symbol) => Ok(Some(p.symbol.clone())),
            (Self::Part(p), ReferenceField::Category) => Ok(p.category.as_ref().map(|c| c.definition.clone())),
            (Self::Part(p), ReferenceField::Footprint { package }) => {
                p.packages.get(package).map(|p| Some(p.footprint.clone())).ok_or("The package no longer exists.".into())
            }
            (Self::Orderable(p), ReferenceField::Part) => Ok(Some(p.part.clone())),
            _ => Err("This reference field does not belong to the part document.".into()),
        }
    }
    /// Validate an explicitly chosen exact definition, then clear only values owned by the
    /// changed association. Geometry/mappings are never inferred; publication validates the draft.
    pub fn select_reference(&self, field: &ReferenceField, reference: Option<Reference>, catalog: &Catalog) -> Result<Self, String> {
        let current = self.reference_field(field)?;
        if let Some(r) = &reference {
            let key = crate::library::sources::Key { library: r.library.clone(), id: r.id.clone(), revision: r.revision, kind: field.kind() };
            catalog.item(&key)?;
            match field {
                ReferenceField::Category => {
                    catalog.category(r)?;
                }
                ReferenceField::Part => {
                    catalog.resolve_part(r)?;
                }
                _ => {}
            }
        } else if !matches!(field, ReferenceField::Category) {
            return Err("Choose an exact library definition.".into());
        }
        if current == reference {
            return Ok(self.clone());
        }
        let mut candidate = self.clone();
        match (&mut candidate, field, reference) {
            (Self::Part(p), ReferenceField::Symbol, Some(r)) => p.symbol = r,
            (Self::Part(p), ReferenceField::Category, r) => {
                p.category = r.map(|definition| crate::library::parts::CategoryBinding { definition, primary_value: None });
                p.fixed_parameters.clear();
            }
            (Self::Part(p), ReferenceField::Footprint { package }, Some(r)) => {
                let package = p.packages.get_mut(package).ok_or("The package no longer exists.")?;
                package.footprint = r;
                package.mapping.clear();
                package.unconnected_pads.clear();
            }
            (Self::Orderable(p), ReferenceField::Part, Some(r)) => {
                p.part = r;
                p.package.clear();
                p.specifications.clear();
            }
            _ => return Err("This reference field does not belong to the part document.".into()),
        }
        candidate.bounded()?;
        Ok(candidate)
    }
}
