//! Original, uncharacterized material templates; family labels do not imply supplier properties.
use super::Material;
use crate::CommandError;

/// Starting material families for a rigid board. These templates carry no measured Tg/Dk/Df.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StarterMaterial {
    Fr4,
    HighTgFr4,
    Polyimide,
    Ptfe,
}
impl StarterMaterial {
    /// Built-in choices in their stable menu order.
    pub const ALL: [Self; 4] = [Self::Fr4, Self::HighTgFr4, Self::Polyimide, Self::Ptfe];
    /// The menu label describes a family, never a supplier grade or physical board capability.
    pub fn name(self) -> &'static str {
        match self {
            Self::Fr4 => "FR-4",
            Self::HighTgFr4 => "High-Tg FR-4",
            Self::Polyimide => "Polyimide laminate",
            Self::Ptfe => "PTFE laminate",
        }
    }
    /// Resolve an original built-in template's stable identity and revision. Reusing the same
    /// family across boards/profiles retains one immutable pin without matching display names.
    pub fn create(self) -> Result<Material, CommandError> {
        let id = match self {
            Self::Fr4 => 0x37b7872a6a174cab8b68000000000001,
            Self::HighTgFr4 => 0x37b7872a6a174cab8b68000000000002,
            Self::Polyimide => 0x37b7872a6a174cab8b68000000000003,
            Self::Ptfe => 0x37b7872a6a174cab8b68000000000004,
        };
        let material = Material {
            id: uuid::Uuid::from_u128(id),
            revision: 1,
            name: self.name().into(),
            family: Some(
                match self {
                    Self::Fr4 | Self::HighTgFr4 => "FR-4",
                    Self::Polyimide => "Polyimide",
                    Self::Ptfe => "PTFE",
                }
                .into(),
            ),
            manufacturer: None,
            product: None,
            source: None,
            tg: Vec::new(),
            dielectric: Vec::new(),
        };
        super::validate_catalog(&mut [material.clone()]).map_err(CommandError::InvalidParams)?;
        Ok(material)
    }
}
