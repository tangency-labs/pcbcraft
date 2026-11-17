//! Neutral assembly output profiles, pinned by immutable content revision in the project.
//! Boards explicitly adopt revisions; names and optional vendor metadata never select behaviour.
use super::{
    Footprint,
    definition::{self, MAX_OFFSET_NM},
    origin::{BottomSide, Placed},
    view::BoardView,
};
use crate::{CommandError, Project, Session, schematic::Point};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

/// Pin an immutable profile revision, or retire its identity without invalidating adoptions.
pub const SET_ASSEMBLY_PROFILE: &str = "pcb.set_assembly_profile";
/// Preview changed corrections and placement values without editing the project.
pub const PREVIEW_ASSEMBLY_PROFILE: &str = "pcb.preview_assembly_profile";
/// Accept exactly a preview against the current project revision, in one undo step.
pub const ADOPT_ASSEMBLY_PROFILE: &str = "pcb.adopt_assembly_profile";
/// Schema version, independent of each profile's content revision.
pub const FORMAT: &str = "pcbcraft-assembly-profile/1";
/// Distinct identities, including retired identities whose revision watermarks prevent reuse.
pub const MAX_PROFILES: usize = 64;
/// Pinned content revisions, including those retained for board adoptions.
pub const MAX_REVISIONS: usize = 128;
/// Corrections in one profile and in all retained project revisions together.
pub const MAX_CORRECTIONS: usize = 4096;
/// Characters in display metadata and provenance hints.
pub const MAX_NAME_CHARS: usize = 64;

/// A package's orientation and centroid corrections, in library footprint coordinates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageCorrection {
    pub library: String,
    pub definition: String,
    /// Counterclockwise whole degrees on the component side, 0 through 359; omitted at zero.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotation: u16,
    /// Integer nanometres, Y down as library pads are given; omitted for the footprint origin.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "super::reference::present")]
    pub centroid: Option<Point>,
}

/// Zero rotation is represented by omission in canonical profile records.
fn is_zero(rotation: &u16) -> bool {
    *rotation == 0
}

impl PackageCorrection {
    /// Refuse invalid package identities, empty corrections and offsets outside the footprint range.
    fn validate(&self) -> Result<(), String> {
        if !definition::valid_id(&self.library) || !definition::valid_id(&self.definition) {
            return Err("A correction needs a valid library and footprint definition id.".into());
        }
        if self.rotation >= 360 {
            return Err("A rotation correction must be a whole number of degrees from 0 through 359.".into());
        }
        if let Some(centroid) = self.centroid
            && (centroid == (Point { x_nm: 0, y_nm: 0 })
                || ![centroid.x_nm, centroid.y_nm].into_iter().all(|n| (-MAX_OFFSET_NM..=MAX_OFFSET_NM).contains(&n)))
        {
            return Err("A centroid correction must be nonzero and within 100 mm of the footprint origin; omit it to use the origin.".into());
        }
        if self.rotation == 0 && self.centroid.is_none() {
            return Err(format!("The correction for {}/{} changes nothing; remove it instead.", self.library, self.definition));
        }
        Ok(())
    }

    /// Apply the component-side correction, subtracting rotation only for bottom parts seen from above.
    fn apply(&self, footprint: &Footprint, row: Placed, frame: super::origin::Frame, convention: BottomSide) -> Placed {
        let rotation = match (footprint.bottom, convention) {
            (true, BottomSide::ViewedFromTop) => (row.rotation + 360 - self.rotation) % 360,
            _ => (row.rotation + self.rotation) % 360,
        };
        let at = match self.centroid {
            Some(centroid) => frame.place(footprint.point(centroid), footprint.rotation, footprint.bottom, convention).at,
            None => row.at,
        };
        Placed { at, rotation }
    }
}

/// Origin of a profile revision. Source is a portable human hint, never a path or an identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Provenance {
    Authored,
    Imported { source: String },
}

/// Immutable content; its revision is not the record's schema version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileContent {
    pub revision: u32,
    pub name: String,
    pub provenance: Provenance,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "super::reference::present")]
    pub vendor: Option<String>,
    pub bottom_side: BottomSide,
    #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "listed")]
    pub corrections: Arc<Vec<Arc<PackageCorrection>>>,
}

/// A portable, self-contained profile revision. The file wrapper is independent of this record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssemblyProfile {
    pub format: String,
    pub id: Uuid,
    pub content: ProfileContent,
}

