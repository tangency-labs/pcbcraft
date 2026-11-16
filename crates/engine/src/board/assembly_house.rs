//! Assembly house profiles: how one house wants placement rows of this board written.
//!
//! A house may count a package's zero orientation or its centroid differently from the
//! library footprint, for example a connector whose reel orientation differs from the
//! IPC-7351 zero component orientation. Those differences are saved per package here, so
//! placement rows for that house come out right without editing an exported file.
use super::{
    Footprint,
    definition::{self, MAX_OFFSET_NM},
    origin::{BottomSide, Placed},
    view::BoardView,
};
use crate::{CommandError, Session, schematic::Point};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

/// Create, replace or remove one assembly house profile on the addressed board.
pub const SET_ASSEMBLY_HOUSE: &str = "pcb.set_assembly_house";
/// The saved vocabulary of a house profile record.
const FORMAT: &str = "pcbcraft-assembly-house/1";
/// Profiles one board may carry.
pub const MAX_HOUSES: usize = 64;
/// Package offsets one profile may carry.
pub const MAX_PACKAGES: usize = 4096;
/// Longest house name, in characters.
pub const MAX_NAME_CHARS: usize = 64;

/// How one house's orientation and centroid of a library package differ from the library's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageOffset {
    /// Library of the footprint definition.
    pub library: String,
    /// Footprint definition id; the offset applies to every revision of it.
    pub definition: String,
    /// Counterclockwise whole degrees, 0 through 359, from the library zero orientation to the
    /// house's zero orientation, seen from the component side. Omitted when zero.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotation: u16,
    /// The house's centroid in library footprint coordinates (integer nanometres, Y down, as
    /// pads are given), which rows for this house report instead of the footprint origin.
    /// Omitted when the house uses the footprint origin.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "super::reference::present")]
    pub centroid: Option<Point>,
}

/// Rotation offsets of zero are not saved.
fn is_zero(rotation: &u16) -> bool {
    *rotation == 0
}

impl PackageOffset {
    /// An entry must name a definition and change something, within the footprint offset range.
    fn validate(&self) -> Result<(), String> {
        if !definition::valid_id(&self.library) || !definition::valid_id(&self.definition) {
            return Err("A package offset needs a valid library and footprint definition id.".into());
        }
        if self.rotation >= 360 {
            return Err("A package rotation offset must be a whole number of degrees from 0 through 359.".into());
        }
        if let Some(centroid) = self.centroid
            && (centroid == (Point { x_nm: 0, y_nm: 0 })
                || ![centroid.x_nm, centroid.y_nm].into_iter().all(|n| (-MAX_OFFSET_NM..=MAX_OFFSET_NM).contains(&n)))
        {
            return Err("A package centroid offset must be nonzero and within 100 mm of the footprint origin; omit it to use the origin.".into());
        }
        if self.rotation == 0 && self.centroid.is_none() {
            return Err(format!("The package offset for {}/{} changes nothing; remove it instead.", self.library, self.definition));
        }
        Ok(())
    }

    /// The house's row for a footprint whose generic row under the same convention is `row`.
    ///
    /// The rotation offset turns the library footprint into the house's zero orientation on
    /// the component side. A top row and a bottom row seen from below show that side as is,
    /// so the offset adds; a bottom row seen from the top shows it mirrored, so it subtracts.
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

/// The editable part of a house profile, as a command supplies it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    name: String,
    #[serde(default)]
    bottom_side: BottomSide,
    #[serde(default)]
    packages: Vec<PackageOffset>,
}

/// One assembly house's conventions for this board, with a stable identity outputs can name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct House {
    /// Supported vocabulary, currently pcbcraft-assembly-house/1.
    pub format: String,
    /// Stable identity retained while the profile is edited.
    pub id: Uuid,
    /// Display name, unique on the board.
    pub name: String,
    /// How this house wants bottom-side rows written.
    pub bottom_side: BottomSide,
    /// Per-package offsets in library and definition order, one per definition.
    #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "listed")]
    pub packages: Vec<PackageOffset>,
}

/// A saved package list is omitted when empty, so an empty array is refused.
fn listed<'de, D>(deserializer: D) -> Result<Vec<PackageOffset>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let packages = Vec::<PackageOffset>::deserialize(deserializer)?;
    if packages.is_empty() {
        return Err(serde::de::Error::custom("an empty packages array is not allowed"));
    }
    Ok(packages)
}

