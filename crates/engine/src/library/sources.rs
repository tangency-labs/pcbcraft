//! Validated native source snapshots and a disposable catalog, independent of project history.
use super::{parse_manifest, parse_symbol, saved_copy, starter};
use crate::{Command, CommandError, Param, ParamKind, Session, board::definition::FootprintDefinition, schematic::SymbolDefinition};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Maximum source registrations in one session, excluding the embedded starter library.
pub const MAX_SOURCES: usize = 16;
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
/// Remove a source from browsing; already pinned project definitions remain intact.
pub const REMOVE: &str = "library.remove_source";
/// Inspect loaded sources, provenance and per-file validation/conflict diagnostics.
pub const LIST: &str = "library.list_sources";
/// Search the latest unambiguous revision of symbols and footprints across sources.
pub const SEARCH: &str = "library.search";

/// Hash a validated symbol's complete pinned content, including portable license attribution.
/// Placement compares this with the search/preview content so a refreshed source cannot substitute
/// different geometry while leaving the electrical project revision unchanged.
pub fn symbol_digest(symbol: &SymbolDefinition) -> Result<String, String> {
    serde_json::to_value(saved_copy(symbol)).map(|value| format!("{:x}", Sha256::digest(value.to_string().as_bytes()))).map_err(|e| e.to_string())
}