/// A board's explicit pinned choice. Revision is content identity, not schema version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Adoption {
    pub profile: Uuid,
    pub revision: u32,
}

/// The last revision seen for an identity, retained after retirement to prohibit identity reuse.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileHead {
    pub profile: Uuid,
    pub revision: u32,
    pub removed: bool,
}

/// Project-owned records. Arc payloads are shared by revisions of the project and by Undo.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub heads: Vec<ProfileHead>,
    #[serde(serialize_with = "serialize_revisions", deserialize_with = "deserialize_revisions")]
    pub revisions: Vec<Arc<AssemblyProfile>>,
}

/// Serialize pinned payloads by reference without changing their shared in-memory ownership.
fn serialize_revisions<S: serde::Serializer>(revisions: &[Arc<AssemblyProfile>], serializer: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut sequence = serializer.serialize_seq(Some(revisions.len()))?;
    for revision in revisions {
        sequence.serialize_element(revision.as_ref())?;
    }
    sequence.end()
}

/// Loaded payloads become immutable shared records before any snapshots or history are created.
fn deserialize_revisions<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<Arc<AssemblyProfile>>, D::Error> {
    Ok(Vec::<AssemblyProfile>::deserialize(deserializer)?.into_iter().map(Arc::new).collect())
}

/// Canonical correction records never contain an explicit zero rotation or an empty list.
fn listed<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Arc<Vec<Arc<PackageCorrection>>>, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SavedCorrection {
        library: String,
        definition: String,
        #[serde(default, deserialize_with = "super::reference::present")]
        rotation: Option<u16>,
        #[serde(default, deserialize_with = "super::reference::present")]
        centroid: Option<Point>,
    }
    let saved = Vec::<SavedCorrection>::deserialize(deserializer)?;
    if saved.is_empty() {
        return Err(serde::de::Error::custom("an empty corrections array is not allowed"));
    }
    let corrections = saved
        .into_iter()
        .map(|entry| {
            if entry.rotation == Some(0) {
                return Err(serde::de::Error::custom("a zero rotation is omitted, never written"));
            }
            Ok(Arc::new(PackageCorrection {
                library: entry.library,
                definition: entry.definition,
                rotation: entry.rotation.unwrap_or(0),
                centroid: entry.centroid,
            }))
        })
        .collect::<Result<Vec<_>, D::Error>>()?;
    Ok(Arc::new(corrections))
}

/// Validate bounded metadata without surrounding whitespace or control characters.
fn valid_text(text: &str) -> bool {
    let space = |c: char| c.is_whitespace() || c == '\u{feff}';
    !text.is_empty() && text.chars().count() <= MAX_NAME_CHARS && text.trim_matches(space) == text && !text.chars().any(char::is_control)
}

impl ProfileContent {
    /// Share unchanged corrections across content revisions; a metadata-only edit shares the list too.
    fn share_corrections(&mut self, previous: &Self) {
        if self.corrections == previous.corrections {
            self.corrections = previous.corrections.clone();
            return;
        }
        for correction in Arc::make_mut(&mut self.corrections) {
            let key = (&correction.library, &correction.definition);
            if let Ok(index) = previous.corrections.binary_search_by(|c| (&c.library, &c.definition).cmp(&key))
                && previous.corrections[index] == *correction
            {
                *correction = previous.corrections[index].clone();
            }
        }
    }

    /// Check exact integer revisions, bounded metadata and canonical, nonempty corrections.
    pub fn validate(&self) -> Result<(), String> {
        if self.revision == 0 {
            return Err("An assembly profile content revision must be positive.".into());
        }
        if !valid_text(&self.name) || self.vendor.as_ref().is_some_and(|v| !valid_text(v)) {
            return Err(format!(
                "Assembly profile metadata needs 1 to {MAX_NAME_CHARS} characters without surrounding spaces or control characters."
            ));
        }
        if let Provenance::Imported { source } = &self.provenance
            && !valid_text(source)
        {
            return Err("Imported profile provenance needs a bounded source hint.".into());
        }
        if self.corrections.len() > MAX_CORRECTIONS {
            return Err(format!("An assembly profile may carry at most {MAX_CORRECTIONS} corrections."));
        }
        for correction in self.corrections.iter() {
            correction.validate()?;
        }
        if self.corrections.windows(2).any(|pair| (&pair[0].library, &pair[0].definition) >= (&pair[1].library, &pair[1].definition)) {
            return Err("Assembly profile corrections must be unique and in library and definition order.".into());
        }
        Ok(())
    }
}

