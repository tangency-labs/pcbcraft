//! Portable setup snapshots contain typed native data, without project paths or executable rules.
use super::{BoardSetup, LayerPurpose, SetupRecords, Stackup, Stratum};
use crate::{CommandError, board::silkscreen::Settings};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Maximum portable profile payload, bounded before parsing or writing.
pub const MAX_PROFILE_BYTES: usize = 1_048_576;
const FORMAT: &str = "pcbcraft-setup-profile/1";

/// A self-contained, named setup profile. Applying copies its values into an explicitly addressed
/// board; it does not create an automatic link or claim fabrication/routing rule qualification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupProfile {
    format: String,
    pub id: Uuid,
    pub revision: u32,
    pub name: String,
    pub setup: BoardSetup,
    pub silkscreen: Settings,
}
impl SetupProfile {
    /// Capture a new shareable snapshot without changing board or material identities.
    pub fn new(name: String, setup: BoardSetup, silkscreen: Settings) -> Result<Self, String> {
        let profile = Self { format: FORMAT.into(), id: crate::fresh_id().map_err(|e| e.to_string())?, revision: 1, name, setup, silkscreen };
        profile.validate()?;
        Ok(profile)
    }
    /// Reject unsupported versions/fields and oversized data before any draft is adopted.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_PROFILE_BYTES {
            return Err("Setup profile exceeds 1 MiB.".into());
        }
        let profile: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        profile.validate()?;
        Ok(profile)
    }
    /// Encode deterministic, human-readable canonical-unit data with all material pins embedded.
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        if bytes.len() > MAX_PROFILE_BYTES {
            return Err("Setup profile exceeds 1 MiB.".into());
        }
        Ok(bytes)
    }
    /// In-memory construction receives the same validation as a decoded profile.
    fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT || self.id.is_nil() || self.revision == 0 {
            return Err("Unsupported or invalid setup profile identity/version.".into());
        }
        if self.name.trim() != self.name || self.name.is_empty() || self.name.chars().count() > 80 || self.name.chars().any(char::is_control) {
            return Err("Profile name must contain 1–80 characters without surrounding whitespace or control characters.".into());
        }
        let _: BoardSetup = serde_json::from_value(serde_json::to_value(&self.setup).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let _: Settings = serde_json::from_value(serde_json::to_value(self.silkscreen).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        Ok(())
    }
}
impl BoardSetup {
    /// Rebase a portable profile onto this board's retained physical and unambiguous technical
    /// layer IDs. Extra target layers survive, preserving graphic ownership. Existing material
    /// pins cannot be rewritten; ambiguity/name collisions fail through ordinary setup validation.
    /// Shrinking authored strata retains the same protection as a manual copper-count edit.
    pub fn with_profile(&self, profile: &SetupProfile) -> Result<Self, CommandError> {
        profile.validate().map_err(CommandError::InvalidParams)?;
        let incoming = &profile.setup;
        let target = self.with_copper_count(incoming.stackup.strata.len().div_ceil(2) as u64)?;
        let mut ids = BTreeMap::new();
        for (source, destination) in incoming.stackup.strata.iter().zip(&target.stackup.strata) {
            match (source, destination) {
                (Stratum::Copper { layer: a, .. }, Stratum::Copper { layer: b, .. }) => {
                    ids.insert(*a, *b);
                }
                (Stratum::Dielectric { id: a, .. }, Stratum::Dielectric { id: b, .. }) => {
                    ids.insert(*a, *b);
                }
                _ => return Err(CommandError::InvalidParams("Profile stackup topology is incompatible.".into())),
            }
        }
        for layer in &incoming.layers {
            if layer.purpose == LayerPurpose::Copper {
                continue;
            }
            let matches = target.layers.iter().filter(|other| other.purpose == layer.purpose && other.side == layer.side).collect::<Vec<_>>();
            let source_count = incoming.layers.iter().filter(|other| other.purpose == layer.purpose && other.side == layer.side).count();
            if let [only] = matches.as_slice()
                && source_count == 1
            {
                ids.insert(layer.id, only.id);
            }
        }
        let mut layers = incoming.layers.clone();
        for layer in &mut layers {
            layer.id = ids.get(&layer.id).copied().unwrap_or(layer.id);
        }
        for layer in &target.layers {
            if !layers.iter().any(|other| other.id == layer.id) {
                layers.push(layer.clone());
            }
        }
        let mut strata = incoming.stackup.strata.clone();
        for stratum in &mut strata {
            match stratum {
                Stratum::Copper { layer, .. } => *layer = ids.get(layer).copied().unwrap_or(*layer),
                Stratum::Dielectric { id, .. } => *id = ids.get(id).copied().unwrap_or(*id),
            }
        }
        let mut materials = target.materials.clone();
        for material in &incoming.materials {
            if let Some(old) = materials.iter().find(|old| old.reference() == material.reference()) {
                if old != material {
                    return Err(CommandError::InvalidParams("Profile would rewrite a pinned material revision.".into()));
                }
            } else {
                materials.push(material.clone());
            }
        }
        Self::try_from(SetupRecords { materials, layers, stackup: Stackup { id: target.stackup.id, strata } }).map_err(CommandError::InvalidParams)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::setup::StackupPreset;
    use crate::{ADD_DRAWING, INITIALIZE_BOARD_SETUP, SET_BOARD_SETUP, Session};
    use serde_json::json;
    /// Create an original two-layer board through production commands for profile tests.
    fn session() -> Session {
        let mut s = Session::default();
        s.apply("project.new", &json!({})).unwrap();
        s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
        s
    }
    /// Profiles round-trip with exact dimensions and reject foreign required fields/versions.
    #[test]
    fn portable_profile_is_strict_and_self_contained() {
        let s = session();
        let setup = s.project().unwrap().board_setup().unwrap().with_starting_stackup(StackupPreset::Fr4OneOz).unwrap();
        let style = Settings { text_height_nm: 1_000_001, stroke_width_nm: 150_001, pad_clearance_nm: 250_001 };
        let profile = SetupProfile::new("Starter board".into(), setup, style).unwrap();
        let bytes = profile.encode().unwrap();
        assert_eq!(SetupProfile::decode(&bytes).unwrap(), profile);
        assert_eq!(SetupProfile::decode(&bytes).unwrap().encode().unwrap(), bytes);
        let mut value = serde_json::to_value(&profile).unwrap();
        value["format"] = json!("pcbcraft-setup-profile/99");
        assert!(SetupProfile::decode(&serde_json::to_vec(&value).unwrap()).is_err());
        value["format"] = json!(FORMAT);
        value["constraints"] = json!({"minimum_copper_clearance_nm":200000});
        assert!(SetupProfile::decode(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(SetupProfile::decode(&vec![b' '; MAX_PROFILE_BYTES + 1]).is_err());
    }
    /// Import preserves target-owned graphic layers and material pins, then applies atomically
    /// through the same revision-bound command as manual edits and supports exact undo.
    #[test]
    fn profile_rebases_layer_ownership_and_preserves_material_pins() {
        let source = session();
        let mut target = session();
        let source_setup = source.project().unwrap().board_setup().unwrap().with_starting_stackup(StackupPreset::Fr4TwoOz).unwrap();
        let profile = SetupProfile::new("Heavy copper".into(), source_setup, Settings::default()).unwrap();
        let edge = target.project().unwrap().board_setup().unwrap().layers().iter().find(|l| l.purpose == LayerPurpose::Outline).unwrap().id;
        target
            .apply(
                ADD_DRAWING,
                &json!({"layer":edge,"points":[{"x_nm":0,"y_nm":0},{"x_nm":1000000,"y_nm":0}],"closed":false,"expected_revision":target.revision()}),
            )
            .unwrap();
        let before = crate::persistence::encode_documents(target.snapshot().unwrap()).unwrap();
        let setup = target.project().unwrap().board_setup().unwrap();
        let identity = setup.stackup().id;
        let adopted = setup.with_profile(&profile).unwrap();
        assert_eq!(adopted.stackup().id, identity);
        assert!(adopted.layers().iter().any(|l| l.id == edge));
        assert_eq!(crate::persistence::encode_documents(target.snapshot().unwrap()).unwrap(), before);
        target.apply(SET_BOARD_SETUP, &json!({"setup":adopted,"silkscreen":profile.silkscreen,"expected_revision":target.revision()})).unwrap();
        assert_eq!(target.project().unwrap().board_setup().unwrap().laminate_thickness_nm(), Some(1600000));
        let mut conflicting = profile.clone();
        conflicting.setup.materials[0].name = "Altered pin".into();
        assert!(target.project().unwrap().board_setup().unwrap().with_profile(&conflicting).is_err());
        target.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(crate::persistence::encode_documents(target.snapshot().unwrap()).unwrap(), before);
    }
}
