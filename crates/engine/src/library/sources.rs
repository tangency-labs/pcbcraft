//! Validated native source snapshots and a disposable catalog, independent of project history.
use super::parts::{Category, ManufacturerPart, PartDefinition, Reference};
use super::{parse_manifest, parse_symbol, saved_copy};
use crate::{Command, CommandError, Param, ParamKind, Session, board::definition::FootprintDefinition, schematic::SymbolDefinition};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};

/// Maximum loaded source registrations in one session, including bundled native libraries.
pub const MAX_SOURCES: usize = 18;
/// Maximum files in a source, including its manifest.
pub const MAX_FILES: usize = 256;
/// Maximum UTF-8 bytes in one native file.
pub const MAX_FILE_BYTES: usize = 1 << 20;
/// Maximum UTF-8 bytes in one source snapshot.
pub const MAX_SOURCE_BYTES: usize = 8 << 20;
/// Native relative paths and UTF-8 file contents; filesystem adapters never enter the engine.
pub type Documents = BTreeMap<String, String>;
/// Install a bounded native snapshot without editing any pinned project definitions.
pub const LOAD: &str = "library.load_source";
/// Remove an external source from browsing; bundled sources and pinned definitions remain intact.
pub const REMOVE: &str = "library.remove_source";
/// Inspect loaded sources, provenance and per-file validation/conflict diagnostics.
pub const LIST: &str = "library.list_sources";
/// Search the latest unambiguous revision of each native record kind across sources.
pub const SEARCH: &str = "library.search";
/// Resolve a part's exact dependencies and validate its typed values and complete package maps.
pub const RESOLVE_PART: &str = "library.resolve_part";
/// Normalize typed quantity input into the canonical base-unit representation.
pub const PARSE_QUANTITY: &str = "library.parse_quantity";

/// Accept missing legacy attribution only when every definition field otherwise matches.
/// Complete source definitions still compare their full content, including provenance.
pub(crate) fn matches_pinned(pinned: &SymbolDefinition, selected: &SymbolDefinition) -> bool {
    if pinned == selected {
        return true;
    }
    if pinned.provenance.is_some() {
        return false;
    }
    let body = |symbol: &SymbolDefinition| {
        serde_json::to_value(saved_copy(symbol)).map(|mut value| {
            if let Some(object) = value.as_object_mut() {
                object.remove("provenance");
            }
            value
        })
    };
    body(pinned).ok().zip(body(selected).ok()).is_some_and(|(a, b)| a == b)
}

/// Hash all definition data and portable attribution, regardless of its delivery adapter.
pub fn symbol_digest(symbol: &SymbolDefinition) -> Result<String, String> {
    serde_json::to_value(saved_copy(symbol)).map(|value| format!("{:x}", Sha256::digest(value.to_string().as_bytes()))).map_err(|e| e.to_string())
}

const SOURCE: Param = Param {
    name: "source",
    summary: "Personal source ID, independent of the library ID or folder path",
    required: true,
    kind: ParamKind::LibrarySource,
};
pub(crate) const COMMANDS: [Command; 6] = [
    Command {
        id: LOAD,
        title: "Load library source",
        summary: "Load native UTF-8 documents into the session catalog. No project change or undo step.",
        params: &[
            SOURCE,
            Param {
                name: "documents",
                summary: "library.json and flat symbols, footprints, categories and parts JSON documents",
                required: true,
                kind: ParamKind::LibraryDocuments,
            },
        ],
    },
    Command {
        id: REMOVE,
        title: "Remove library source",
        summary: "Remove an external source from the catalog, preserving bundled sources and pinned project copies.",
        params: &[SOURCE],
    },
    Command {
        id: LIST,
        title: "List library sources",
        summary: "Inspect source identities, counts and validation/conflict diagnostics.",
        params: &[],
    },
    Command {
        id: SEARCH,
        title: "Search libraries",
        summary: "Search current native symbols and footprints, excluding conflicting identities.",
        params: &[
            Param {
                name: "query",
                summary: "Search name, item/library ID, symbol prefix or manufacturer/MPN",
                required: false,
                kind: ParamKind::LibraryQuery,
            },
            Param {
                name: "kind",
                summary: "symbol, footprint, category, part or manufacturer_part; omit for all",
                required: false,
                kind: ParamKind::LibraryKind,
            },
        ],
    },
    Command {
        id: RESOLVE_PART,
        title: "Resolve native part",
        summary: "Inspect one exact part and validate all dependencies without editing a project or fetching sources.",
        params: &[
            Param { name: "library", summary: "Exact library identity", required: true, kind: ParamKind::LibrarySource },
            Param { name: "definition", summary: "Exact native part identity", required: true, kind: ParamKind::LibrarySource },
            Param { name: "definition_revision", summary: "Pinned part revision", required: true, kind: ParamKind::DefinitionRevision },
        ],
    },
    Command {
        id: PARSE_QUANTITY,
        title: "Normalize typed quantity",
        summary: "Convert unit-aware input to an exact canonical base-unit value, without editing a project.",
        params: &[
            Param {
                name: "quantity",
                summary: "Declared physical quantity from the category field",
                required: true,
                kind: ParamKind::LibraryQuantity,
            },
            Param {
                name: "value",
                summary: "Decimal input with an optional compatible unit or SI prefix",
                required: true,
                kind: ParamKind::LibraryQuery,
            },
        ],
    },
];

