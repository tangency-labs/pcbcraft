//! Bounded read-only projection of authored relations, shared by DRC and relation queries.

use super::{
    Kind, MAX_RELATIONS, Point, Reference, State, Witness,
    catalog::{self, Axis, Measure},
};
use crate::Project;
use serde::Serialize;
use serde_json::Number;
use uuid::Uuid;

/// Separate relation results, never counted as native DRC violations or used as edit authority.
/// Serialized as the existing ordered array of reference, suppressed and lost rows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct RelationAdvisory(Vec<Row>);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Row {
    id: Uuid,
    kind: Kind,
    refs: [Reference; 2],
    datum: u8,
    label: Option<Point>,
    axes_set: [Axis; 0],
    suppressed: bool,
    #[serde(flatten)]
    status: Status,
}

type ReportMeasure = Measure<Option<Number>>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Status {
    Reference {
        measure: ReportMeasure,
    },
    Suppressed {
        measure: ReportMeasure,
    },
    Lost {
        #[serde(flatten)]
        loss: Loss,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
enum Loss {
    LostReference { since_revision: u64, last_resolved: [Witness; 2] },
    UnresolvedReference,
}

impl RelationAdvisory {
    /// Omit the section without authored rows. Work is limited by the native relation/field
    /// bounds, and saved witnesses go only into the latched display case, never the resolver.
    pub(in crate::board) fn capture(project: &Project) -> Option<Self> {
        let relations = &project.board.relations.as_ref()?.relations;
        if relations.is_empty() {
            return None;
        }
        let resolver = catalog::Resolver::new(project);
        Some(Self(
            relations
                .iter()
                .take(MAX_RELATIONS)
                .map(|row| {
                    let status = match row.state {
                        State::Lost { since_revision } => {
                            Status::Lost { loss: Loss::LostReference { since_revision, last_resolved: row.last_resolved } }
                        }
                        State::Ok => match catalog::live_measure(&resolver, row) {
                            Some(measure) if row.suppressed => Status::Suppressed { measure: measure.for_report() },
                            Some(measure) => Status::Reference { measure: measure.for_report() },
                            None => Status::Lost { loss: Loss::UnresolvedReference },
                        },
                    };
                    Row {
                        id: row.id,
                        kind: row.kind,
                        refs: row.refs.clone(),
                        datum: row.datum,
                        label: row.label,
                        axes_set: [],
                        suppressed: row.suppressed,
                        status,
                    }
                })
                .collect(),
        ))
    }
}
