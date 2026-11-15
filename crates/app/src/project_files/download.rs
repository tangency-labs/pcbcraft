//! Incremental browser download preparation, testable without a browser or a download side effect.
use pcbcraft_engine::{
    Project,
    persistence::{Documents, DocumentsEncoder},
};
use std::sync::Arc;

/// Retain the raw saved snapshot alongside its original payloads, even after an opening migration.
pub(super) struct Original {
    snapshot: Arc<Project>,
    documents: Arc<Documents>,
}
impl Original {
    /// A baseline may preserve unchanged bytes after edits or migration, but never across projects.
    pub(super) fn for_project(snapshot: Arc<Project>, documents: Arc<Documents>, current: &Project) -> Option<Self> {
        snapshot.same_document(current).then_some(Self { snapshot, documents })
    }
    /// Raw documents can bypass encoding only for the actual unmodified source snapshot.
    pub(super) fn unchanged(&self, snapshot: &Arc<Project>) -> Option<Arc<Documents>> {
        Arc::ptr_eq(&self.snapshot, snapshot).then(|| self.documents.clone())
    }
}

/// Downloads add current folder/occurrence metadata to their copy without adopting editor state.
pub(super) fn needs_layout(snapshot: &Project) -> bool {
    snapshot.layout_root().is_none() || !snapshot.has_sheet_occurrences()
}

/// Compare old and current canonical encodings before reusing the old document's exact bytes.
/// Both encodings advance in bounded batches; changed references always keep their new encoding.
pub(super) struct Encoder {
    current: DocumentsEncoder,
    previous: Option<DocumentsEncoder>,
    canonical: Option<Documents>,
    original: Option<Arc<Documents>>,
    old_board: Option<String>,
    new_board: Option<String>,
}
impl Encoder {
    /// Prepare layout changes and retain an independent immutable baseline; no encoding or I/O yet.
    pub(super) fn new(snapshot: Arc<Project>, original: Option<Original>) -> Result<Self, String> {
        let snapshot = if needs_layout(&snapshot) {
            let prepared = if snapshot.layout_root().is_none() { snapshot.with_board_folders(".")? } else { Project::clone(&snapshot) };
            Arc::new(prepared.with_sheet_occurrences()?)
        } else {
            snapshot
        };
        Ok(Self {
            old_board: original.as_ref().and_then(|source| source.snapshot.board_document_path()),
            new_board: snapshot.board_document_path(),
            current: DocumentsEncoder::new(snapshot),
            previous: original.as_ref().map(|source| DocumentsEncoder::new(source.snapshot.clone())),
            original: original.map(|source| source.documents),
            canonical: None,
        })
    }

    /// Advance one encoder by at most `records`; return only the complete, checked document set.
    pub(super) fn step(&mut self, records: usize) -> Result<Option<Documents>, String> {
        if let Some(previous) = &mut self.previous {
            if let Some(canonical) = previous.step(records)? {
                self.canonical = Some(canonical);
                self.previous = None;
            }
            return Ok(None);
        }
        let Some(mut documents) = self.current.step(records)? else { return Ok(None) };
        if let (Some(canonical), Some(original)) = (&self.canonical, &self.original) {
            for (path, bytes) in &mut documents {
                let source = if Some(path.as_str()) == self.new_board.as_deref() { self.old_board.as_deref().unwrap_or(path) } else { path };
                if canonical.get(source) == Some(bytes)
                    && let Some(retained) = original.get(source)
                {
                    bytes.clone_from(retained);
                }
            }
        }
        Ok(Some(documents))
    }
}

#[cfg(test)]
mod tests;