impl House {
    /// Unsupported versions, nil identities, unusable names and unordered, duplicate or empty
    /// package offsets are refused on load and before saving.
    pub fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT || self.id.is_nil() {
            return Err("Unsupported assembly house format or invalid house identity.".into());
        }
        validate_profile(&self.name, &self.packages)
    }

    /// The offset saved for a placed footprint's package, if any. Legacy marker footprints have
    /// no library identity and never match.
    pub(crate) fn offset_for(&self, footprint: &Footprint) -> Option<&PackageOffset> {
        let library = footprint.library.as_deref()?;
        let key = (library, footprint.definition.as_str());
        self.packages
            .binary_search_by(|package| (package.library.as_str(), package.definition.as_str()).cmp(&key))
            .ok()
            .map(|index| &self.packages[index])
    }
}

/// A usable name and valid package offsets, unique and in library and definition order.
fn validate_profile(name: &str, packages: &[PackageOffset]) -> Result<(), String> {
    let length = name.chars().count();
    // White space as the catalog pattern's `\s` counts it, which also includes U+FEFF.
    let space = |c: char| c.is_whitespace() || c == '\u{feff}';
    if length == 0 || length > MAX_NAME_CHARS || name.trim_matches(space) != name || name.chars().any(char::is_control) {
        return Err(format!("An assembly house name needs 1 to {MAX_NAME_CHARS} characters without surrounding spaces or control characters."));
    }
    if packages.len() > MAX_PACKAGES {
        return Err(format!("An assembly house may carry at most {MAX_PACKAGES} package offsets."));
    }
    for package in packages {
        package.validate()?;
    }
    if packages.windows(2).any(|pair| (&pair[0].library, &pair[0].definition) >= (&pair[1].library, &pair[1].definition)) {
        return Err("Assembly house package offsets must be unique and in library and definition order.".into());
    }
    Ok(())
}

/// A board's houses must be valid, at most [`MAX_HOUSES`], with unique ids and names.
pub(crate) fn validate_all(houses: &[House]) -> Result<(), String> {
    if houses.len() > MAX_HOUSES {
        return Err(format!("A board may carry at most {MAX_HOUSES} assembly houses."));
    }
    for (index, house) in houses.iter().enumerate() {
        house.validate()?;
        if houses[..index].iter().any(|other| other.id == house.id || other.name == house.name) {
            return Err(format!("Assembly house \"{}\" repeats an identity or name on this board.", house.name));
        }
    }
    Ok(())
}

impl<'a> BoardView<'a> {
    /// The assembly house profiles saved on this board, in creation order.
    pub fn assembly_houses(self) -> &'a [House] {
        &self.project().board.assembly_houses
    }

    /// One placement row as written for `house`, or the generic row without one.
    ///
    /// Rows are measured from the board's placement origin. The generic row writes bottom
    /// parts seen from the top and the footprint origin with the board rotation; a house row
    /// uses the house's bottom-side convention and adds the house's offsets for that package.
    /// Read-only. Fails when the footprint or house is not on this board, or the placement
    /// origin cannot be resolved.
    pub fn placement_row(self, footprint: &str, house: Option<Uuid>) -> Result<Placed, String> {
        let placement = self.placement(footprint).ok_or_else(|| format!("No footprint {footprint} on this board."))?;
        let frame = self.placement_origin()?.frame();
        let house = match house {
            Some(id) => {
                Some(self.assembly_houses().iter().find(|house| house.id == id).ok_or_else(|| format!("No assembly house {id} on this board."))?)
            }
            None => None,
        };
        let convention = house.map_or_else(BottomSide::default, |house| house.bottom_side);
        let at = Point { x_nm: placement.x_nm, y_nm: placement.y_nm };
        let row = frame.place(at, placement.rotation, placement.bottom, convention);
        Ok(match house.and_then(|house| house.offset_for(placement)) {
            Some(offset) => offset.apply(placement, row, frame, convention),
            None => row,
        })
    }
}