impl AssemblyProfile {
    /// Refuse unsupported schema versions, nil identities or invalid content.
    pub fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT || self.id.is_nil() {
            return Err("Unsupported assembly profile format or invalid profile identity.".into());
        }
        self.content.validate()
    }
    /// The definition's correction, independent of the footprint revision; markers never match.
    pub(crate) fn correction_for(&self, footprint: &Footprint) -> Option<&PackageCorrection> {
        let library = footprint.library.as_deref()?;
        let key = (library, footprint.definition.as_str());
        self.content
            .corrections
            .binary_search_by(|c| (c.library.as_str(), c.definition.as_str()).cmp(&key))
            .ok()
            .map(|i| self.content.corrections[i].as_ref())
    }
}

impl Catalog {
    /// Resolve an exact immutable revision. No lookup ever substitutes a newer revision.
    pub fn get(&self, adoption: &Adoption) -> Option<&AssemblyProfile> {
        self.revisions.iter().find(|r| r.id == adoption.profile && r.content.revision == adoption.revision).map(Arc::as_ref)
    }
    /// Validate bounds, canonical identities, retained revisions and every board reference.
    pub fn validate(&self, adoptions: &[Adoption]) -> Result<(), String> {
        if self.heads.len() > MAX_PROFILES || self.revisions.len() > MAX_REVISIONS {
            return Err(format!("A project may retain at most {MAX_PROFILES} assembly profile identities and {MAX_REVISIONS} revisions."));
        }
        if self.heads.windows(2).any(|p| p[0].profile >= p[1].profile)
            || self.heads.iter().any(|h| h.profile.is_nil() || h.revision == 0)
            || self.revisions.windows(2).any(|p| (p[0].id, p[0].content.revision) >= (p[1].id, p[1].content.revision))
        {
            return Err("Assembly profile identities and revisions must be non-nil, positive, unique and sorted.".into());
        }
        if self.revisions.iter().map(|r| r.content.corrections.len()).sum::<usize>() > MAX_CORRECTIONS {
            return Err(format!("The assembly profiles of one project may carry at most {MAX_CORRECTIONS} corrections together."));
        }
        for revision in &self.revisions {
            revision.validate()?;
            if !self.heads.iter().any(|h| h.profile == revision.id && h.revision >= revision.content.revision) {
                return Err("An assembly profile revision has no matching identity watermark.".into());
            }
        }
        let mut names = BTreeSet::new();
        for head in self.heads.iter().filter(|head| !head.removed) {
            let latest = self
                .get(&Adoption { profile: head.profile, revision: head.revision })
                .ok_or("An available assembly profile is missing its latest revision.")?;
            if !names.insert(&latest.content.name) {
                return Err("Available assembly profile names must be unique in the project.".into());
            }
        }
        if adoptions.windows(2).any(|p| p[0].profile >= p[1].profile) || adoptions.iter().any(|a| self.get(a).is_none()) {
            return Err("Board assembly profile adoptions must be unique, sorted and reference pinned content revisions.".into());
        }
        Ok(())
    }
    /// Release only unadopted, superseded or retired payloads in the same transaction as the edit.
    /// Old snapshots keep their Arc payloads, so Undo and Redo never depend on the current catalog.
    fn cleanup(&mut self, adoptions: &[Adoption]) {
        self.revisions.retain(|record| {
            adoptions.iter().any(|a| a.profile == record.id && a.revision == record.content.revision)
                || self.heads.iter().any(|h| !h.removed && h.profile == record.id && h.revision == record.content.revision)
        });
    }
}

impl Project {
    /// Pinned revisions and revision watermarks, independent of board adoption and display names.
    pub fn assembly_profiles(&self) -> &Catalog {
        &self.assembly_profiles
    }
}

