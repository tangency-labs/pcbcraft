//! Read-only removal plans describe exact revision groups and known library dependencies.
use super::*;

/// One source-local item identity includes every published revision, never its dependencies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemRemoval {
    pub library: String,
    pub id: String,
    pub kind: Kind,
    pub name: String,
    pub revisions: BTreeSet<u32>,
    pub files: Vec<String>,
    pub fingerprints: Vec<Value>,
}
impl ItemRemoval {
    /// Exact references remain kind-specific; display names and file names never imply ownership.
    pub fn includes(&self, kind: Kind, reference: &Reference) -> bool {
        kind == self.kind && reference.library == self.library && reference.id == self.id && self.revisions.contains(&reference.revision)
    }
    /// Manufacturer records are the only native records that reference reusable part identities.
    fn dependent(&self, item: &Item) -> bool {
        item.manufacturer_part().is_some_and(|record| self.includes(Kind::Part, &record.part))
    }
}
impl NativeSource {
    /// Plan deletion of all revisions of a generic/orderable item in this source. Unknown,
    /// malformed or locally referenced records fail without changing the source snapshot.
    pub fn item_removal(&self, key: &Key) -> Result<ItemRemoval, String> {
        if !matches!(key.kind, Kind::Part | Kind::ManufacturerPart) {
            return Err("Delete part supports generic and orderable part records.".into());
        }
        if !self.problems.is_empty() {
            return Err("Resolve library diagnostics before deleting a part.".into());
        }
        let selected = self.items.iter().find(|item| item.key == *key).ok_or("The selected revision is unavailable in this library folder.")?;
        let items: Vec<_> =
            self.items.iter().filter(|item| item.key.kind == key.kind && item.key.library == key.library && item.key.id == key.id).collect();
        let plan = ItemRemoval {
            library: key.library.clone(),
            id: key.id.clone(),
            kind: key.kind,
            name: selected.name.clone(),
            revisions: items.iter().map(|item| item.key.revision).collect(),
            files: items.iter().map(|item| item.file.clone()).collect(),
            fingerprints: items.iter().map(|item| item.inspect()).collect(),
        };
        check_dependents(&plan, self.items.iter())?;
        Ok(plan)
    }
}
impl Catalog {
    /// A registration owns the operation even when identical records are available elsewhere.
    /// Known dependencies in every loaded source block removal rather than silently breaking it.
    pub fn item_removal(&self, source: &str, key: &Key) -> Result<ItemRemoval, String> {
        if self.builtin.contains(source) {
            return Err("Built-in library records cannot be deleted.".into());
        }
        let plan = self.sources.get(source).ok_or("The library source is unavailable.")?.item_removal(key)?;
        self.check_item_removal(&plan)?;
        Ok(plan)
    }
    /// Recheck known library relationships immediately before confirming a captured plan.
    pub fn check_item_removal(&self, plan: &ItemRemoval) -> Result<(), String> {
        check_dependents(plan, self.sources.values().flat_map(|source| source.items.iter()))
    }
}
/// Report a bounded list of named blockers; no dependent record is implicitly cascaded.
fn check_dependents<'a>(plan: &ItemRemoval, items: impl Iterator<Item = &'a Item>) -> Result<(), String> {
    let mut names = BTreeSet::new();
    for item in items.filter(|item| plan.dependent(item)) {
        names.insert(format!("{} ({}/{} r{})", item.name, item.key.library, item.key.id, item.key.revision));
        if names.len() >= 8 {
            break;
        }
    }
    if names.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "This part is referenced by orderable records: {}. Delete or revise those records first.",
            names.into_iter().collect::<Vec<_>>().join(", ")
        ))
    }
}
