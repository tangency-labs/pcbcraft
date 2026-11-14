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
    Class { rule: Uuid, revision: u32 },
}

/// All applicable values remain inspectable, including overridden preferences and weaker limits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Contribution {
    pub source: Source,
    pub value_nm: i64,
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

impl Project {
    /// Resolve explicit class IDs with built-ins or an explicitly addressed board's defaults.
    /// Classes tighten hard bounds; equally specific differing preferences are conflicts. Board
    /// defaults replace built-ins as in #242, but cannot weaken class limits. Unknown/foreign IDs
    /// are errors. No net lookup, stackup, revision change or derived geometry is needed.
    pub fn resolve_net_classes(&self, classes: &[Uuid], board: Option<BoardTarget>) -> Result<Resolution, CommandError> {
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
        let mut result = Resolution {
            classes: classes.into_iter().collect(),
            board,
            values: BTreeMap::new(),
            contributions: BTreeMap::new(),
            conflicts: Vec::new(),
        };
        for property in Property::ALL {
            let base = property.value(&defaults);
            let mut candidates = vec![Contribution { source: source.clone(), value_nm: base }];
            candidates.extend(definitions.iter().filter_map(|class| {
                class
                    .rules
                    .get(&property)
                    .map(|&value_nm| Contribution { source: Source::Class { rule: class.id, revision: class.revision }, value_nm })
            }));
            let effective = if property.is_preference() {
                let preferences: BTreeSet<_> = candidates.iter().skip(1).map(|candidate| candidate.value_nm).collect();
                if preferences.len() > 1 { None } else { Some(preferences.first().copied().unwrap_or(base)) }
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
            Source::Class { rule, .. } => (2, rule),
        });
        let clearance_nm =
            contributions.iter().map(|c| c.value_nm).max().ok_or_else(|| CommandError::InvalidParams("Missing clearance baseline.".into()))?;
        Ok(PairResolution { board, clearance_nm, contributions, objects, conflicts })
    }
}