impl<'a> BoardView<'a> {
    /// Explicit adopted profile revisions on this board, sorted by profile identity.
    pub fn assembly_profiles(self) -> &'a [Adoption] {
        &self.project().board.assembly_profiles
    }
    /// One placement row using the board's exact adopted revision, or the generic convention.
    /// Refuses unknown profiles or footprints and unresolved placement origins; it never edits.
    pub fn placement_row(self, footprint: &str, profile: Option<Uuid>) -> Result<Placed, String> {
        let record = match profile {
            Some(id) => {
                let adoption = self
                    .assembly_profiles()
                    .iter()
                    .find(|a| a.profile == id)
                    .ok_or_else(|| format!("No adopted assembly profile {id} on this board."))?;
                Some(self.project().assembly_profiles.get(adoption).ok_or("The adopted assembly profile revision is missing.")?)
            }
            None => None,
        };
        self.profile_row(footprint, record)
    }
    /// Apply generic transforms to supplied, already validated profile data, without selecting by vendor.
    fn profile_row(self, footprint: &str, profile: Option<&AssemblyProfile>) -> Result<Placed, String> {
        let placement = self.placement(footprint).ok_or_else(|| format!("No footprint {footprint} on this board."))?;
        let frame = self.placement_origin()?.frame();
        let convention = profile.map_or_else(BottomSide::default, |p| p.content.bottom_side);
        let row = frame.place(Point { x_nm: placement.x_nm, y_nm: placement.y_nm }, placement.rotation, placement.bottom, convention);
        Ok(match profile.and_then(|p| p.correction_for(placement)) {
            Some(c) => c.apply(placement, row, frame, convention),
            None => row,
        })
    }
}

/// The profile content schema, separate from the profile identity and the record schema version.
/// Semantic constraints, including unique ordered corrections, are enforced by the shared validator.
pub(crate) fn schema() -> Value {
    let coordinate = json!({"type":"integer","minimum":-MAX_OFFSET_NM,"maximum":MAX_OFFSET_NM});
    let id = json!({"type":"string","minLength":1,"maxLength":64,"pattern":"^[a-z0-9]([a-z0-9-]*[a-z0-9])?$"});
    let text = json!({"type":"string","minLength":1,"maxLength":MAX_NAME_CHARS});
    json!({"oneOf":[{"type":"null"},{"type":"object","properties":{
        "revision":{"type":"integer","minimum":1,"maximum":u32::MAX}, "name":text,"vendor":text,
        "provenance":{"oneOf":[
            {"type":"object","properties":{"kind":{"const":"authored"}},"required":["kind"],"additionalProperties":false},
            {"type":"object","properties":{"kind":{"const":"imported"},"source":text},"required":["kind","source"],"additionalProperties":false}]},
        "bottom_side":{"type":"string","enum":["viewed_from_top","viewed_from_bottom"]},
        "corrections":{"type":"array","minItems":1,"maxItems":MAX_CORRECTIONS,"items":{"type":"object","properties":{
            "library":id,"definition":id,"rotation":{"type":"integer","minimum":0,"maximum":359},
            "centroid":{"type":"object","properties":{"x_nm":coordinate,"y_nm":coordinate},"required":["x_nm","y_nm"],"additionalProperties":false,
                "not":{"properties":{"x_nm":{"const":0},"y_nm":{"const":0}}}}
        },"required":["library","definition"],"additionalProperties":false,"anyOf":[{"required":["rotation"]},{"required":["centroid"]}]}}
    },"required":["revision","name","provenance"],"additionalProperties":false}]})
}

/// Read JSON's integer-valued decimal spellings exactly, without comparing through floating point.
/// Bounded profile numbers must fit i64; fractional or larger numbers remain invalid.
pub(crate) fn integer(value: &Value) -> Option<i64> {
    if let Some(integer) = value.as_i64() {
        return Some(integer);
    }
    let number = value.as_number()?.to_string();
    let (mantissa, exponent) = number.split_once(['e', 'E']).map_or((number.as_str(), 0), |(m, e)| (m, e.parse::<i32>().unwrap_or(i32::MAX)));
    let decimals = mantissa.split_once('.').map_or(0, |(_, tail)| tail.len() as i32);
    let digits = mantissa.replace('.', "");
    let mut result = digits.parse::<i128>().ok()?;
    if result == 0 {
        return Some(0);
    }
    let power = exponent.checked_sub(decimals)?;
    if power >= 0 {
        result = result.checked_mul(10_i128.checked_pow(power.try_into().ok()?)?)?;
    } else {
        let divisor = 10_i128.checked_pow(power.checked_neg()?.try_into().ok()?)?;
        if result % divisor != 0 {
            return None;
        }
        result /= divisor;
    }
    result.try_into().ok()
}

/// Normalize only the command's numeric fields; saved records retain their canonical syntax checks.
fn normalize_number(value: &mut Value) {
    if let Some(integer) = integer(value) {
        *value = json!(integer);
    }
}

