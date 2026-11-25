//! Portable setup snapshots contain typed native data, without project paths or executable rules.
use super::{BoardSetup, LayerPurpose, SetupRecords, Stackup, Stratum};
use crate::{CommandError, board::silkscreen::Settings};
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::BTreeMap};
use uuid::Uuid;

/// Maximum portable profile payload, bounded before parsing or writing.
pub const MAX_PROFILE_BYTES: usize = 1_048_576;
const FORMAT: &str = "pcbcraft-setup-profile/1";
/// Adds copper roles, base copper, copper thickness qualifiers and finished board thickness.
const STACK_FORMAT: &str = "pcbcraft-setup-profile/2";
/// Adds thermal conductivity, thermal expansion, dielectric strength and tracking reports and
/// may carry every version 2 field.
const REPORT_FORMAT: &str = "pcbcraft-setup-profile/3";
/// Authored layer groups; derived physical and role groups remain computed.
const GROUP_FORMAT: &str = "pcbcraft-setup-profile/5";
/// Portable exports use one format with optional stack fields, reports and named groups.
pub const CURRENT_FORMAT: &str = "pcbcraft-setup-profile/6";

/// Identify the historical format that required this setup vocabulary.
fn historical_format_for(setup: &BoardSetup) -> &'static str {
    if setup.requires_group_format() {
        GROUP_FORMAT
    } else if setup.requires_report_format() {
        REPORT_FORMAT
    } else if setup.requires_stack_format() {
        STACK_FORMAT
    } else {
        FORMAT
    }
}

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
        let profile = Self { format: CURRENT_FORMAT.into(), id: crate::fresh_id().map_err(|e| e.to_string())?, revision: 1, name, setup, silkscreen };
        profile.validate()?;
        Ok(profile)
    }
    /// Reject unsupported versions/fields and oversized data before any draft is adopted.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_PROFILE_BYTES {
            return Err("Setup profile exceeds 1 MiB.".into());
        }
        let profile: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let raw: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if let Some(groups) = raw["setup"]["stackup"].get("layer_groups") {
            match profile.format.as_str() {
                CURRENT_FORMAT if groups.as_array().is_some_and(Vec::is_empty) => {
                    return Err("Setup profile layer_groups must be omitted when empty.".into());
                }
                CURRENT_FORMAT | GROUP_FORMAT => {}
                _ => return Err("The layer_groups field requires setup profile version 5 or 6.".into()),
            }
        }
        profile.validate()?;
        Ok(profile)
    }
    /// Export the current format in canonical units, preserving the profile identity and material pins.
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut profile = Cow::Borrowed(self);
        if self.format != CURRENT_FORMAT {
            profile.to_mut().format = CURRENT_FORMAT.into();
        }
        let mut bytes = serde_json::to_vec_pretty(profile.as_ref()).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        if bytes.len() > MAX_PROFILE_BYTES {
            return Err("Setup profile exceeds 1 MiB.".into());
        }
        Ok(bytes)
    }
    /// Validate typed setup and silkscreen for every profile. Historical headers also enforce their
    /// released requirements for stack fields, reports and named groups.
    fn validate(&self) -> Result<(), String> {
        if !matches!(self.format.as_str(), FORMAT | STACK_FORMAT | REPORT_FORMAT | GROUP_FORMAT | CURRENT_FORMAT)
            || self.id.is_nil()
            || self.revision == 0
        {
            return Err("Unsupported or invalid setup profile identity/version.".into());
        }
        if self.format != CURRENT_FORMAT && self.format != historical_format_for(&self.setup) {
            return Err(if self.setup.requires_group_format() || self.format == GROUP_FORMAT {
                "User layer groups require setup profile version 5, which requires at least one group."
            } else if self.setup.requires_report_format() || self.format == REPORT_FORMAT {
                "Thermal conductivity, thermal expansion, dielectric strength and tracking reports require setup profile version 3, which requires at least one of them."
            } else {
                "Copper roles, base copper, copper thickness qualifiers and finished board thickness require setup profile version 2, which requires at least one of them."
            }
            .into());
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
    /// Rebase a portable profile onto this board's surviving physical and unambiguous technical
    /// layer IDs. Extra target technical layers survive, preserving their graphic ownership.
    /// Existing material pins cannot be rewritten; ambiguity/name collisions fail through ordinary
    /// setup validation. Copper-count changes use the ordinary authored-stratum/removal checks.
    /// The profile's declared finished board thickness, tolerance and minimum replace the board's.
    /// The profile replaces the target group's list with its own (even if empty) and rebases member IDs.
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
        let mut stackup = Stackup { id: target.stackup.id, strata, ..incoming.stackup.clone() };
        for group in &mut stackup.layer_groups {
            for layer in &mut group.layers {
                *layer = ids.get(layer).copied().unwrap_or(*layer);
            }
        }
        Self::try_from(SetupRecords { materials, layers, stackup }).map_err(CommandError::InvalidParams)
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
        s.apply("project.new", &json!({"create_board":true,})).unwrap();
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
    /// Stack fields are optional in current exports. Historical /1 and /2 retain their vocabulary.
    #[test]
    fn stack_fields_share_the_current_profile_format() {
        let s = session();
        let plain = s.project().unwrap().board_setup().unwrap().with_starting_stackup(StackupPreset::Fr4OneOz).unwrap();
        let mut value = serde_json::to_value(&plain).unwrap();
        value["stackup"]["strata"][0]["role"] = json!("signal");
        value["stackup"]["strata"][0]["thickness_basis"] = json!("nominal");
        value["stackup"]["finished_thickness_nm"] = json!(1_600_000);
        let stacked: BoardSetup = serde_json::from_value(value).unwrap();
        let old = SetupProfile::new("Plain".into(), plain, Settings::default()).unwrap();
        let new = SetupProfile::new("Stack".into(), stacked.clone(), Settings::default()).unwrap();
        assert_eq!(old.format, CURRENT_FORMAT);
        assert_eq!(new.format, CURRENT_FORMAT);
        for (profile, historical) in [(&old, FORMAT), (&new, STACK_FORMAT)] {
            let mut legacy = serde_json::to_value(profile).unwrap();
            legacy["format"] = json!(historical);
            let legacy = SetupProfile::decode(&serde_json::to_vec(&legacy).unwrap()).unwrap();
            assert_eq!(legacy.encode().unwrap(), profile.encode().unwrap());
        }
        let bytes = new.encode().unwrap();
        assert_eq!(SetupProfile::decode(&bytes).unwrap(), new);
        assert_eq!(SetupProfile::decode(&bytes).unwrap().encode().unwrap(), bytes);
        let adopted = s.project().unwrap().board_setup().unwrap().with_profile(&new).unwrap();
        assert_eq!(adopted.stackup().finished_thickness_nm, Some(1_600_000));
        let mut downgraded = serde_json::to_value(&new).unwrap();
        downgraded["format"] = json!(FORMAT);
        let version_refusal = "require setup profile version 2, which requires at least one of them";
        let error = SetupProfile::decode(&serde_json::to_vec(&downgraded).unwrap()).unwrap_err();
        assert!(error.contains(version_refusal), "{error}");
        let mut upgraded = serde_json::to_value(&old).unwrap();
        upgraded["format"] = json!(STACK_FORMAT);
        let error = SetupProfile::decode(&serde_json::to_vec(&upgraded).unwrap()).unwrap_err();
        assert!(error.contains(version_refusal), "{error}");
    }
    /// Reports survive import and re-export under the current format. Historical /3 requires reports.
    #[test]
    fn material_reports_remain_optional_in_the_current_profile_format() {
        let s = session();
        let plain = s.project().unwrap().board_setup().unwrap().with_starting_stackup(StackupPreset::Fr4OneOz).unwrap();
        let mut value = serde_json::to_value(&plain).unwrap();
        let material = json!({"id":uuid::Uuid::from_u128(77),"revision":1,"name":"Synthetic",
            "cti":[{"cti_v":175,"material_group":"IIIa","basis":"minimum","source":{"document":"Synthetic"}}]});
        match value["materials"].as_array_mut() {
            Some(materials) => materials.push(material),
            None => value["materials"] = json!([material]),
        }
        let reported: BoardSetup = serde_json::from_value(value.clone()).unwrap();
        value["stackup"]["strata"][0]["role"] = json!("signal");
        let both: BoardSetup = serde_json::from_value(value).unwrap();
        for setup in [reported, both] {
            let profile = SetupProfile::new("Reports".into(), setup, Settings::default()).unwrap();
            assert_eq!(profile.format, CURRENT_FORMAT);
            let bytes = profile.encode().unwrap();
            let mut legacy = serde_json::to_value(&profile).unwrap();
            legacy["format"] = json!(REPORT_FORMAT);
            let legacy = SetupProfile::decode(&serde_json::to_vec(&legacy).unwrap()).unwrap();
            assert_eq!(legacy.encode().unwrap(), bytes);
            assert_eq!(SetupProfile::decode(&bytes).unwrap().encode().unwrap(), bytes);
            let adopted = s.project().unwrap().board_setup().unwrap().with_profile(&profile).unwrap();
            let reports = |setup: &BoardSetup| setup.materials().iter().find(|m| m.id == uuid::Uuid::from_u128(77)).map(|m| m.cti.clone());
            assert_eq!(reports(&adopted), reports(&profile.setup));
            assert!(reports(&adopted).is_some_and(|cti| cti.len() == 1));
            for older in [FORMAT, STACK_FORMAT] {
                let mut downgraded = serde_json::to_value(&profile).unwrap();
                downgraded["format"] = json!(older);
                let error = SetupProfile::decode(&serde_json::to_vec(&downgraded).unwrap()).map(|_| ()).unwrap_err();
                assert!(error.contains("require setup profile version 3"), "{older}: {error}");
            }
        }
        let mut upgraded = serde_json::to_value(SetupProfile::new("Plain".into(), plain, Settings::default()).unwrap()).unwrap();
        upgraded["format"] = json!(REPORT_FORMAT);
        let error = SetupProfile::decode(&serde_json::to_vec(&upgraded).unwrap()).map(|_| ()).unwrap_err();
        assert!(error.contains("require setup profile version 3"), "{error}");
    }
    /// Import preserves target-owned graphic layers and material pins, then applies atomically
    /// through the same revision-bound command as manual edits and supports exact undo.
    #[test]
    fn profile_rebases_layer_ownership_and_preserves_material_pins() {
        let mut source = session();
        let mut target = session();
        let plain = source.project().unwrap().board_setup().unwrap().with_starting_stackup(StackupPreset::Fr4TwoOz).unwrap();
        source.apply(SET_BOARD_SETUP, &json!({"setup":plain,"expected_revision":source.revision()})).unwrap();
        let mut combined = serde_json::to_value(&plain).unwrap();
        let material = Uuid::from_u128(77);
        let pin = json!({"id":material,"revision":7});
        combined["materials"].as_array_mut().unwrap().push(json!({
            "id":material,"revision":7,"name":"Synthetic core",
            "cti":[{"cti_v":175,"material_group":"IIIa","basis":"minimum","source":{"document":"Synthetic"}}]
        }));
        combined["stackup"]["strata"][1]["construction"]["material"] = pin.clone();
        combined["stackup"]["strata"][0]["role"] = json!("plane");
        combined["stackup"]["finished_thickness_nm"] = json!(1_600_001);
        let front = plain.copper_stack().next().unwrap().layer;
        let group = Uuid::from_u128(79);
        combined["stackup"]["layer_groups"] = json!([{"id":group,"name":"Return","layers":[front]}]);
        source.apply(SET_BOARD_SETUP, &json!({"setup":combined,"expected_revision":source.revision()})).unwrap();
        let export = |session: &Session| {
            let setup = session.project().unwrap().board_setup().unwrap().clone();
            let profile = SetupProfile::new("Heavy copper".into(), setup, Settings::default()).unwrap();
            SetupProfile::decode(&profile.encode().unwrap()).unwrap()
        };
        let profile = export(&source);
        assert_eq!(profile.format, CURRENT_FORMAT);
        source.apply("edit.undo", &json!({})).unwrap();
        let cleared = export(&source);
        assert_eq!(cleared.format, CURRENT_FORMAT);
        assert_eq!(cleared.setup, plain);
        assert!(cleared.setup.stackup().layer_groups.is_empty());
        assert!(cleared.setup.materials().iter().all(|definition| definition.cti.is_empty()));
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
        let target_front = setup.copper_stack().next().unwrap().layer;
        assert_ne!(front, target_front);
        assert_eq!(adopted.stackup().layer_groups[0].id, group);
        assert_eq!(adopted.stackup().layer_groups[0].layers, vec![target_front]);
        assert_eq!(crate::persistence::encode_documents(target.snapshot().unwrap()).unwrap(), before);
        target.apply(SET_BOARD_SETUP, &json!({"setup":adopted,"silkscreen":profile.silkscreen,"expected_revision":target.revision()})).unwrap();
        let applied = target.project().unwrap().board_setup().unwrap();
        assert_eq!(applied.laminate_thickness_nm(), Some(1_600_000));
        assert_eq!(applied.stackup().finished_thickness_nm, Some(1_600_001));
        assert_eq!(applied.stackup().layer_groups[0].layers, vec![target_front]);
        let applied_json = serde_json::to_value(applied).unwrap();
        assert_eq!(applied_json["stackup"]["strata"][1]["construction"]["material"], pin);
        let definition = |setup: &BoardSetup| setup.materials().iter().find(|m| m.id == material).cloned().unwrap();
        assert_eq!(definition(applied), definition(&profile.setup));
        let mut conflicting = profile.clone();
        conflicting.setup.materials.iter_mut().find(|m| m.id == material).unwrap().name = "Altered pin".into();
        assert!(target.project().unwrap().board_setup().unwrap().with_profile(&conflicting).is_err());
        target.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(crate::persistence::encode_documents(target.snapshot().unwrap()).unwrap(), before);
    }
}
