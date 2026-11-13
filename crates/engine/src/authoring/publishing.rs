//! Filesystem-free publication preparation and durable-save adoption for native definitions.
use super::{Item, Kind, Session};
use crate::library::{self, sources::Documents};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;

/// A published address belongs to the authoring session, independently of its undo stack.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Address {
    pub library: String,
    pub id: String,
    pub revision: u32,
}

/// Validated immutable bytes plus the exact captured draft. Preparation never marks a save.
#[derive(Clone, Debug)]
pub struct Prepared {
    captured: Arc<Item>,
    published: Arc<Item>,
    manifest: String,
    latest: u32,
    documents: Documents,
    copy: bool,
}
impl Prepared {
    /// Documents contain only one portable native record, never an editor file or absolute path.
    pub fn documents(&self) -> &Documents {
        &self.documents
    }
    /// Exact manifest bytes captured by the host; a replaced destination must be reselected.
    pub fn manifest(&self) -> &str {
        &self.manifest
    }
    /// Highest known revision when the user confirmed publishing; hosts recheck before writing.
    pub fn latest(&self) -> u32 {
        self.latest
    }
    /// The validated target snapshot is adopted only after a host confirms durable publication.
    pub fn item(&self) -> &Arc<Item> {
        &self.published
    }
    /// Transport the prepared native documents to MCP without accepting filesystem destinations.
    pub fn inspect(&self) -> Value {
        json!({"address":{"library":self.published.library,"id":self.published.id,"revision":self.published.revision},
            "documents":self.documents,"saved":false,"copy":self.copy})
    }
}

/// Build a native manifest using existing identity/text validation; reserved destinations fail.
pub fn new_library(id: &str, name: &str, license: &str) -> Result<String, String> {
    let manifest = json!({"format":library::LIBRARY_FORMAT,"id":id,"name":name,"license":license});
    let mut text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    text.push('\n');
    destination(&text)?;
    Ok(text)
}

/// Return manifest identity after applying normal source validation and reserved-name policy.
pub fn destination(manifest: &str) -> Result<String, String> {
    let manifest = library::parse_manifest(manifest)?;
    if manifest.id == "user" || manifest.id.starts_with("project-") || matches!(manifest.id.as_str(), "starter" | "power") {
        return Err("Choose a writable personal library; user, project- and bundled identities are reserved.".into());
    }
    Ok(manifest.id)
}

/// Validate a captured draft and allocate an unused later revision without mutating the session.
/// An older base requires explicit consent; preparation neither merges changes nor writes files.
pub fn prepare(item: Arc<Item>, manifest: &str, latest: u32, base: Option<u32>, allow_older: bool) -> Result<Prepared, String> {
    item.encode()?;
    let library = destination(manifest)?;
    if item.library != "user" && item.library != library {
        return Err("Use Edit copy to publish this item into another library.".into());
    }
    if item.kind == Kind::Symbol && item.license != library::parse_manifest(manifest)?.license {
        return Err("A symbol inherits its library's license; choose a library matching this item's recorded license.".into());
    }
    if latest > 0 && base.is_none_or(|base| latest > base) && !allow_older {
        return Err(format!("Revision {latest} is available. Publishing this older edit may omit intervening changes; confirm before publishing."));
    }
    let mut published = (*item).clone();
    published.library = library;
    // Reserve a following content revision too, so successful adoption cannot exhaust the editor.
    let authored = if item.revision < item.allocation.revision {
        item.allocation.revision.checked_add(1).ok_or("Library revision limit reached.")?
    } else {
        item.revision
    };
    published.revision = authored.max(latest.checked_add(1).ok_or("Library revision limit reached.")?);
    if published.revision >= u32::MAX - 1 {
        return Err("Library revision limit reached.".into());
    }
    published.allocation.revision = published.allocation.revision.max(published.revision);
    published.validate()?;
    let mut native = published.native()?;
    if item.kind == Kind::Symbol {
        // Standalone symbols obtain library identity and attribution from their manifest.
        native.as_object_mut().ok_or("Invalid native symbol.")?.remove("library");
    }
    let directory = if item.kind == Kind::Symbol { "symbols" } else { "footprints" };
    let path = format!("{directory}/{}-r{}.json", published.id, published.revision);
    let mut text = serde_json::to_string_pretty(&native).map_err(|e| e.to_string())?;
    text.push('\n');
    let documents = Documents::from([(path, text)]);
    let mut complete = documents.clone();
    complete.insert("library.json".into(), manifest.to_owned());
    let parsed = library::sources::NativeSource::load(&complete)?;
    if !parsed.problems().is_empty() {
        return Err(parsed.problems().join("\n"));
    }
    Ok(Prepared { captured: item, published: Arc::new(published), manifest: manifest.into(), latest, documents, copy: false })
}

