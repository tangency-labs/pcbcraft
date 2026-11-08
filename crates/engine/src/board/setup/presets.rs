//! Original editable starting stackups, independent of any manufacturer's qualified construction.
use super::{BoardSetup, SetupRecords, Stratum};
use crate::{
    CommandError,
    board::materials::{Construction, StarterMaterial},
};

/// Two-layer starting choices use nominal foil thickness and a chosen finished laminate total.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackupPreset {
    Fr4OneOz,
    Fr4TwoOz,
    HighTgOneOz,
}
impl StackupPreset {
    /// Built-in choices in their stable menu order.
    pub const ALL: [Self; 3] = [Self::Fr4OneOz, Self::Fr4TwoOz, Self::HighTgOneOz];
    /// Describe the material family, nominal copper and laminate thickness in the chooser.
    pub fn name(self) -> &'static str {
        match self {
            Self::Fr4OneOz => "FR-4 · 1 oz copper · 1.6 mm",
            Self::Fr4TwoOz => "FR-4 · 2 oz copper · 1.6 mm",
            Self::HighTgOneOz => "High-Tg FR-4 · 1 oz copper · 1.6 mm",
        }
    }
}
impl BoardSetup {
    /// Stage a two-layer starting stackup with stable layer/profile IDs. Existing material pins
    /// remain intact; the chosen built-in material retains its catalog pin. Multilayer drafts are refused.
    /// Nominal foil uses 35/70 µm; the dielectric takes the residual of a chosen 1.6 mm total.
    pub fn with_starting_stackup(&self, preset: StackupPreset) -> Result<Self, CommandError> {
        if self.stackup.strata.len() != 3 {
            return Err(CommandError::InvalidParams("starting stackup presets require two copper layers".into()));
        }
        let family = if preset == StackupPreset::HighTgOneOz { StarterMaterial::HighTgFr4 } else { StarterMaterial::Fr4 };
        let material = family.create()?;
        let copper = if preset == StackupPreset::Fr4TwoOz { 70_000 } else { 35_000 };
        let mut strata = self.stackup.strata.clone();
        for stratum in &mut strata {
            match stratum {
                Stratum::Copper { thickness_nm, .. } => *thickness_nm = Some(copper),
                Stratum::Dielectric { thickness_nm, material: label, construction, thickness_tolerance_nm, .. } => {
                    *thickness_nm = Some(1_600_000 - 2 * copper);
                    *thickness_tolerance_nm = None;
                    *label = Some(material.name.clone());
                    *construction = Some(Construction::Core { material: Some(material.reference()), glass_construction: None });
                }
            }
        }
        let materials = retain_template(&self.materials, material)?;
        Self::try_from(SetupRecords { materials, layers: self.layers.clone(), stackup: super::Stackup { id: self.stackup.id, strata } })
            .map_err(CommandError::InvalidParams)
    }
}

impl BoardSetup {
    /// Add an uncharacterized template. Two-layer boards use its pinned definition for the core;
    /// multilayer boards retain their per-gap choices. Dimensions and existing pins are preserved.
    pub fn with_starting_material(&self, family: StarterMaterial) -> Result<Self, CommandError> {
        let material = family.create()?;
        let mut stackup = self.stackup.clone();
        if self.stackup.strata.len() == 3
            && let Stratum::Dielectric { material: label, construction, .. } = &mut stackup.strata[1]
        {
            *label = Some(material.name.clone());
            *construction = Some(Construction::Core { material: Some(material.reference()), glass_construction: None });
        }
        let materials = retain_template(&self.materials, material)?;
        Self::try_from(SetupRecords { materials, layers: self.layers.clone(), stackup }).map_err(CommandError::InvalidParams)
    }
}

impl BoardSetup {
    /// Select an existing immutable pin for a two-layer core without changing dimensions or
    /// manufacturing reports. Unknown references and multilayer-wide replacement are rejected.
    pub fn with_core_material(&self, reference: crate::board::materials::MaterialRef) -> Result<Self, CommandError> {
        if self.stackup.strata.len() != 3 {
            return Err(CommandError::InvalidParams("choose materials per dielectric on multilayer boards".into()));
        }
        let material = self
            .materials
            .iter()
            .find(|m| m.reference() == reference)
            .ok_or_else(|| CommandError::InvalidParams("unknown pinned material".into()))?;
        let mut stackup = self.stackup.clone();
        if let Stratum::Dielectric { material: label, construction, .. } = &mut stackup.strata[1] {
            *label = Some(material.name.clone());
            *construction = Some(Construction::Core { material: Some(reference), glass_construction: None });
        }
        Self::try_from(SetupRecords { materials: self.materials.clone(), layers: self.layers.clone(), stackup }).map_err(CommandError::InvalidParams)
    }
}

/// Reuse exact built-in pins while rejecting contradictory content for an immutable key.
fn retain_template(
    existing: &[crate::board::materials::Material],
    material: crate::board::materials::Material,
) -> Result<Vec<crate::board::materials::Material>, CommandError> {
    let mut materials = existing.to_vec();
    if let Some(old) = materials.iter().find(|old| old.reference() == material.reference()) {
        if old != &material {
            return Err(CommandError::InvalidParams("built-in material conflicts with a pinned revision".into()));
        }
    } else {
        materials.push(material);
    }
    Ok(materials)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Built-in defaults keep exact totals, reuse immutable pins, and do not fabricate reports.
    #[test]
    fn starting_materials_are_stable_and_repeated_selection_is_a_no_op() {
        let blank = BoardSetup::preset(2).unwrap();
        for preset in StackupPreset::ALL {
            let setup = blank.with_starting_stackup(preset).unwrap();
            assert_eq!(setup.laminate_thickness_nm(), Some(1_600_000));
            assert_eq!(setup.with_starting_stackup(preset).unwrap(), setup);
            assert!(setup.materials().iter().all(|m| m.tg.is_empty() && m.dielectric.is_empty() && m.manufacturer.is_none()));
        }
        for family in StarterMaterial::ALL {
            let a = family.create().unwrap();
            let b = family.create().unwrap();
            assert_eq!(a, b);
            let setup = blank.with_starting_material(family).unwrap();
            assert_eq!(setup.with_starting_material(family).unwrap(), setup);
        }
        let multi = BoardSetup::preset(4).unwrap();
        assert!(multi.with_starting_stackup(StackupPreset::Fr4OneOz).is_err());
        let with_material = multi.with_starting_material(StarterMaterial::Polyimide).unwrap();
        assert_eq!(with_material.stackup(), multi.stackup());
    }
}
