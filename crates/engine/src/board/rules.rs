//! Board-owned dimensions and one effective-rule resolver for GUI, MCP and geometry consumers.
use super::{track, view::BoardView};
use crate::{CommandError, Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use uuid::Uuid;

/// Replace the addressed board's defaults, or remove authored defaults with explicit null.
pub const SET_RULES: &str = "pcb.set_rules";
/// Hard dimensions accepted as request-local diagnostic overrides.
pub const LIMIT_FIELDS: &[&str] = &[
    "clearance_nm",
    "min_track_width_nm",
    "edge_clearance_nm",
    "min_annular_ring_nm",
    "min_via_diameter_nm",
    "max_via_diameter_nm",
    "min_via_drill_nm",
    "max_via_drill_nm",
];
/// Preferred upcoming dimensions are independent of hard bounds and existing authored copper.
pub const PREFERENCE_FIELDS: &[&str] = &["preferred_track_width_nm", "preferred_via_diameter_nm", "preferred_via_drill_nm"];

/// Integer nanometres with explicit minimum/maximum and preferred semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Values {
    /// Minimum copper spacing between different intended nets; equality passes.
    pub clearance_nm: i64,
    /// Hard minimum track width; zero disables this diagnostic.
    pub min_track_width_nm: i64,
    /// Minimum copper-to-board-outline spacing.
    pub edge_clearance_nm: i64,
    /// Minimum radial copper around each routed via drill.
    pub min_annular_ring_nm: i64,
    /// Hard lower bound on routed via outer diameter.
    pub min_via_diameter_nm: i64,
    /// Hard upper bound on routed via outer diameter.
    pub max_via_diameter_nm: i64,
    /// Hard lower bound on routed via drill diameter.
    pub min_via_drill_nm: i64,
    /// Hard upper bound on routed via drill diameter.
    pub max_via_drill_nm: i64,
    /// Initial upcoming track width, independently validated against hard limits.
    pub preferred_track_width_nm: i64,
    /// Initial upcoming via outer diameter.
    pub preferred_via_diameter_nm: i64,
    /// Initial upcoming via drill diameter, paired with the preferred outer diameter.
    pub preferred_via_drill_nm: i64,
}

impl Default for Values {
    /// Preserve released DRC tolerances and route sizes; via bounds match supported geometry.
    fn default() -> Self {
        Self {
            clearance_nm: super::drc::DEFAULT_CLEARANCE_NM,
            min_track_width_nm: super::drc::DEFAULT_MIN_TRACK_WIDTH_NM,
            edge_clearance_nm: super::drc::DEFAULT_EDGE_CLEARANCE_NM,
            min_annular_ring_nm: super::drc::DEFAULT_MIN_ANNULAR_RING_NM,
            min_via_diameter_nm: track::MIN_VIA_DRILL_NM + 1,
            max_via_diameter_nm: track::MAX_VIA_DIAMETER_NM,
            min_via_drill_nm: track::MIN_VIA_DRILL_NM,
            max_via_drill_nm: track::MAX_VIA_DIAMETER_NM - 1,
            preferred_track_width_nm: track::DEFAULT_WIDTH_NM,
            preferred_via_diameter_nm: track::DEFAULT_VIA_DIAMETER_NM,
            preferred_via_drill_nm: track::DEFAULT_VIA_DRILL_NM,
        }
    }
}

impl Values {
    /// Reject unsupported, reversed or impossible hard bounds before publishing a rule change.
    fn validate_limits(&self) -> Result<(), String> {
        for (name, value) in
            [("clearance", self.clearance_nm), ("edge clearance", self.edge_clearance_nm), ("annular ring", self.min_annular_ring_nm)]
        {
            if !(0..=crate::schematic::MAX_CENTER_NM).contains(&value) {
                return Err(format!("{name} must be within the board coordinate range"));
            }
        }
        if !(0..=track::MAX_WIDTH_NM).contains(&self.min_track_width_nm)
            || !(track::MIN_VIA_DRILL_NM + 1..=track::MAX_VIA_DIAMETER_NM).contains(&self.min_via_diameter_nm)
            || !(self.min_via_diameter_nm..=track::MAX_VIA_DIAMETER_NM).contains(&self.max_via_diameter_nm)
            || !(track::MIN_VIA_DRILL_NM..track::MAX_VIA_DIAMETER_NM).contains(&self.min_via_drill_nm)
            || !(self.min_via_drill_nm..track::MAX_VIA_DIAMETER_NM).contains(&self.max_via_drill_nm)
            || self.min_via_drill_nm + (2 * self.min_annular_ring_nm).max(1) > self.max_via_diameter_nm
        {
            return Err("Rule bounds are unsupported, reversed or leave no legal via size.".into());
        }
        Ok(())
    }

    /// Preferences must fit every hard bound. Changing rules never resizes existing copper.
    pub fn validate(&self) -> Result<(), String> {
        self.validate_limits()?;
        self.validate_sizes(self.preferred_track_width_nm, self.preferred_via_diameter_nm, self.preferred_via_drill_nm)
    }

    /// Validate upcoming route sizes against supported geometry and the effective hard limits.
    pub fn validate_sizes(&self, width: i64, diameter: i64, drill: i64) -> Result<(), String> {
        self.validate_limits()?;
        if !(track::MIN_WIDTH_NM.max(self.min_track_width_nm)..=track::MAX_WIDTH_NM).contains(&width)
            || !(self.min_via_diameter_nm..=self.max_via_diameter_nm).contains(&diameter)
            || !(self.min_via_drill_nm..=self.max_via_drill_nm).contains(&drill)
            || diameter - drill < (2 * self.min_annular_ring_nm).max(1)
        {
            return Err("Routing dimensions must satisfy the board's width, via, drill and annular-ring limits.".into());
        }
        Ok(())
    }
}