/// Parse command content, sorting corrections and accepting integer-valued JSON numbers.
/// Omitted convention defaults to viewed-from-top; stored revisions always state it explicitly.
fn parse(value: &Value) -> Result<Option<ProfileContent>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.entry("bottom_side").or_insert(json!(BottomSide::default()));
        if let Some(revision) = object.get_mut("revision") {
            normalize_number(revision);
        }
        if let Some(corrections) = object.get_mut("corrections").and_then(Value::as_array_mut) {
            if corrections.len() > MAX_CORRECTIONS {
                return Err(format!("An assembly profile may carry at most {MAX_CORRECTIONS} corrections."));
            }
            for correction in corrections.iter_mut().filter_map(Value::as_object_mut) {
                if let Some(rotation) = correction.get_mut("rotation") {
                    normalize_number(rotation);
                }
                if correction.get("rotation") == Some(&json!(0)) {
                    correction.remove("rotation");
                }
                if let Some(centroid) = correction.get_mut("centroid").and_then(Value::as_object_mut) {
                    for axis in ["x_nm", "y_nm"] {
                        if let Some(number) = centroid.get_mut(axis) {
                            normalize_number(number);
                        }
                    }
                }
            }
        }
    }
    let mut profile: ProfileContent = serde_json::from_value(value).map_err(|e| e.to_string())?;
    Arc::make_mut(&mut profile.corrections).sort_by(|a, b| (&a.library, &a.definition).cmp(&(&b.library, &b.definition)));
    profile.validate()?;
    Ok(Some(profile))
}

/// Whether the command content is valid; uses the same parser as the command, not a schema surrogate.
pub(crate) fn valid_profile(value: &Value) -> bool {
    parse(value).is_ok()
}