/// Attribution pinned with an external symbol. Personal source IDs and machine paths stay out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub library_name: String,
    pub license: String,
}
impl Provenance {
    /// Match the manifest's single-line bounds before adopting a saved attribution record.
    pub(super) fn validate(&self) -> Result<(), String> {
        super::check_text("library name", &self.library_name, 64)?;
        super::check_text("license", &self.license, 128)
    }
}

/// Item kind is part of identity: a symbol and a footprint may share an item ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Symbol,
    Footprint,
    Category,
    Part,
    ManufacturerPart,
}

/// An exact native item revision, independent of source registration and file location.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Key {
    pub library: String,
    pub kind: Kind,
    pub id: String,
    pub revision: u32,
}

#[derive(Clone, Debug)]
enum Definition {
    Symbol(Arc<SymbolDefinition>),
    Footprint(Arc<FootprintDefinition>),
    Category(Arc<Category>),
    Part(Arc<PartDefinition>),
    ManufacturerPart(Arc<ManufacturerPart>),
}

/// A validated catalog item with portable provenance and its source-relative diagnostic path.
#[derive(Clone, Debug)]
pub struct Item {
    pub key: Key,
    pub name: String,
    pub license: String,
    pub file: String,
    definition: Definition,
    canonical: Value,
}
impl Item {
    /// Inspect explicit identities and provenance without embedding large geometry in search results.
    pub fn inspect(&self) -> Value {
        let mut value = json!({"key":self.key,"name":self.name,"license":self.license,"file":self.file,
            "digest":format!("{:x}",Sha256::digest(self.canonical.to_string().as_bytes())),
            "format":self.canonical["format"]});
        if let Definition::Symbol(symbol) = &self.definition {
            value["units"] = json!(symbol.units.iter().map(|unit| unit.unit_id()).collect::<Vec<_>>());
        }
        if let Some(record) = self.manufacturer_part() {
            value["manufacturer_part"] = json!(record.as_ref());
        }
        value
    }
    /// Return symbol geometry for preview or validated placement; footprints return None.
    pub fn symbol(&self) -> Option<Arc<SymbolDefinition>> {
        match &self.definition {
            Definition::Symbol(symbol) => Some(symbol.clone()),
            _ => None,
        }
    }
    /// Return validated footprint geometry for inspection; assignment remains a separate workflow.
    pub fn footprint(&self) -> Option<Arc<FootprintDefinition>> {
        match &self.definition {
            Definition::Footprint(footprint) => Some(footprint.clone()),
            _ => None,
        }
    }
    /// Inspect a parsed generic part without assuming that its dependencies are available.
    pub fn part(&self) -> Option<Arc<PartDefinition>> {
        match &self.definition {
            Definition::Part(part) => Some(part.clone()),
            _ => None,
        }
    }
    /// Exact manufacturer metadata remains a library record, independent of supplier integrations.
    pub fn manufacturer_part(&self) -> Option<Arc<ManufacturerPart>> {
        match &self.definition {
            Definition::ManufacturerPart(record) => Some(record.clone()),
            _ => None,
        }
    }
    /// Inspect a parsed typed category without changing source or project state.
    pub fn category(&self) -> Option<Arc<Category>> {
        match &self.definition {
            Definition::Category(category) => Some(category.clone()),
            _ => None,
        }
    }
    /// Search stable identity, display name and symbol prefix, without interpreting names as IDs.
    fn matches(&self, query: &str) -> bool {
        self.name.to_lowercase().contains(query)
            || self.key.id.contains(query)
            || self.key.library.contains(query)
            || self
                .manufacturer_part()
                .is_some_and(|part| part.manufacturer.to_lowercase().contains(query) || part.mpn.to_lowercase().contains(query))
            || self.symbol().is_some_and(|symbol| symbol.reference_prefix().eq_ignore_ascii_case(query))
    }
}

