//! Board references move together while physical geometry and accepted capture provenance stay fixed.
use super::*;
impl Board {
    /// Normalize qualified package and logical addresses through one project migration plan.
    /// Captured IDs/digests continue to describe their original historical input, not a new update.
    pub(crate) fn migrate_library_addresses(&mut self, replacements: &crate::library::compatibility::migration::Replacements) -> Result<(), String> {
        for assignment in self.assignments.values_mut() {
            if let Some(library) = replacements.footprint_library(&assignment.key()) {
                assignment.library = library.into();
            }
        }
        for placement in &mut self.footprints {
            if let Some(key) = placement.key()
                && let Some(library) = replacements.footprint_library(&key)
            {
                placement.library = Some(library.into());
            }
        }
        replacements.definitions(&mut self.definitions)?;
        if let Some(baseline) = &mut self.baseline {
            for component in Arc::make_mut(baseline).components.values_mut() {
                if let Some(library) = replacements.symbol_library(&component.library, &component.definition, component.revision) {
                    component.library = library.into();
                }
                if let Some(package) = &mut component.package
                    && let Some(library) = &package.library
                    && let Some(library) = replacements.footprint_library(&(library.clone(), package.definition.clone(), package.revision))
                {
                    package.library = Some(library.into());
                }
            }
        }
        Ok(())
    }
}
