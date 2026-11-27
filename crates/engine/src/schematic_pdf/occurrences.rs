//! Read order and parent page indices for the export, independent of PDF encoding.

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use crate::{Project, instances::SheetOccurrence, sheets::SheetInfo};

use super::PdfError;

/// One page and, for a child, the parent page and stable sheet-symbol ID on that page.
pub(super) struct Visit<'a> {
    pub(super) sheet: SheetInfo<'a>,
    pub(super) parent: Option<(usize, u64)>,
}

/// Traverse the occurrence forest in preorder. Output indices refer to pages, not to unique
/// definitions. Fail if a record cannot be reached or has no project sheet to draw.
pub(super) fn ordered<'a>(project: &'a Project, records: &[SheetOccurrence]) -> Result<Vec<Visit<'a>>, PdfError> {
    let definitions: BTreeMap<_, _> = project.sheets().enumerate().map(|(index, info)| (info.target.id(), (index, info))).collect();
    let mut ids = BTreeSet::new();
    let mut roots = Vec::new();
    let mut children = BTreeMap::<Uuid, Vec<_>>::new();
    for occurrence in records {
        if !ids.insert(occurrence.id) {
            return Err(PdfError::InvalidHierarchy);
        }
        let &(index, info) = definitions.get(&occurrence.definition).ok_or(PdfError::UnknownSheet)?;
        match (occurrence.parent, occurrence.parent_slot) {
            (None, None) => roots.push((index, occurrence.id, info)),
            (Some(parent), Some(slot)) => children.entry(parent).or_default().push((slot, occurrence.id, info)),
            _ => return Err(PdfError::InvalidHierarchy),
        }
    }
    roots.sort_unstable_by_key(|&(index, id, _)| (index, id));
    let mut pending: Vec<_> = roots.into_iter().rev().map(|(_, id, info)| (id, info, None)).collect();
    let mut pages = Vec::with_capacity(records.len());
    while let Some((id, info, parent)) = pending.pop() {
        let page = pages.len();
        pages.push(Visit { sheet: info, parent });
        if let Some(mut group) = children.remove(&id) {
            group.sort_unstable_by_key(|&(slot, id, _)| (slot, id));
            pending.extend(group.into_iter().rev().map(|(slot, id, info)| (id, info, Some((page, slot)))));
        }
    }
    if pages.is_empty() || pages.len() != records.len() {
        return Err(PdfError::InvalidHierarchy);
    }
    Ok(pages)
}