/// Immutable parsed source. Invalid files are diagnosed individually; an invalid manifest fails the load.
#[derive(Clone, Debug)]
pub struct NativeSource {
    library: String,
    name: String,
    license: String,
    items: Vec<Item>,
    problems: Vec<String>,
}
impl NativeSource {
    /// Validated native items, retaining exact revisions and source-relative diagnostic paths.
    pub fn items(&self) -> &[Item] {
        &self.items
    }
    /// Per-file schema and adapter diagnostics; catalog conflicts are added during inspection.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }
    /// Add bounded read/encoding diagnostics from an adapter without changing any parsed item.
    pub fn add_adapter_problems(&mut self, problems: impl IntoIterator<Item = String>) {
        let available = MAX_FILES.saturating_sub(self.problems.len());
        self.problems.extend(problems.into_iter().take(available));
    }
    /// Validate all bounds before parsing. Unknown paths are refused rather than silently omitted.
    /// Duplicate identities remain available to the catalog for conflict detection and quarantine.
    pub fn load(documents: &Documents) -> Result<Self, String> {
        check_documents(documents)?;
        let manifest =
            parse_manifest(documents.get("library.json").ok_or("library.json: missing manifest")?).map_err(|e| format!("library.json: {e}"))?;
        let mut source = Self { library: manifest.id, name: manifest.name, license: manifest.license, items: Vec::new(), problems: Vec::new() };
        for (path, text) in documents.iter().filter(|(path, _)| path.as_str() != "library.json") {
            match source.parse_item(path, text) {
                Ok(item) => source.items.push(item),
                Err(error) => source.problems.push(format!("{path}: {error}")),
            }
        }
        Ok(source)
    }

    /// Read a browser-uploadable bundle using the same bounded document loader as desktop folders.
    pub fn from_bundle(bytes: &[u8]) -> Result<Self, String> {
        Self::load(&Self::bundle_documents(bytes)?)
    }

    /// Validate the transport and path/size limits while retaining original native text for installation.
    pub fn bundle_documents(bytes: &[u8]) -> Result<Documents, String> {
        // JSON string escaping can expand source text; bound the transport separately too.
        if bytes.len() > MAX_SOURCE_BYTES * 2 {
            return Err("Library bundle exceeds 16 MiB".into());
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Bundle {
            format: String,
            #[serde(deserialize_with = "bounded_documents")]
            documents: Documents,
        }
        let bundle: Bundle = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if bundle.format != "pcbcraft-library-bundle/1" {
            return Err("Unsupported library bundle format".into());
        }
        check_documents(&bundle.documents)?;
        Self::load(&bundle.documents)?;
        Ok(bundle.documents)
    }

    /// Parse an individual native item and bind its identity to the manifest, never its filename.
    fn parse_item(&self, file: &str, text: &str) -> Result<Item, String> {
        let (definition, key, name, license, canonical) = if file.starts_with("symbols/") {
            let mut symbol = parse_symbol(text)?;
            symbol.library = self.library.clone();
            symbol.provenance = Some(Provenance { library_name: self.name.clone(), license: self.license.clone() });
            let canonical = serde_json::to_value(saved_copy(&symbol)).map_err(|e| e.to_string())?;
            let key = Key { library: self.library.clone(), kind: Kind::Symbol, id: symbol.id.clone(), revision: symbol.revision() };
            let name = symbol.name.clone();
            (Definition::Symbol(Arc::new(symbol)), key, name, self.license.clone(), canonical)
        } else if file.starts_with("footprints/") {
            let footprint = crate::board::definition::read_saved(text.as_bytes())?;
            if footprint.library != self.library {
                return Err("Footprint library does not match library.json".into());
            }
            let key = Key { library: self.library.clone(), kind: Kind::Footprint, id: footprint.id.clone(), revision: footprint.revision };
            let canonical = serde_json::to_value(footprint.as_ref()).map_err(|e| e.to_string())?;
            (Definition::Footprint(footprint.clone()), key, footprint.name.clone(), footprint.license.clone(), canonical)
        } else if file.starts_with("parts/") {
            let part = Arc::new(PartDefinition::read(text)?);
            if part.library != self.library {
                return Err("Part library does not match library.json".into());
            }
            let key = Key { library: part.library.clone(), kind: Kind::Part, id: part.id.clone(), revision: part.revision };
            let canonical = serde_json::to_value(part.as_ref()).map_err(|e| e.to_string())?;
            (Definition::Part(part.clone()), key, part.name.clone(), part.license.clone(), canonical)
        } else if file.starts_with("manufacturer-parts/") {
            let record = Arc::new(ManufacturerPart::read(text)?);
            if record.library != self.library {
                return Err("Manufacturer-part library does not match library.json".into());
            }
            let key = Key { library: record.library.clone(), kind: Kind::ManufacturerPart, id: record.id.clone(), revision: record.revision };
            let canonical = serde_json::to_value(record.as_ref()).map_err(|error| error.to_string())?;
            (Definition::ManufacturerPart(record.clone()), key, record.name.clone(), record.license.clone(), canonical)
        } else {
            let category = Arc::new(Category::read(text)?);
            if category.library != self.library {
                return Err("Category library does not match library.json".into());
            }
            let key = Key { library: category.library.clone(), kind: Kind::Category, id: category.id.clone(), revision: category.revision };
            let canonical = serde_json::to_value(category.as_ref()).map_err(|e| e.to_string())?;
            (Definition::Category(category.clone()), key, category.name.clone(), category.license.clone(), canonical)
        };
        Ok(Item { key, name, license, file: file.into(), definition, canonical })
    }
}

/// Bound the map during deserialization rather than allocating an entire oversized index first.
/// Duplicate paths are rejected instead of silently choosing the last supplied file.
fn bounded_documents<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Documents, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = Documents;
        /// Describe the source map expected by the bundle decoder.
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("bounded native library documents")
        }
        /// Reject count/path collisions before reading another value; bound text before retaining it.
        fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Documents, M::Error> {
            let mut documents = Documents::new();
            let mut total = 0;
            while let Some(path) = map.next_key::<String>()? {
                if documents.len() >= MAX_FILES || !is_document_path(&path) || documents.contains_key(&path) {
                    return Err(serde::de::Error::custom("Too many, duplicate or unsupported library paths"));
                }
                let text = map.next_value::<String>()?;
                total += text.len();
                if text.len() > MAX_FILE_BYTES || total > MAX_SOURCE_BYTES {
                    return Err(serde::de::Error::custom("Library content exceeds its per-file or aggregate bounds"));
                }
                documents.insert(path, text);
            }
            Ok(documents)
        }
    }
    deserializer.deserialize_map(Visitor)
}