impl Session {
    /// Pin immutable content or retire a profile. Conflicting content at an existing revision is refused.
    /// Cleanup is transactional, retains all board adoptions, and cannot alter older Undo snapshots.
    pub(crate) fn set_assembly_profile(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let mut candidate = self.open_project()?.clone();
        let id = match params.get("profile") {
            Some(value) => serde_json::from_value::<Uuid>(value.clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?,
            None => crate::fresh_id()?,
        };
        let content = parse(&params["content"]).map_err(CommandError::InvalidParams)?;
        let catalog = Arc::make_mut(&mut candidate.assembly_profiles);
        match content {
            Some(mut content) => {
                if let Some(head) = catalog.heads.iter().find(|head| head.profile == id)
                    && let Some(previous) = catalog.get(&Adoption { profile: id, revision: head.revision })
                {
                    content.share_corrections(&previous.content);
                }
                let record = AssemblyProfile { format: FORMAT.into(), id, content };
                let adoption = Adoption { profile: id, revision: record.content.revision };
                if let Some(existing) = catalog.get(&adoption) {
                    return if existing == &record {
                        Ok(())
                    } else {
                        Err(CommandError::InvalidParams(
                            "Assembly profile content conflict: the same ID and revision already have different content.".into(),
                        ))
                    };
                }
                if let Some(head) = catalog.heads.iter_mut().find(|h| h.profile == id) {
                    if record.content.revision <= head.revision {
                        return Err(CommandError::InvalidParams(
                            "Assembly profile revision conflict: this revision was already superseded or removed.".into(),
                        ));
                    }
                    head.revision = record.content.revision;
                    head.removed = false;
                } else {
                    catalog.heads.push(ProfileHead { profile: id, revision: record.content.revision, removed: false });
                }
                catalog.revisions.push(Arc::new(record));
                catalog.revisions.sort_by_key(|r| (r.id, r.content.revision));
                catalog.heads.sort_by_key(|h| h.profile);
            }
            None => {
                let head = catalog
                    .heads
                    .iter_mut()
                    .find(|h| h.profile == id)
                    .ok_or_else(|| CommandError::InvalidParams("Name an existing assembly profile to remove.".into()))?;
                if head.removed {
                    return Ok(());
                }
                head.removed = true;
            }
        }
        catalog.cleanup(&candidate.board.assembly_profiles);
        catalog.validate(&candidate.board.assembly_profiles).map_err(CommandError::InvalidParams)?;
        self.change(candidate)
    }

    /// Compute the complete adoption review, including the actual old and corrected placement values.
    /// The acceptance token binds the project opening, electrical revision, target and review bytes.
    pub(crate) fn preview_assembly_profile(&self, params: &Value) -> Result<Value, CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let board = project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let view = project.board_view(board).ok_or_else(|| CommandError::InvalidParams("Unknown board.".into()))?;
        let id: Uuid = serde_json::from_value(params["profile"].clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
        let revision = params
            .get("revision")
            .filter(|v| !v.is_null())
            .map(|v| {
                integer(v)
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| *n > 0)
                    .ok_or_else(|| CommandError::InvalidParams("Name a positive content revision.".into()))
            })
            .transpose()?;
        let before = project.board.assembly_profiles.iter().find(|a| a.profile == id);
        let after = revision.map(|revision| Adoption { profile: id, revision });
        let previous = before.and_then(|a| project.assembly_profiles.get(a));
        let proposed = match &after {
            Some(a) => {
                if before != Some(a) && project.assembly_profiles.heads.iter().any(|h| h.profile == id && h.removed) {
                    return Err(CommandError::InvalidParams("A removed assembly profile cannot be newly adopted.".into()));
                }
                Some(
                    project
                        .assembly_profiles
                        .get(a)
                        .ok_or_else(|| CommandError::InvalidParams("No pinned assembly profile at that revision.".into()))?,
                )
            }
            None => None,
        };
        let keys: BTreeSet<_> =
            previous.into_iter().chain(proposed).flat_map(|p| p.content.corrections.iter().map(|c| (&c.library, &c.definition))).collect();
        let mut corrections = Vec::new();
        for (library, definition) in keys {
            let lookup = |profile: Option<&AssemblyProfile>| {
                profile.and_then(|p| {
                    p.content
                        .corrections
                        .binary_search_by(|c| (&c.library, &c.definition).cmp(&(library, definition)))
                        .ok()
                        .map(|index| p.content.corrections[index].clone())
                })
            };
            let old = lookup(previous);
            let new = lookup(proposed);
            if old != new {
                corrections.push(json!({"library":library,"definition":definition,"before":old,"after":new}));
            }
        }
        let mut rows = Vec::new();
        for footprint in project.board.footprints() {
            let key = format!("footprint-{}", footprint.id);
            let old = view.profile_row(&key, previous).map_err(CommandError::InvalidParams)?;
            let new = view.profile_row(&key, proposed).map_err(CommandError::InvalidParams)?;
            if old != new {
                rows.push(json!({"footprint":key,"before":old,"after":new}));
            }
        }
        let mut review = json!({"project":project.id(),"opening":self.history_root_revision(),"board":board.board,"expected_revision":self.revision(),"profile":id,
            "before":before,"after":after,"changed_corrections":corrections,"affected_rows":rows,
            "bottom_side":{"before":previous.map_or_else(BottomSide::default, |p| p.content.bottom_side),"after":proposed.map_or_else(BottomSide::default, |p| p.content.bottom_side)}});
        use sha2::{Digest, Sha256};
        let bytes = serde_json::to_vec(&review).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
        review["acceptance"] = json!(format!("{:x}", Sha256::digest(bytes)));
        Ok(review)
    }

    /// Accept only the current preview. Revision or token mismatch leaves project and history intact.
    pub(crate) fn adopt_assembly_profile(&mut self, params: &Value) -> Result<(), CommandError> {
        let preview = self.preview_assembly_profile(params)?;
        if params.get("acceptance") != preview.get("acceptance") {
            return Err(CommandError::InvalidParams("Assembly profile preview changed; preview again before accepting.".into()));
        }
        let id: Uuid = serde_json::from_value(params["profile"].clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
        let after: Option<Adoption> = serde_json::from_value(preview["after"].clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
        let mut candidate = self.open_project()?.clone();
        let board = Arc::make_mut(&mut candidate.board);
        board.assembly_profiles.retain(|a| a.profile != id);
        board.assembly_profiles.extend(after);
        board.assembly_profiles.sort_by_key(|a| a.profile);
        if board.assembly_profiles == self.open_project()?.board.assembly_profiles {
            return Ok(());
        }
        let catalog = Arc::make_mut(&mut candidate.assembly_profiles);
        catalog.cleanup(&candidate.board.assembly_profiles);
        catalog.validate(&candidate.board.assembly_profiles).map_err(CommandError::InvalidParams)?;
        self.change(candidate)
    }
}

#[cfg(test)]
mod tests;
