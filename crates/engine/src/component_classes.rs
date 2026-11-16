//! Project-owned component classes: named groups of components addressed by UUID, with explicit
//! membership stored by component identity in the project's instance home.

mod commands;
#[cfg(test)]
mod tests;

use crate::{Project, net_classes::MAX_CLASSES};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Create a component class with a fresh stable identity.
pub const CREATE: &str = "component_classes.create";
/// Rename a component class, retaining its identity and membership.
pub const UPDATE: &str = "component_classes.update";
/// Remove a component class that no component belongs to.
pub const REMOVE: &str = "component_classes.remove";
/// Inspect the component-class catalog without reading membership or connectivity.
pub const INSPECT: &str = "component_classes.inspect";
/// Replace the complete class set of one component.
pub const SET_CLASSES: &str = "components.set_classes";
/// Inspect the class memberships of one component, or of every classified component.
pub const INSPECT_CLASSES: &str = "components.inspect_classes";
/// Saved catalog version; later property vocabularies arrive under a new version.
pub const FORMAT: &str = "pcbcraft-component-classes/1";
/// Every component may belong to every class, and no more.
pub const MAX_BINDINGS: usize = crate::schematic::MAX_SYMBOLS * MAX_CLASSES;

/// One independently revisioned component class, addressed by UUID rather than display name.
/// Duplicate names are allowed because names never select membership.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub id: Uuid,
    pub revision: u32,
    pub name: String,
}

impl crate::class_catalog::Entry for Definition {
    fn id(&self) -> Uuid {
        self.id
    }
    fn revision(&self) -> u32 {
        self.revision
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// Versioned project catalog; a project with no component classes omits this record entirely.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub format: String,
    pub definitions: Vec<Definition>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self { format: FORMAT.into(), definitions: Vec::new() }
    }
}

impl Catalog {
    /// Validate the complete bounded catalog before adoption. Duplicate IDs never overwrite records.
    pub fn validate(&self) -> Result<(), String> {
        crate::class_catalog::validate(&self.format, FORMAT, "component-class", &self.definitions)
    }

    /// Resolve an exact class identity. Names and vector positions are never lookup authority.
    pub fn get(&self, id: Uuid) -> Option<&Definition> {
        crate::class_catalog::get(&self.definitions, id)
    }
}

/// The saved identity of a component in membership records: today its numeric component ID,
/// the same number its project record and placed symbols carry.
pub type ComponentKey = u64;

/// Read a component address (`component-N`, as reported by project inspection) as a key.
/// This is the only conversion between addressed components and stored membership keys.
pub fn component_key(address: &str) -> Option<ComponentKey> {
    crate::schematic::wiring::parse_id(address, "component-")
}

/// Write the address of a stored membership key, matching [`crate::PartInstance::id`].
pub fn component_address(key: ComponentKey) -> String {
    format!("component-{key}")
}

/// One explicit membership of one component in one class. Records sort by component, then class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub component: ComponentKey,
    pub class: Uuid,
}

/// Explicitly empty stored lists are not a second encoding of absent membership.
pub(crate) fn stored_bindings<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Option<Vec<Binding>>, D::Error> {
    let bindings = Vec::<Binding>::deserialize(de)?;
    if bindings.is_empty() || bindings.len() > MAX_BINDINGS {
        return Err(serde::de::Error::custom("Component class bindings must be omitted or nonempty and bounded."));
    }
    Ok(Some(bindings))
}

/// Check sorted order, the bound and that every binding names a known component and class.
/// Each distinct component and each distinct class is looked up once, however many bindings
/// share it; callers rely on that to keep validation linear in the number of bindings.
fn check_bindings(bindings: &[Binding], component_exists: impl Fn(ComponentKey) -> bool, class_exists: impl Fn(Uuid) -> bool) -> Result<(), String> {
    if bindings.len() > MAX_BINDINGS || bindings.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("Component class bindings must be bounded, unique and sorted by component and class.".into());
    }
    let components: BTreeSet<_> = bindings.iter().map(|binding| binding.component).collect();
    let classes: BTreeSet<_> = bindings.iter().map(|binding| binding.class).collect();
    if !components.into_iter().all(component_exists) || !classes.into_iter().all(class_exists) {
        return Err("Component class bindings must name existing components and classes.".into());
    }
    Ok(())
}

impl Project {
    /// Inspect immutable project-level component classes without reading membership.
    pub fn component_classes(&self) -> &Catalog {
        &self.component_classes
    }

    /// Every explicit component membership, sorted by component key and then class UUID.
    pub fn component_class_bindings(&self) -> &[Binding] {
        &self.component_class_bindings
    }

    /// The classes a component belongs to, in UUID order. Unknown components belong to none.
    pub fn classes_of_component(&self, component: ComponentKey) -> impl Iterator<Item = Uuid> + '_ {
        let start = self.component_class_bindings.partition_point(|binding| binding.component < component);
        self.component_class_bindings[start..].iter().take_while(move |binding| binding.component == component).map(|binding| binding.class)
    }

    /// Refuse unsorted, repeated, foreign or dangling memberships before adoption. Every
    /// binding names a live component and a class in this project's catalog. The catalog's
    /// identities are collected into a set once per call, so the cost grows with the number
    /// of bindings and classes, not with their product.
    pub(crate) fn validate_component_class_bindings(&self, bindings: &[Binding]) -> Result<(), String> {
        let classes: BTreeSet<Uuid> = self.component_classes.definitions.iter().map(|class| class.id).collect();
        check_bindings(bindings, |component| self.components.contains_key(&component), |class| classes.contains(&class))
    }

    /// Drop memberships of components this snapshot no longer contains, so a deleted component
    /// takes its memberships with it in the same history step. Undo restores both.
    pub(crate) fn prune_component_class_bindings(&mut self) {
        if self.component_class_bindings.iter().any(|binding| !self.components.contains_key(&binding.component)) {
            let components = &self.components;
            std::sync::Arc::make_mut(&mut self.component_class_bindings).retain(|binding| components.contains_key(&binding.component));
        }
    }
}