/// Validate transported document bounds before cloning JSON values for native parsing.
pub(crate) fn valid_documents_value(value: &Value) -> bool {
    value.as_object().is_some_and(|files| {
        files.len() <= MAX_FILES
            && files.iter().all(|(path, text)| is_document_path(path) && text.as_str().is_some_and(|text| text.len() <= MAX_FILE_BYTES))
            && files.values().filter_map(Value::as_str).map(str::len).sum::<usize>() <= MAX_SOURCE_BYTES
    })
}

/// Only flat JSON item directories are supported; no traversal, nested trees or arbitrary files.
pub fn is_document_path(path: &str) -> bool {
    if path == "library.json" {
        return true;
    }
    let Some((directory, name)) = path.split_once('/') else { return false };
    matches!(directory, "symbols" | "footprints" | "categories" | "parts" | "manufacturer-parts")
        && name.ends_with(".json")
        && name.len() <= 128
        && !name.starts_with('.')
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

/// Bound the complete source before allocating parsed definitions, on GUI and MCP paths alike.
pub fn check_documents(documents: &Documents) -> Result<(), String> {
    if documents.len() > MAX_FILES {
        return Err(format!("A library source has at most {MAX_FILES} files"));
    }
    let mut total = 0;
    for (path, text) in documents {
        if !is_document_path(path) {
            return Err(format!("Unsupported library path {path:?}"));
        }
        if text.len() > MAX_FILE_BYTES {
            return Err(format!("{path}: exceeds 1 MiB"));
        }
        total += text.len();
        if total > MAX_SOURCE_BYTES {
            return Err("Library source exceeds 8 MiB".into());
        }
    }
    Ok(())
}

/// A session-local index. Refresh/removal cannot edit a project or its undo/redo snapshots.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    generation: u64,
    sources: BTreeMap<String, Arc<NativeSource>>,
    /// Trusted delivery origin, assigned only by the embedded-source adapter.
    builtin: BTreeSet<String>,
    unavailable: BTreeSet<String>,
    /// Keep changed revisions quarantined for this registration until it is removed.
    changed: BTreeMap<String, BTreeSet<Key>>,
    conflicts: BTreeSet<Key>,
    /// Exact available definitions are indexed once per source generation.
    index: BTreeMap<Key, (String, usize)>,
    field_conflicts: BTreeSet<String>,
    /// Read-only resolution is cached by exact identity and invalidated on every catalog mutation.
    resolved_parts: BTreeMap<Key, OnceLock<Result<ResolvedPart, String>>>,
}

