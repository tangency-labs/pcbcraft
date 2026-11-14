use super::*;
use crate::authoring::{
    Item as Draft, Kind as DraftKind,
    publishing::{new_library, prepare},
};

/// Pinned revisions coexist; current, newer, absent and conflicting sources never replace them.
#[test]
fn exact_revisions_status_and_deliberate_variants_remain_distinct() {
    let manifest = new_library("workshop", "Workshop", "MIT OR Apache-2.0").unwrap();
    let first = prepare(Arc::new(Draft::new(DraftKind::Symbol).unwrap()), &manifest, 0, None, false).unwrap();
    let mut draft = first.item().as_ref().clone();
    draft.name = "Later symbol".into();
    let second = prepare(Arc::new(draft), &manifest, 1, Some(1), false).unwrap();
    let mut inventory = Inventory::default();
    inventory.insert(Definition::Symbol(first.item().symbol().unwrap()), "U1").unwrap();
    inventory.insert(Definition::Symbol(second.item().symbol().unwrap()), "U2").unwrap();
    let mut documents = first.documents().clone();
    documents.extend(second.documents().clone());
    documents.insert("library.json".into(), manifest.clone());
    let mut catalog = Catalog::default();
    catalog.install("source", Arc::new(super::super::NativeSource::load(&documents).unwrap())).unwrap();
    let rows: Vec<_> = inventory.entries().collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(inventory.status(&rows[0].item, &catalog), "Update available");
    assert_eq!(inventory.status(&rows[1].item, &catalog), "Matches source");
    // Higher revisions beside this identity, including the same ID in another kind/library,
    // cannot manufacture an update or keep a removed source available.
    let mut neighbors = super::super::Documents::from([("library.json".into(), manifest.clone())]);
    let mut neighbor = first.item().as_ref().clone();
    neighbor.id.push_str("-neighbor");
    neighbors.extend(prepare(Arc::new(neighbor), &manifest, 200, None, true).unwrap().documents().clone());
    let mut footprint = Draft::new(DraftKind::Footprint).unwrap();
    footprint.id = first.item().id.clone();
    neighbors.extend(prepare(Arc::new(footprint), &manifest, u32::MAX - 3, None, true).unwrap().documents().clone());
    catalog.install("neighbors", Arc::new(super::super::NativeSource::load(&neighbors).unwrap())).unwrap();
    let other_manifest = new_library("workshop-next", "Neighbor library", "MIT OR Apache-2.0").unwrap();
    let mut other = first.item().as_ref().clone();
    other.library = "workshop-next".into();
    let mut documents = prepare(Arc::new(other), &other_manifest, 200, None, true).unwrap().documents().clone();
    documents.insert("library.json".into(), other_manifest);
    catalog.install("other-library", Arc::new(super::super::NativeSource::load(&documents).unwrap())).unwrap();
    assert_eq!(inventory.status(&rows[0].item, &catalog), "Update available");
    assert_eq!(inventory.status(&rows[1].item, &catalog), "Matches source");
    catalog.remove("source").unwrap();
    assert!(rows.iter().all(|row| inventory.status(&row.item, &catalog) == "Source unavailable"));
    let mut conflicting = first.item().as_ref().clone();
    conflicting.name = "Conflicting symbol".into();
    let changed = prepare(Arc::new(conflicting), &manifest, 0, None, false).unwrap();
    let mut changed_docs = changed.documents().clone();
    changed_docs.insert("library.json".into(), manifest);
    catalog.install("source", Arc::new(super::super::NativeSource::load(&changed_docs).unwrap())).unwrap();
    assert_eq!(inventory.status(&rows[0].item, &catalog), "Conflict");
    assert_ne!(rows[0].item.name, "Conflicting symbol");
    let mut variant = Draft::new(DraftKind::Footprint).unwrap();
    variant.library = "project-owned-test".into();
    inventory.insert(Definition::Footprint(Arc::new(variant.footprint().unwrap())), "U3").unwrap();
    let variant = inventory.entries().find(|entry| entry.item.key.library == "project-owned-test").unwrap();
    assert_eq!(inventory.status(&variant.item, &catalog), "Project-only");
}
