//! Deterministic intersection of dimensional requirements and explicit preference conflicts.
use super::{MAX_CLASSES, Property, Rules};
use crate::{CommandError, Project, board::view::BoardTarget};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// The exact authored rule identity/revision or the released fallback behind a dimension.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    Builtin,
    Board { rule: Uuid },
    Request,
    Class { rule: Uuid, revision: u32 },
}

/// Both addressed objects retain their own requirements; pair spacing uses the stricter one.
pub(crate) fn pair_clearance(left: i64, right: i64) -> i64 {
    left.max(right)
}

/// All applicable values remain inspectable, including overridden preferences and weaker limits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Contribution {
    pub source: Source,
    pub value_nm: i64,
}

/// Read-only participation in one published dimensional result. This does not change policy,
/// the serialized contribution contract or whether cross-property conflicts make routing valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContributionStatus {
    /// Defines the resolved bound, or supplies a valid highest-precedence preference.
    Active,
    /// A class preference replaces this baseline, even when both numbers are identical.
    Overridden,
    /// Still applicable, but a stricter hard limit determines the effective bound.
    LessRestrictive,
    /// Participates in a preference that cannot currently resolve to a valid value.
    Conflicting,
}

/// A contradiction names the implicated dimensions and the exact contributing records.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Conflict {
    pub properties: Vec<Property>,
    pub message: String,
    pub contributions: BTreeMap<Property, Vec<Contribution>>,
}

/// Inspectable result, never a successful checking claim. Null preferences have no resolved winner;
/// contradictory hard bounds stay visible and must not be consumed while conflicts are present.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Resolution {
    pub classes: Vec<Uuid>,
    pub board: Option<BoardTarget>,
    pub values: BTreeMap<Property, Option<i64>>,
    pub contributions: BTreeMap<Property, Vec<Contribution>>,
    pub conflicts: Vec<Conflict>,
}

/// Symmetric pair spacing with independent per-object dimensional resolutions. Different route
/// preferences on different objects are valid; each object's internal conflicts remain explicit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PairResolution {
    pub board: Option<BoardTarget>,
    pub clearance_nm: i64,
    pub contributions: Vec<Contribution>,
    /// Canonically sorted by class set, rather than labeled left/right, to preserve symmetry.
    pub objects: [Resolution; 2],
    /// Deduplicated conflicts from the two independently resolved objects.
    pub conflicts: Vec<Conflict>,
}

/// Evaluate relationships only where authored operands exist, except supported geometry bounds.
/// Missing preferences remain unspecified; this never inserts board/built-in policy into a class.
pub(super) fn relationships(values: &Rules) -> Vec<(Vec<Property>, &'static str)> {
    use Property::*;
    let mut result = Vec::new();
    for (min, max) in [(MinViaDiameterNm, MaxViaDiameterNm), (MinViaDrillNm, MaxViaDrillNm)] {
        if let (Some(lo), Some(hi)) = (values.get(&min), values.get(&max))
            && lo > hi
        {
            result.push((vec![min, max], "Minimum exceeds maximum."));
        }
    }
    let min_drill = values.get(&MinViaDrillNm).copied().unwrap_or(MinViaDrillNm.range().0);
    let max_diameter = values.get(&MaxViaDiameterNm).copied().unwrap_or(MaxViaDiameterNm.range().1);
    let ring = values.get(&MinAnnularRingNm).copied().unwrap_or(0);
    let margin = (2 * ring).max(1);
    if min_drill + margin > max_diameter {
        result.push((vec![MinViaDrillNm, MinAnnularRingNm, MaxViaDiameterNm], "No via can satisfy the drill, annular ring and diameter bounds."));
    }
    for (preference, min, max) in [
        (PreferredTrackWidthNm, MinTrackWidthNm, None),
        (PreferredViaDiameterNm, MinViaDiameterNm, Some(MaxViaDiameterNm)),
        (PreferredViaDrillNm, MinViaDrillNm, Some(MaxViaDrillNm)),
    ] {
        if let Some(preferred) = values.get(&preference) {
            if values.get(&min).is_some_and(|bound| preferred < bound) {
                result.push((vec![preference, min], "Preference is below a hard minimum."));
            }
            if let Some(max) = max
                && values.get(&max).is_some_and(|bound| preferred > bound)
            {
                result.push((vec![preference, max], "Preference exceeds a hard maximum."));
            }
        }
    }
    let diameter = values.get(&PreferredViaDiameterNm).copied();
    let drill = values.get(&PreferredViaDrillNm).copied();
    if (diameter.is_some() || drill.is_some()) && diameter.unwrap_or(max_diameter) < drill.unwrap_or(min_drill) + margin {
        result.push((
            vec![PreferredViaDiameterNm, PreferredViaDrillNm, MinViaDrillNm, MaxViaDiameterNm, MinAnnularRingNm],
            "Preferred via dimensions cannot satisfy drill and annular ring requirements.",
        ));
    }
    result
}