/// One validated library snapshot of a part and all definitions it requires; no project mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPart {
    pub definition: Arc<PartDefinition>,
    pub symbol: Arc<SymbolDefinition>,
    pub category: Option<Arc<Category>>,
    pub footprints: BTreeMap<String, Arc<FootprintDefinition>>,
}
impl ResolvedPart {
    /// Expose exact native documents for shared GUI/MCP inspection without fetching or changing them.
    fn snapshot(&self) -> Value {
        let footprints: BTreeMap<_, _> = self.footprints.iter().map(|(id, definition)| (id, definition.as_ref())).collect();
        json!({"part":self.definition.as_ref(),"symbol":saved_copy(&self.symbol),"category":self.category.as_deref(),"footprints":footprints})
    }
    /// Return immutable native documents and a digest covering the complete dependency closure.
    pub fn inspect(&self) -> Value {
        let mut value = self.snapshot();
        value["digest"] = json!(part_digest(self));
        value
    }
}
/// Fingerprint exact dependency content, so refreshing a category or footprint invalidates a pending placement.
pub fn part_digest(part: &ResolvedPart) -> String {
    format!("{:x}", Sha256::digest(part.snapshot().to_string().as_bytes()))
}
/// Manufacturer selections pin supplied specification content in addition to the generic-part closure.
pub fn manufacturer_digest(part: &ManufacturerPart) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_value(part).unwrap_or(Value::Null).to_string().as_bytes()))
}
impl Catalog {
    /// Resolve an exact item, refusing missing, conflicting and unavailable definitions without fallback.
    fn exact(&self, reference: &Reference, kind: Kind) -> Result<&Item, String> {
        reference.validate()?;
        let key = Key { library: reference.library.clone(), kind, id: reference.id.clone(), revision: reference.revision };
        if self.conflicts.contains(&key) {
            return Err(format!("Conflicting {kind:?} {}/{} r{}.", reference.library, reference.id, reference.revision));
        }
        self.index
            .get(&key)
            .and_then(|(source, index)| self.sources.get(source).and_then(|source| source.items.get(*index)))
            .ok_or_else(|| format!("Unavailable {kind:?} {}/{} r{}.", reference.library, reference.id, reference.revision))
    }

    /// Resolve a category and reject conflicting meanings for shared namespaced field identities.
    pub fn category(&self, reference: &Reference) -> Result<Arc<Category>, String> {
        let category = self.exact(reference, Kind::Category)?.category().ok_or("Expected a category record.")?;
        if let Some(id) = category.fields.keys().find(|id| self.field_conflicts.contains(*id)) {
            return Err(format!("Conflicting type definitions for category field {id}."));
        }
        Ok(category)
    }

    /// Resolve an exact unambiguous footprint for native assignment without substituting a newer revision.
    pub fn footprint(&self, library: &str, id: &str, revision: u32) -> Option<Arc<FootprintDefinition>> {
        self.exact(&Reference { library: library.into(), id: id.into(), revision }, Kind::Footprint).ok()?.footprint()
    }
    /// Resolve native manufacturer metadata and its exact part/package/category dependencies.
    pub fn manufacturer_part(&self, reference: &Reference) -> Result<Arc<ManufacturerPart>, String> {
        let record = self.exact(reference, Kind::ManufacturerPart)?.manufacturer_part().ok_or("Expected a manufacturer-part record.")?;
        record.validate_part(self.resolve_part(&record.part)?)?;
        Ok(record)
    }
    /// List resolvable manufacturer records belonging to one exact reusable part revision.
    pub fn manufacturers_for(&self, part: &Reference) -> Vec<Arc<ManufacturerPart>> {
        self.search("", Some(Kind::ManufacturerPart))
            .into_iter()
            .filter_map(|(_, item)| {
                let record = item.manufacturer_part()?;
                (record.part == *part).then(|| self.manufacturer_part(&record.reference()).ok()).flatten()
            })
            .collect()
    }
    /// Validate a part against exact currently available dependencies, including every package option.
    /// The operation is read-only, performs no network I/O, and never substitutes a newer revision.
    pub fn resolve_part(&self, reference: &Reference) -> Result<&ResolvedPart, String> {
        reference.validate()?;
        let key = Key { library: reference.library.clone(), kind: Kind::Part, id: reference.id.clone(), revision: reference.revision };
        let cache = self
            .resolved_parts
            .get(&key)
            .ok_or_else(|| format!("Unavailable Part {}/{} r{}.", reference.library, reference.id, reference.revision))?;
        cache.get_or_init(|| self.resolve_part_uncached(reference)).as_ref().map_err(Clone::clone)
    }