/// A stable board-default rule identity and explicitly versioned value vocabulary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Supported vocabulary, currently pcbcraft-rules/1.
    pub format: String,
    /// Stable identity retained while changing this board-default record.
    pub id: Uuid,
    /// Resolved or authored dimensions; preferences never substitute for hard bounds.
    pub values: Values,
}
impl Settings {
    /// Unsupported versions, nil identities and contradictory dimensions are refused on load.
    pub fn validate(&self) -> Result<(), String> {
        if self.format != "pcbcraft-rules/1" || self.id.is_nil() {
            return Err("Unsupported board rule format or invalid rule identity.".into());
        }
        self.values.validate()
    }
}

/// Provenance is explicit even when a request happens to supply the existing value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// Released fallback when the board has no authored record.
    Builtin,
    /// Authored default of the addressed board, qualified by the stored rule UUID.
    Board { rule: Uuid },
    /// Diagnostic override used for this request only.
    Request,
}

/// A resolved value set and per-property sources, derived without dirtying the project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Effective {
    /// Resolved or authored dimensions; preferences never substitute for hard bounds.
    pub values: Values,
    /// One explicit source for each typed property, keyed by its stable schema field name.
    pub sources: BTreeMap<String, Source>,
}

impl<'a> BoardView<'a> {
    /// Return authored intent, independently from effective values and application preferences.
    pub fn authored_rules(self) -> Option<&'a Settings> {
        self.project().board.rules.as_ref()
    }

    /// Resolve supported board defaults without allocation of identities or state changes.
    pub fn effective_rules(self) -> Effective {
        let (values, source) = self.authored_rules().map_or((Values::default(), Source::Builtin), |s| (s.values, Source::Board { rule: s.id }));
        Effective { values, sources: LIMIT_FIELDS.iter().chain(PREFERENCE_FIELDS).map(|name| ((*name).into(), source.clone())).collect() }
    }

    /// Diagnostic overrides replace built-ins on legacy boards; authored hard limits may only
    /// tighten. They never change saved intent or upcoming preferred routing dimensions.
    pub fn resolve_rules(self, params: &Value) -> Result<Effective, CommandError> {
        let mut effective = self.effective_rules();
        let mut value = json!(effective.values);
        for name in LIMIT_FIELDS {
            if let Some(given) = params.get(*name) {
                let n = given.as_i64().ok_or_else(|| CommandError::InvalidParams(format!("{name} must be integer nanometres")))?;
                let bound = value[*name].as_i64().ok_or_else(|| CommandError::InvalidParams("Invalid rule value".into()))?;
                if self.authored_rules().is_some() && if name.starts_with("max_") { n > bound } else { n < bound } {
                    return Err(CommandError::InvalidParams(format!("{name} cannot weaken an authored board limit")));
                }
                value[*name] = json!(n);
                effective.sources.insert((*name).into(), Source::Request);
            }
        }
        effective.values = serde_json::from_value(value).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
        effective.values.validate_limits().map_err(CommandError::InvalidParams)?;
        Ok(effective)
    }
}

/// The shared catalog accepts complete values or explicit null to restore built-in behavior.
pub(crate) fn schema() -> Value {
    let properties: BTreeMap<_, _> = LIMIT_FIELDS
        .iter()
        .chain(PREFERENCE_FIELDS)
        .map(|name| {
            let (min, max) = match *name {
                "min_track_width_nm" => (0, track::MAX_WIDTH_NM),
                "preferred_track_width_nm" => (track::MIN_WIDTH_NM, track::MAX_WIDTH_NM),
                "min_via_drill_nm" | "max_via_drill_nm" | "preferred_via_drill_nm" => (track::MIN_VIA_DRILL_NM, track::MAX_VIA_DIAMETER_NM - 1),
                "min_via_diameter_nm" | "max_via_diameter_nm" | "preferred_via_diameter_nm" => {
                    (track::MIN_VIA_DRILL_NM + 1, track::MAX_VIA_DIAMETER_NM)
                }
                _ => (0, crate::schematic::MAX_CENTER_NM),
            };
            (*name, json!({"type":"integer","minimum":min,"maximum":max}))
        })
        .collect();
    json!({"oneOf":[{"type":"null"},{"type":"object","properties":properties,"required":LIMIT_FIELDS.iter().chain(PREFERENCE_FIELDS).collect::<Vec<_>>(),"additionalProperties":false}]})
}

impl Session {
    /// Replace all board defaults in one revision-checked undo step, retaining an existing rule
    /// UUID. Null removes intent; identical values and repeated resets do not advance history.
    pub(crate) fn set_rules(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let next = if params["rules"].is_null() {
            None
        } else {
            let values: Values = serde_json::from_value(params["rules"].clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
            values.validate().map_err(CommandError::InvalidParams)?;
            if project.board.rules.as_ref().is_some_and(|settings| settings.values == values) {
                return Ok(());
            }
            Some(Settings {
                format: "pcbcraft-rules/1".into(),
                id: match &project.board.rules {
                    Some(s) => s.id,
                    None => crate::fresh_id()?,
                },
                values,
            })
        };
        if project.board.rules == next {
            return Ok(());
        }
        let mut candidate = project.clone();
        Arc::make_mut(&mut candidate.board).rules = next;
        self.change(candidate)
    }
}

#[cfg(test)]
mod tests;
