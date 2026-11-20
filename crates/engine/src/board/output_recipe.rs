//! Authored fabrication selections. A recipe is not an export run or a destination.

use super::{assembly_profile::Adoption, origin::Anchor};
use crate::{
    CommandError, ParamKind, Project, Session,
    persistence::layout::{RESERVED_PORTABLE_SEGMENTS, portable_segment},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

/// Replace one authored recipe or remove it with null.
pub const SET_OUTPUT_RECIPE: &str = "pcb.set_output_recipe";
const MAX_RECIPES: usize = 32;
const MAX_RECIPE_LAYERS: usize = 128;
const MAX_RECIPE_NAME: usize = 80;
const MAX_FILE_STEM: usize = 48;
/// Trimmed, nonempty recipe name without C0/C1 control characters.
const RECIPE_NAME_PATTERN: &str =
    "^[^\\s\\u0000-\\u001f\\u007f-\\u009f](?:[^\\u0000-\\u001f\\u007f-\\u009f]*[^\\s\\u0000-\\u001f\\u007f-\\u009f])?(?![\\s\\S])";
/// Lowercase ASCII file stem with interior hyphens only.
const FILE_STEM_PATTERN: &str = "^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?![\\s\\S])";

/// The desired output units, not the units of authored board coordinates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", try_from = "String")]
pub enum Units {
    Mm,
    In,
}

impl TryFrom<String> for Units {
    type Error = &'static str;

    /// Accept the two published string spellings, not externally tagged objects.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "mm" => Ok(Self::Mm),
            "in" => Ok(Self::In),
            _ => Err("output recipe units must be mm or in"),
        }
    }
}

/// One saved recipe identified independently of its display name and board path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub id: Uuid,
    pub content: Content,
}

/// The authored selections shared by the native codec and the edit command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Content {
    pub name: String,
    pub board: Uuid,
    pub units: Units,
    pub file_stem: String,
    pub gerber_layers: Vec<Uuid>,
    pub drill: bool,
    #[serde(default, deserialize_with = "crate::persistence::present", skip_serializing_if = "Option::is_none")]
    pub origin: Option<Anchor>,
    #[serde(default, deserialize_with = "crate::persistence::present", skip_serializing_if = "Option::is_none")]
    pub assembly_profile: Option<Adoption>,
}

impl Content {
    /// Validate authored data without resolving layers or profile revisions at save time.
    /// A later design change may leave a selection unresolved; that is not a save error.
    fn validate(&self) -> Result<(), String> {
        if self.board.is_nil()
            || self.name.is_empty()
            || self.name.trim() != self.name
            || self.name.chars().count() > MAX_RECIPE_NAME
            || self.name.chars().any(char::is_control)
        {
            return Err("An output recipe requires a board UUID and a trimmed, nonempty name of at most 80 characters.".into());
        }
        if self.file_stem.len() > MAX_FILE_STEM || !portable_segment(&self.file_stem) {
            return Err("The output recipe file stem must be a portable name of at most 48 ASCII characters.".into());
        }
        if self.gerber_layers.len() > MAX_RECIPE_LAYERS
            || self.gerber_layers.iter().any(Uuid::is_nil)
            || self.gerber_layers.iter().collect::<BTreeSet<_>>().len() != self.gerber_layers.len()
            || (!self.drill && self.gerber_layers.is_empty())
        {
            return Err("Select distinct non-nil Gerber layer UUIDs (at most 128), drill, or both.".into());
        }
        if self.assembly_profile.as_ref().is_some_and(|address| address.profile.is_nil() || address.revision == 0) {
            return Err("An output recipe assembly profile address requires a non-nil UUID and positive revision.".into());
        }
        Ok(())
    }
}