    /// Resolve only direct symbol/category/footprint dependencies, so cache initialization cannot recurse.
    fn resolve_part_uncached(&self, reference: &Reference) -> Result<ResolvedPart, String> {
        let part = self.exact(reference, Kind::Part)?.part().ok_or("Expected a part record.")?;
        let symbol = self.exact(&part.symbol, Kind::Symbol)?.symbol().ok_or("Expected a symbol record.")?;
        let category = part.category.as_ref().map(|binding| self.category(&binding.definition)).transpose()?;
        part.validate_category(category.as_deref())?;
        let mut footprints = BTreeMap::new();
        for (id, package) in &part.packages {
            let footprint = self.exact(&package.footprint, Kind::Footprint)?.footprint().ok_or("Expected a footprint record.")?;
            package.validate_definitions(&symbol, &footprint).map_err(|error| format!("Package {id}: {error}"))?;
            footprints.insert(id.clone(), footprint);
        }
        Ok(ResolvedPart { definition: part, symbol, category, footprints })
    }
    /// Install read-only embedded registrations through the ordinary validator and conflict index.
    /// Broken bundled manifests remain visible as unavailable registrations rather than hiding
    /// another library. This adapter has no project or history ownership.
    pub fn bundled() -> Self {
        let mut catalog = Self::default();
        for (id, documents) in super::bundled_documents() {
            let source = NativeSource::load(&documents).unwrap_or_else(|error| NativeSource {
                library: id.into(),
                name: id.into(),
                license: String::new(),
                items: Vec::new(),
                problems: vec![error],
            });
            // Embedded registrations are bounded by this repository's delivery adapter.
            let source_id = format!("bundled-{id}");
            catalog.builtin.insert(source_id.clone());
            catalog.sources.insert(source_id, Arc::new(source));
        }
        catalog.rebuild_conflicts();
        catalog.generation = 0;
        catalog
    }
    /// Resolve the latest unambiguous native symbol through the same index as an exact revision.
    pub fn latest_symbol(&self, library: &str, id: &str) -> Option<Arc<SymbolDefinition>> {
        self.search("", Some(Kind::Symbol))
            .into_iter()
            .find(|(_, item)| item.key.library == library && item.key.id == id)
            .and_then(|(_, item)| item.symbol())
    }
    /// A session-local load/removal generation for rejecting delayed background source results.
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// Refuse mutations to registrations owned by embedded delivery before changing catalog state.
    fn check_writable(&self, id: &str) -> Result<(), String> {
        if self.builtin.contains(id) { Err("Built-in library sources are read-only".into()) } else { Ok(()) }
    }
    /// Atomically replace a source snapshot. Reusing an immutable revision with changed content
    /// quarantines it rather than silently replacing old geometry. Bundled registrations are
    /// read-only; external removal and re-adding is explicit.
    pub fn install(&mut self, id: &str, source: Arc<NativeSource>) -> Result<(), String> {
        self.check_writable(id)?;
        if !crate::board::definition::valid_id(id) {
            return Err("Invalid or reserved source ID".into());
        }
        if !self.sources.contains_key(id) && self.sources.len() >= MAX_SOURCES {
            return Err("At most 18 native sources may be loaded".into());
        }
        let mut changed = self.changed.get(id).cloned().unwrap_or_default();
        if let Some(previous) = self.sources.get(id) {
            for item in &source.items {
                if previous.items.iter().any(|old| old.key == item.key && old.canonical != item.canonical) {
                    changed.insert(item.key.clone());
                }
            }
        }
        if changed.len() > MAX_FILES {
            return Err("Too many conflicting revisions in this source registration; correct the source or remove it".into());
        }
        self.changed.insert(id.into(), changed);
        self.sources.insert(id.into(), source);
        self.unavailable.remove(id);
        self.rebuild_conflicts();
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }
    /// Remove external disposable source state, refusing unknown and read-only bundled IDs.
    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        self.check_writable(id)?;
        if self.sources.remove(id).is_none() {
            return Err("Unknown native source ID".into());
        }
        self.changed.remove(id);
        self.unavailable.remove(id);
        self.rebuild_conflicts();
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }
    /// Stop offering a failed external source while retaining its prior revision identities for
    /// conflict checks. Bundled registrations cannot be disabled by a failed external refresh.
    pub fn mark_unavailable(&mut self, id: &str) {
        if !self.builtin.contains(id) && self.sources.contains_key(id) && self.unavailable.insert(id.into()) {
            self.rebuild_conflicts();
            self.generation = self.generation.wrapping_add(1);
        }
    }
    /// Rebuild immutable-revision conflicts, exact addresses and shared field meanings after a
    /// source change. Drop all cached dependency outcomes so missing/conflicting peers can recover.
    fn rebuild_conflicts(&mut self) {
        let mut known: BTreeMap<&Key, &Value> = BTreeMap::new();
        let mut conflicts: BTreeSet<_> = self.changed.values().flatten().cloned().collect();
        for item in self.sources.values().flat_map(|source| &source.items) {
            if let Some(previous) = known.insert(&item.key, &item.canonical)
                && previous != &item.canonical
            {
                conflicts.insert(item.key.clone());
            }
        }
        self.conflicts = conflicts;
        self.index.clear();
        self.field_conflicts.clear();
        let mut fields: BTreeMap<String, super::parts::FieldType> = BTreeMap::new();
        for (source_id, source) in self.sources.iter().filter(|(id, _)| !self.unavailable.contains(*id)) {
            for (index, item) in source.items.iter().enumerate() {
                self.index.entry(item.key.clone()).or_insert_with(|| (source_id.clone(), index));
                if self.conflicts.contains(&item.key) {
                    continue;
                }
                if let Some(category) = item.category() {
                    for (id, field) in &category.fields {
                        if let Some(previous) = fields.get(id) {
                            if !previous.same_meaning(&field.value_type) {
                                self.field_conflicts.insert(id.clone());
                            }
                        } else {
                            fields.insert(id.clone(), field.value_type.clone());
                        }
                    }
                }
            }
        }
        self.resolved_parts = self.index.keys().filter(|key| key.kind == Kind::Part).map(|key| (key.clone(), OnceLock::new())).collect();
    }
    /// Search the latest revision of each native identity, omitting all copies of conflicting keys.
    /// Identical copies appear once; their source registrations remain separately inspectable.
    pub fn search(&self, query: &str, kind: Option<Kind>) -> Vec<(&str, &Item)> {
        let query = query.trim().to_lowercase();
        let conflicts = &self.conflicts;
        let mut latest: BTreeMap<(&str, Kind, &str), (&str, &Item)> = BTreeMap::new();
        for (source, item) in self
            .sources
            .iter()
            .filter(|(id, _)| !self.unavailable.contains(*id))
            .flat_map(|(id, source)| source.items.iter().map(move |item| (id.as_str(), item)))
        {
            if kind.is_some_and(|kind| kind != item.key.kind) {
                continue;
            }
            let key = (item.key.library.as_str(), item.key.kind, item.key.id.as_str());
            if latest.get(&key).is_none_or(|(_, old)| item.key.revision > old.key.revision) {
                latest.insert(key, (source, item));
            }
        }
        latest.into_values().filter(|(_, item)| !conflicts.contains(&item.key) && item.matches(&query)).collect()
    }
    /// Resolve an exact unambiguous symbol for placement; a stale catalog selection fails explicitly.
    pub fn symbol(&self, library: &str, id: &str, revision: u32) -> Option<Arc<SymbolDefinition>> {
        let conflicts = &self.conflicts;
        self.sources
            .iter()
            .filter(|(id, _)| !self.unavailable.contains(*id))
            .flat_map(|(_, source)| &source.items)
            .find(|item| {
                item.key.library == library
                    && item.key.id == id
                    && item.key.revision == revision
                    && item.key.kind == Kind::Symbol
                    && !conflicts.contains(&item.key)
            })
            .and_then(Item::symbol)
    }
    /// Inspect every revision from one available registration, including quarantined items.
    /// Browsing does not select placement candidates or silently substitute another source.
    pub fn source_items(&self, source: &str) -> &[Item] {
        if self.unavailable.contains(source) {
            return &[];
        }
        self.sources.get(source).map_or(&[], |source| source.items.as_slice())
    }

    /// Explain whether an exact item is quarantined; previews may show it but cannot place it.
    pub fn conflicting(&self, key: &Key) -> bool {
        self.conflicts.contains(key)
    }

    /// Refuse a redundant installation only when every revision already has identical content
    /// in available registrations. Conflicting or empty collections are never called redundant.
    pub fn contains_source(&self, source: &NativeSource) -> bool {
        !source.items.is_empty()
            && source.items.iter().all(|item| {
                !self.conflicts.contains(&item.key)
                    && self
                        .sources
                        .iter()
                        .filter(|(id, _)| !self.unavailable.contains(*id))
                        .any(|(_, existing)| existing.items.iter().any(|other| other.key == item.key && other.canonical == item.canonical))
            })
    }

    /// List sources with trusted built-in status and validation/conflict messages by filename.
    pub fn inspect(&self) -> Value {
        let conflicts = &self.conflicts;
        let mut sources = Vec::new();
        sources.extend(self.sources.iter().map(|(id,source)| {
            let mut problems = source.problems.clone();
            for item in &source.items {
                if conflicts.contains(&item.key) { problems.push(format!("{}: conflicting {:?} {}/{} revision {}; use a new revision",item.file,item.key.kind,item.key.library,item.key.id,item.key.revision)); }
                if let Some(part) = item.part()
                    && let Err(error) = self.resolve_part(&part.reference())
                {
                    problems.push(format!("{}: {error}",item.file));
                }
                if let Some(record) = item.manufacturer_part() && let Err(error) = self.manufacturer_part(&record.reference()) {
                    problems.push(format!("{}: {error}",item.file));
                }
                if let Some(symbol) = item.symbol()
                    && let Some(binding) = &symbol.category
                    && let Err(error) = self.category(&binding.definition).and_then(|category| binding.validate_category(&category))
                {
                    problems.push(format!("{}: {error}",item.file));
                }
            }
            json!({"source":id,"builtin":self.builtin.contains(id),"library":source.library,"name":source.name,"license":source.license,"available":!self.unavailable.contains(id),"problems":problems,
                "symbols":source.items.iter().filter(|i|i.key.kind==Kind::Symbol).count(),"footprints":source.items.iter().filter(|i|i.key.kind==Kind::Footprint).count(),
                "manufacturer_parts":source.items.iter().filter(|i|i.key.kind==Kind::ManufacturerPart).count(),
                "parts":source.items.iter().filter(|i|i.key.kind==Kind::Part).count(),"categories":source.items.iter().filter(|i|i.key.kind==Kind::Category).count()})
        }));
        json!(sources)
    }
}