/// The catalog accepts one house profile object, or null to remove the addressed house.
pub(crate) fn schema() -> Value {
    let coordinate = json!({"type":"integer","minimum":-MAX_OFFSET_NM,"maximum":MAX_OFFSET_NM});
    let id = json!({"type":"string","minLength":1,"maxLength":64,"pattern":"^[a-z0-9]([a-z0-9-]*[a-z0-9])?$"});
    // No control characters and no leading or trailing white space, as `validate_profile` requires.
    let name = "^[^\\s\\u0000-\\u001f\\u007f-\\u009f]([^\\u0000-\\u001f\\u007f-\\u009f]*[^\\s\\u0000-\\u001f\\u007f-\\u009f])?$";
    json!({"oneOf":[
        {"type":"null"},
        {"type":"object","properties":{
            "name":{"type":"string","minLength":1,"maxLength":MAX_NAME_CHARS,"pattern":name},
            "bottom_side":{"type":"string","enum":["viewed_from_top","viewed_from_bottom"]},
            "packages":{"type":"array","maxItems":MAX_PACKAGES,"items":{"type":"object","properties":{
                "library":id,"definition":id,
                "rotation":{"type":"integer","minimum":0,"maximum":359},
                "centroid":{"type":"object","properties":{"x_nm":coordinate,"y_nm":coordinate},"required":["x_nm","y_nm"],"additionalProperties":false,
                    "not":{"properties":{"x_nm":{"const":0},"y_nm":{"const":0}}}}
            },"required":["library","definition"],"additionalProperties":false,
            "anyOf":[{"required":["rotation"],"properties":{"rotation":{"minimum":1}}},{"required":["centroid"]}]}}
        },"required":["name"],"additionalProperties":false}
    ]})
}

/// Parse a request's profile into canonical package order, refusing unknown fields, invalid
/// offsets and repeated packages. Null means removal.
fn parse(value: &Value) -> Result<Option<Profile>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let mut profile: Profile = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    profile.packages.sort_by(|a, b| (&a.library, &a.definition).cmp(&(&b.library, &b.definition)));
    if let Some(pair) = profile.packages.windows(2).find(|pair| (&pair[0].library, &pair[0].definition) == (&pair[1].library, &pair[1].definition)) {
        return Err(format!("The package {}/{} is listed more than once.", pair[0].library, pair[0].definition));
    }
    validate_profile(&profile.name, &profile.packages)?;
    Ok(Some(profile))
}

/// Whether a request's profile parses; the command catalog checks parameters with this.
pub(crate) fn valid_profile(value: &Value) -> bool {
    parse(value).is_ok()
}

impl Session {
    /// Create a house (no `house` id), replace one keeping its UUID and position, or remove one
    /// with a null profile, in one revision-checked undo step. Repeating the current profile
    /// does not advance history. Names must stay unique on the board.
    pub(crate) fn set_assembly_house(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let target = match params.get("house") {
            Some(value) => Some(
                serde_json::from_value::<Uuid>(value.clone())
                    .ok()
                    .filter(|id| !id.is_nil())
                    .ok_or_else(|| CommandError::InvalidParams("house must identify an assembly house by non-nil UUID".into()))?,
            ),
            None => None,
        };
        let profile = parse(&params["profile"]).map_err(CommandError::InvalidParams)?;
        let mut houses = project.board.assembly_houses.clone();
        let index = match target {
            Some(id) => Some(
                houses
                    .iter()
                    .position(|house| house.id == id)
                    .ok_or_else(|| CommandError::InvalidParams(format!("No assembly house {id} on this board.")))?,
            ),
            None => None,
        };
        match (index, profile) {
            (Some(index), None) => {
                houses.remove(index);
            }
            (Some(index), Some(Profile { name, bottom_side, packages })) => {
                let house = &mut houses[index];
                (house.name, house.bottom_side, house.packages) = (name, bottom_side, packages);
            }
            (None, Some(Profile { name, bottom_side, packages })) => {
                houses.push(House { format: FORMAT.into(), id: crate::fresh_id()?, name, bottom_side, packages });
            }
            (None, None) => return Err(CommandError::InvalidParams("Name the house to remove.".into())),
        }
        if houses == project.board.assembly_houses {
            return Ok(());
        }
        validate_all(&houses).map_err(CommandError::InvalidParams)?;
        let mut candidate = project.clone();
        Arc::make_mut(&mut candidate.board).assembly_houses = houses;
        self.change(candidate)
    }
}

#[cfg(test)]
mod tests;