impl Resolution {
    /// Explain original contributions without resolving again or altering any values. Hard
    /// ties jointly define their bound; explicit class preferences override the baseline even
    /// at equal numbers. Null preferences have conflicting candidates and no active winner.
    /// Independent conflicts between hard bounds remain in `conflicts`; Active alone is not
    /// a claim that the complete configuration is usable. Order and source IDs are retained.
    pub fn contribution_statuses(&self, property: Property) -> impl Iterator<Item = (&Contribution, ContributionStatus)> + '_ {
        let candidates = self.contributions.get(&property).map(Vec::as_slice).unwrap_or_default();
        let value = self.values.get(&property).copied().flatten();
        let class_preference = property.is_preference() && candidates.iter().any(|candidate| matches!(candidate.source, Source::Class { .. }));
        candidates.iter().map(move |candidate| {
            let status = if class_preference && !matches!(candidate.source, Source::Class { .. }) {
                ContributionStatus::Overridden
            } else if value.is_none() {
                ContributionStatus::Conflicting
            } else if value == Some(candidate.value_nm) {
                ContributionStatus::Active
            } else if property.is_preference() {
                ContributionStatus::Overridden
            } else {
                ContributionStatus::LessRestrictive
            };
            (candidate, status)
        })
    }

    /// Recover a single preferred operand before relationship validation, for a consumer that
    /// explicitly replaces its counterpart. Equal-precedence ambiguity still has no winner;
    /// callers must validate the completed pair against every hard limit.
    pub(crate) fn unambiguous_preference(&self, property: Property) -> Option<i64> {
        if !property.is_preference() {
            return None;
        }
        preferred_value(self.contributions.get(&property)?)
    }
    /// Attach full provenance to a diagnostic and mark any implicated preferences unresolved.
    fn conflict(&mut self, properties: Vec<Property>, message: &str) {
        for property in &properties {
            if property.is_preference() {
                self.values.insert(*property, None);
            }
        }
        let contributions = properties.iter().filter_map(|p| self.contributions.get(p).map(|c| (*p, c.clone()))).collect();
        self.conflicts.push(Conflict { properties, message: message.into(), contributions });
    }
}

/// Class preferences replace the baseline only when all equally specific contributors agree.
/// Relationship validation is separate, so an explicit counterpart can repair a preferred pair.
fn preferred_value(contributions: &[Contribution]) -> Option<i64> {
    let preferences: BTreeSet<_> =
        contributions.iter().filter(|candidate| matches!(candidate.source, Source::Class { .. })).map(|candidate| candidate.value_nm).collect();
    if preferences.len() > 1 { None } else { preferences.first().copied().or_else(|| contributions.first().map(|candidate| candidate.value_nm)) }
}

impl Project {
    /// Resolve explicit class IDs with built-ins or an explicitly addressed board's defaults.
    /// Classes tighten hard bounds; equally specific differing preferences are conflicts. Board
    /// defaults replace built-ins as in #242, but cannot weaken class limits. Unknown/foreign IDs
    /// are errors. No net lookup, stackup, revision change or derived geometry is needed.
    pub fn resolve_net_classes(&self, classes: &[Uuid], board: Option<BoardTarget>) -> Result<Resolution, CommandError> {
        self.resolve_net_classes_with_rules(classes, board, None)
    }