/// Check the native catalog against its explicitly registered board and recipe identity bounds.
pub(crate) fn validate_recipes(recipes: &[Recipe], board: Option<Uuid>) -> Result<(), String> {
    if recipes.len() > MAX_RECIPES {
        return Err(format!("A project can store at most {MAX_RECIPES} output recipes."));
    }
    let mut ids = BTreeSet::new();
    for recipe in recipes {
        if recipe.id.is_nil() || !ids.insert(recipe.id) || Some(recipe.content.board) != board {
            return Err("Output recipes require distinct non-nil IDs and must target the registered board.".into());
        }
        recipe.content.validate()?;
    }
    Ok(())
}

/// Describe a complete recipe replacement, or null to remove it.
pub(crate) fn schema() -> Value {
    let id = ParamKind::OutputRecipeId.schema();
    let origin = super::origin::schema();
    json!({"oneOf":[{"type":"null"},{"type":"object","additionalProperties":false,
        "required":["name","board","units","file_stem","gerber_layers","drill"],
        "anyOf":[{"properties":{"drill":{"const":true}}},{"properties":{"gerber_layers":{"minItems":1}}}],"properties":{
            "name":{"type":"string","minLength":1,"maxLength":MAX_RECIPE_NAME,"pattern":RECIPE_NAME_PATTERN},
            "board":id,
            "units":{"type":"string","enum":["mm","in"]},
            "file_stem":{"type":"string","minLength":1,"maxLength":MAX_FILE_STEM,"pattern":FILE_STEM_PATTERN,"not":{"enum":RESERVED_PORTABLE_SEGMENTS}},
            "gerber_layers":{"type":"array","maxItems":MAX_RECIPE_LAYERS,"uniqueItems":true,"items":id},
            "drill":{"type":"boolean"},
            "origin":{"oneOf":origin["oneOf"].as_array().into_iter().flatten().filter(|item| item["type"] != "null").collect::<Vec<_>>()},
            "assembly_profile":{"type":"object","additionalProperties":false,"required":["profile","revision"],
                "properties":{"profile":id,"revision":{"type":"integer","minimum":1,"maximum":u32::MAX}}}
        }
    }]})
}

/// Decode the same content type used in native documents and apply its intrinsic bounds.
fn parse(value: &Value) -> Result<Option<Content>, String> {
    let content: Option<Content> = serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
    if let Some(content) = &content {
        content.validate()?;
    }
    Ok(content)
}

/// Validate the command content without a project; the edit will check board ownership.
pub(crate) fn valid_content(value: &Value) -> bool {
    parse(value).is_ok()
}

impl Project {
    /// Authored recipes in stable ID order; output runs are not part of this collection.
    pub fn output_recipes(&self) -> &[Recipe] {
        &self.output_recipes
    }
}

impl Session {
    /// Edit one recipe at the supplied project revision. Publish no state on refusal or no-op.
    /// The first recipe adopts native folder and occurrence metadata in the same undo step.
    pub(crate) fn set_output_recipe(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let id: Uuid = serde_json::from_value(params["recipe"].clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?;
        let content = parse(&params["content"]).map_err(CommandError::InvalidParams)?;
        let mut candidate = project.clone();
        let recipes = Arc::make_mut(&mut candidate.output_recipes);
        match (recipes.binary_search_by_key(&id, |recipe| recipe.id), content) {
            (Ok(index), Some(content)) if recipes[index].content == content => return Ok(()),
            (Ok(index), Some(content)) => recipes[index].content = content,
            (Ok(index), None) => {
                recipes.remove(index);
            }
            (Err(index), Some(content)) => recipes.insert(index, Recipe { id, content }),
            (Err(_), None) => return Ok(()),
        }
        validate_recipes(recipes, project.board_target().map(|target| target.board)).map_err(CommandError::InvalidParams)?;
        if !recipes.is_empty() {
            if candidate.layout.is_none() {
                candidate = candidate.with_board_folders(".").map_err(CommandError::InvalidParams)?;
            }
            if !candidate.has_sheet_occurrences() {
                candidate = candidate.with_sheet_occurrences().map_err(CommandError::InvalidParams)?;
            }
        }
        self.change(candidate)
    }
}

#[cfg(test)]
mod tests;