impl Session {
    /// Share the disposable library catalog with presentation, independently of the open project.
    pub fn libraries(&self) -> &Catalog {
        &self.libraries
    }

    /// Adopt a source parsed by a background filesystem worker through the same catalog transaction
    /// as library.load_source. This never changes project revision, dirty state or undo history.
    pub fn install_library_source(&mut self, id: &str, source: Arc<NativeSource>) -> Result<(), CommandError> {
        Arc::make_mut(&mut self.libraries).install(id, source).map_err(CommandError::InvalidParams)
    }

    /// Mark a filesystem source unavailable after a failed refresh, preserving pinned projects and
    /// cached revision identity checks while removing that source's items from search/placement.
    pub fn mark_library_unavailable(&mut self, id: &str) {
        Arc::make_mut(&mut self.libraries).mark_unavailable(id);
    }

    /// Execute validated source commands, returning library inspection/search rather than project data.
    pub(crate) fn library_command(&mut self, id: &str, params: &Value) -> Result<Value, CommandError> {
        let command = crate::command(id).ok_or_else(|| CommandError::Unknown(id.into()))?;
        crate::check_params(command, params)?;
        match id {
            LOAD => {
                let documents: Documents =
                    serde_json::from_value(params["documents"].clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
                let source = NativeSource::load(&documents).map_err(CommandError::InvalidParams)?;
                self.install_library_source(params["source"].as_str().unwrap_or_default(), Arc::new(source))?;
                Ok(self.libraries.inspect())
            }
            REMOVE => {
                Arc::make_mut(&mut self.libraries).remove(params["source"].as_str().unwrap_or_default()).map_err(CommandError::InvalidParams)?;
                Ok(self.libraries.inspect())
            }
            LIST => Ok(self.libraries.inspect()),
            PARSE_QUANTITY => {
                let quantity: super::parts::Quantity =
                    serde_json::from_value(params["quantity"].clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?;
                let value = quantity
                    .parse(params["value"].as_str().unwrap_or_default())
                    .map(super::parts::Value::from)
                    .map_err(CommandError::InvalidParams)?;
                serde_json::to_value(value).map_err(|error| CommandError::InvalidParams(error.to_string()))
            }
            RESOLVE_PART => {
                let reference = Reference {
                    library: params["library"].as_str().unwrap_or_default().into(),
                    id: params["definition"].as_str().unwrap_or_default().into(),
                    revision: params["definition_revision"]
                        .as_u64()
                        .and_then(|revision| u32::try_from(revision).ok())
                        .ok_or_else(|| CommandError::InvalidParams("Invalid part revision.".into()))?,
                };
                self.libraries.resolve_part(&reference).map(|part| part.inspect()).map_err(CommandError::InvalidParams)
            }
            SEARCH => {
                let query = params["query"].as_str().unwrap_or_default().trim().to_lowercase();
                let kind = params
                    .get("kind")
                    .map(|kind| serde_json::from_value::<Kind>(kind.clone()))
                    .transpose()
                    .map_err(|e| CommandError::InvalidParams(e.to_string()))?;
                let items: Vec<_> = self
                    .libraries
                    .search(&query, kind)
                    .into_iter()
                    .map(|(source, item)| {
                        let mut value = item.inspect();
                        value["source"] = json!(source);
                        if let Some(part) = item.part()
                            && let Ok(resolved) = self.libraries.resolve_part(&part.reference())
                        {
                            value["digest"] = json!(part_digest(resolved));
                        }
                        if let Some(part) = item.part() {
                            value["resolution"] = match self.libraries.resolve_part(&part.reference()) {
                                Ok(_) => json!({"status":"resolved"}),
                                Err(error) => json!({"status":"unresolved","reason":error}),
                            };
                        }
                        value
                    })
                    .collect();
                Ok(json!(items))
            }
            _ => Err(CommandError::Unknown(id.into())),
        }
    }
}

#[cfg(test)]
mod tests;