    /// Consumer-local diagnostic defaults retain per-property provenance. Applicable class
    /// limits still tighten that baseline, and preference conflicts remain explicit.
    pub(crate) fn resolve_net_classes_with_rules(
        &self,
        classes: &[Uuid],
        board: Option<BoardTarget>,
        baseline: Option<&crate::board::rules::Effective>,
    ) -> Result<Resolution, CommandError> {
        if classes.len() > MAX_CLASSES {
            return Err(CommandError::InvalidParams("Too many explicit class IDs.".into()));
        }
        let classes: BTreeSet<_> = classes.iter().copied().collect();
        let definitions = classes
            .iter()
            .map(|id| self.net_classes.get(*id).ok_or_else(|| CommandError::InvalidParams(format!("Unknown net class {id}."))))
            .collect::<Result<Vec<_>, _>>()?;
        let authored = match board {
            Some(target) => {
                self.board_view(target).ok_or_else(|| CommandError::InvalidParams("Unknown or foreign board target.".into()))?.authored_rules()
            }
            None => None,
        };
        let (defaults, source) = authored
            .map_or((crate::board::rules::Values::default(), Source::Builtin), |settings| (settings.values, Source::Board { rule: settings.id }));
        let defaults = baseline.map_or(defaults, |effective| effective.values);
        let mut result = Resolution {
            classes: classes.into_iter().collect(),
            board,
            values: BTreeMap::new(),
            contributions: BTreeMap::new(),
            conflicts: Vec::new(),
        };
        for property in Property::ALL {
            let base = property.value(&defaults);
            let base_source = baseline.and_then(|effective| effective.sources.get(property.as_str())).map_or_else(
                || source.clone(),
                |source| match source {
                    crate::board::rules::Source::Builtin => Source::Builtin,
                    crate::board::rules::Source::Board { rule } => Source::Board { rule: *rule },
                    crate::board::rules::Source::Request => Source::Request,
                },
            );
            let mut candidates = vec![Contribution { source: base_source, value_nm: base }];
            candidates.extend(definitions.iter().filter_map(|class| {
                class
                    .rules
                    .get(&property)
                    .map(|&value_nm| Contribution { source: Source::Class { rule: class.id, revision: class.revision }, value_nm })
            }));
            let effective = if property.is_preference() {
                preferred_value(&candidates)
            } else if property.is_maximum() {
                candidates.iter().map(|candidate| candidate.value_nm).min()
            } else {
                candidates.iter().map(|candidate| candidate.value_nm).max()
            };
            result.values.insert(property, effective);
            result.contributions.insert(property, candidates);
            if effective.is_none() {
                result.conflict(vec![property], "Classes supply different preferences at the same precedence.");
            }
        }
        let resolved: Rules = result.values.iter().filter_map(|(&property, &value)| value.map(|n| (property, n))).collect();
        for (properties, message) in relationships(&resolved) {
            result.conflict(properties, message);
        }
        Ok(result)
    }

    /// Resolve each object's dimensions independently, then take the larger clearance. Sources
    /// and object reports are canonicalized so swapping sides is identical, including conflicts.
    /// Different preferences across objects do not conflict or change either object's route sizes.
    pub fn resolve_class_clearance(&self, left: &[Uuid], right: &[Uuid], board: Option<BoardTarget>) -> Result<PairResolution, CommandError> {
        let mut objects = [self.resolve_net_classes(left, board)?, self.resolve_net_classes(right, board)?];
        objects.sort_by(|a, b| a.classes.cmp(&b.classes));
        let mut contributions = Vec::new();
        let mut conflicts = Vec::new();
        for object in &objects {
            for source in object.contributions.get(&Property::ClearanceNm).into_iter().flatten() {
                if !contributions.contains(source) {
                    contributions.push(source.clone());
                }
            }
            for conflict in &object.conflicts {
                if !conflicts.contains(conflict) {
                    conflicts.push(conflict.clone());
                }
            }
        }
        // Canonical source order is independent of which object supplies a class first.
        contributions.sort_by_key(|c| match c.source {
            Source::Builtin => (0, Uuid::nil()),
            Source::Board { rule } => (1, rule),
            Source::Request => (2, Uuid::nil()),
            Source::Class { rule, .. } => (3, rule),
        });
        let mut spacing = objects.iter().map(|object| object.values.get(&Property::ClearanceNm).copied().flatten());
        let (Some(Some(left)), Some(Some(right))) = (spacing.next(), spacing.next()) else {
            return Err(CommandError::InvalidParams("Missing clearance baseline.".into()));
        };
        let clearance_nm = pair_clearance(left, right);
        Ok(PairResolution { board, clearance_nm, contributions, objects, conflicts })
    }
}