const SOURCE: Param = Param {
    name: "source",
    summary: "Personal source ID, independent of the library ID or folder path",
    required: true,
    kind: ParamKind::LibrarySource,
};
pub(crate) const COMMANDS: [Command; 4] = [
    Command {
        id: LOAD,
        title: "Load library source",
        summary: "Load native UTF-8 documents into the session catalog. No project change or undo step.",
        params: &[
            SOURCE,
            Param {
                name: "documents",
                summary: "library.json and symbols/*.json or footprints/*.json, as path-to-text entries",
                required: true,
                kind: ParamKind::LibraryDocuments,
            },
        ],
    },
    Command {
        id: REMOVE,
        title: "Remove library source",
        summary: "Remove a source from the catalog, preserving pinned project copies.",
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
                summary: "Search name, item/library ID or symbol reference prefix",
                required: false,
                kind: ParamKind::LibraryQuery,
            },
            Param { name: "kind", summary: "symbol or footprint; omit for both", required: false, kind: ParamKind::LibraryKind },
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
        json!({"key":self.key,"name":self.name,"license":self.license,"file":self.file,
            "digest":format!("{:x}",Sha256::digest(self.canonical.to_string().as_bytes())),
            "format":match self.key.kind {Kind::Symbol=>super::SYMBOL_FORMAT,Kind::Footprint=>"pcbcraft-footprint/1"}})
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
    /// Search stable identity, display name and symbol prefix, without interpreting names as IDs.
    fn matches(&self, query: &str) -> bool {
        self.name.to_lowercase().contains(query)
            || self.key.id.contains(query)
            || self.key.library.contains(query)
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
        if manifest.id == "starter" {
            return Err("The starter library identity is reserved for the embedded source".into());
        }
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
        Self::load(&bundle.documents)
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
        } else {
            let footprint = crate::board::definition::read_saved(text.as_bytes())?;
            if footprint.library != self.library {
                return Err("Footprint library does not match library.json".into());
            }
            let key = Key { library: self.library.clone(), kind: Kind::Footprint, id: footprint.id.clone(), revision: footprint.revision };
            let canonical = serde_json::to_value(footprint.as_ref()).map_err(|e| e.to_string())?;
            (Definition::Footprint(footprint.clone()), key, footprint.name.clone(), footprint.license.clone(), canonical)
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
    matches!(directory, "symbols" | "footprints")
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
    unavailable: BTreeSet<String>,
    /// Keep changed revisions quarantined for this registration until it is removed.
    changed: BTreeMap<String, BTreeSet<Key>>,
    conflicts: BTreeSet<Key>,
}
impl Catalog {
    /// A session-local load/removal generation for rejecting delayed background source results.
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// Atomically replace a source snapshot. Reusing an immutable revision with changed content
    /// quarantines it rather than silently replacing old geometry. Removing then re-adding is explicit.
    pub fn install(&mut self, id: &str, source: Arc<NativeSource>) -> Result<(), String> {
        if !crate::board::definition::valid_id(id) || id == "builtin" {
            return Err("Invalid or reserved source ID".into());
        }
        if !self.sources.contains_key(id) && self.sources.len() >= MAX_SOURCES {
            return Err("At most 16 native sources may be loaded".into());
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
    /// Remove only the named registration's disposable source state, refusing unknown IDs.
    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        if self.sources.remove(id).is_none() {
            return Err("Unknown native source ID".into());
        }
        self.changed.remove(id);
        self.unavailable.remove(id);
        self.rebuild_conflicts();
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }
    /// Stop offering a failed native source while retaining its prior revision identities for
    /// conflict checks on refresh. Explicit removal forgets this disposable registration state.
    pub fn mark_unavailable(&mut self, id: &str) {
        if self.sources.contains_key(id) && self.unavailable.insert(id.into()) {
            self.generation = self.generation.wrapping_add(1);
        }
    }
    /// Identify every conflicting key, including collisions within one source and refresh mutations.
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
    /// List sources with validation and conflict messages; filenames identify invalid items.
    pub fn inspect(&self) -> Value {
        let conflicts = &self.conflicts;
        let mut sources = vec![
            json!({"source":"builtin","library":starter().id,"name":starter().name,"license":starter().license,"problems":starter().problems(),"symbols":starter().symbols().len(),"footprints":crate::board::definition::STARTER_IDS.len()}),
        ];
        sources.extend(self.sources.iter().map(|(id,source)| {
            let mut problems = source.problems.clone();
            for item in &source.items {
                if conflicts.contains(&item.key) { problems.push(format!("{}: conflicting {:?} {}/{} revision {}; use a new revision",item.file,item.key.kind,item.key.library,item.key.id,item.key.revision)); }
            }
            json!({"source":id,"library":source.library,"name":source.name,"license":source.license,"available":!self.unavailable.contains(id),"problems":problems,
                "symbols":source.items.iter().filter(|i|i.key.kind==Kind::Symbol).count(),"footprints":source.items.iter().filter(|i|i.key.kind==Kind::Footprint).count()})
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
            SEARCH => {
                let query = params["query"].as_str().unwrap_or_default().trim().to_lowercase();
                let kind = params
                    .get("kind")
                    .map(|kind| serde_json::from_value::<Kind>(kind.clone()))
                    .transpose()
                    .map_err(|e| CommandError::InvalidParams(e.to_string()))?;
                let mut items: Vec<_> = self
                    .libraries
                    .search(&query, kind)
                    .into_iter()
                    .map(|(source, item)| {
                        let mut value = item.inspect();
                        value["source"] = json!(source);
                        value
                    })
                    .collect();
                if kind != Some(Kind::Footprint) {
                    for symbol in starter().symbols().iter().filter(|symbol| {
                        query.is_empty()
                            || symbol.name.to_lowercase().contains(&query.to_lowercase())
                            || symbol.id.contains(&query.to_lowercase())
                            || symbol.library.contains(&query)
                            || symbol.reference_prefix().eq_ignore_ascii_case(&query)
                    }) {
                        items.push(json!({"source":"builtin","key":{"library":symbol.library,"kind":"symbol","id":symbol.id,"revision":symbol.revision()},"name":symbol.name,"license":starter().license,"format":super::SYMBOL_FORMAT}));
                    }
                }
                if kind != Some(Kind::Symbol) {
                    for footprint in
                        crate::board::definition::STARTER_IDS.into_iter().filter_map(crate::board::definition::starter).filter(|footprint| {
                            query.is_empty()
                                || footprint.name.to_lowercase().contains(&query.to_lowercase())
                                || footprint.id.contains(&query.to_lowercase())
                                || footprint.library.contains(&query)
                        })
                    {
                        items.push(json!({"source":"builtin","key":{"library":footprint.library,"kind":"footprint","id":footprint.id,"revision":footprint.revision},"name":footprint.name,"license":footprint.license,"format":footprint.format}));
                    }
                }
                Ok(json!(items))
            }
            _ => Err(CommandError::Unknown(id.into())),
        }
    }
}

#[cfg(test)]
mod tests;