/// Prepare Save As from the current draft under a fresh item identity. Its editor is rebound
/// only after durable confirmation, while the original published records remain untouched.
pub fn prepare_copy(item: Arc<Item>, manifest: &str) -> Result<Prepared, String> {
    let mut copy = item.edit_copy()?;
    copy.name = item.name.clone();
    let mut prepared = prepare(Arc::new(copy), manifest, 0, None, false)?;
    prepared.captured = item;
    prepared.copy = true;
    Ok(prepared)
}

impl Item {
    /// Fork an editable draft under a fresh identity without changing the original or requiring
    /// a complete publishable drawing. Primitive and electrical identities remain local to the copy.
    pub fn edit_copy(&self) -> Result<Self, String> {
        let mut copy = self.clone();
        copy.id = Self::new(self.kind)?.id;
        copy.library = "user".into();
        copy.revision = 1;
        copy.allocation.revision = 1;
        copy.name = format!("{} copy", self.name).chars().take(64).collect();
        copy.source = format!("Fork of {}/{} r{}; {}", self.library, self.id, self.revision, self.source).chars().take(1024).collect();
        copy.validate()?;
        Ok(copy)
    }
}

impl Session {
    /// Bind a freshly reconstructed published item without changing its geometry or history.
    pub fn bind_published(&mut self) {
        self.published = Some(address(&self.item));
        self.publication_saved = Some(self.item.clone());
    }

    /// Last durably published address. Draft-file saves do not change publication identity.
    pub fn published(&self) -> Option<&Address> {
        self.published.as_ref()
    }

    /// A saved draft can still contain changes that have never been published to the catalog.
    pub fn publication_dirty(&self) -> bool {
        self.publication_saved.as_ref() != Some(&self.item)
    }

    /// Adopt host-confirmed publication across history. A newer draft keeps its geometry and
    /// remains dirty; Undo cannot turn a published item back into the user placeholder.
    pub fn confirm_publication(&mut self, prepared: &Prepared) {
        let unchanged = self.item == prepared.captured;
        let rebind = |item: &Arc<Item>| {
            let mut item = (**item).clone();
            item.library = prepared.published.library.clone();
            item.id = prepared.published.id.clone();
            if prepared.copy && item.source == prepared.captured.source {
                item.source = prepared.published.source.clone();
            }
            item.allocation.revision = item.allocation.revision.max(prepared.published.revision);
            Arc::new(item)
        };
        self.item = if unchanged { prepared.published.clone() } else { rebind(&self.item) };
        self.undo = self.undo.iter().map(&rebind).collect();
        self.redo = self.redo.iter().map(&rebind).collect();
        self.next_content_revision = self.next_content_revision.max(prepared.published.revision + 1);
        self.saved = Some(prepared.published.clone());
        self.publication_saved = self.saved.clone();
        self.published = Some(address(&prepared.published));
        if prepared.copy {
            // Save As changes the edited identity; fields/gestures captured for the original
            // remain available but cannot silently commit their metadata into the new item.
            self.revision = self.revision.saturating_add(1);
        }
    }
}

/// Capture one item's explicit portable address, never a source path or display name.
fn address(item: &Item) -> Address {
    Address { library: item.library.clone(), id: item.id.clone(), revision: item.revision }
}
