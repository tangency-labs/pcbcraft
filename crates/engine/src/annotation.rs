//! Read-only reference designator proposals for existing schematic part instances.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use uuid::Uuid;

use crate::{CommandError, MAX_REFERENCE_NUMBER, PartInstance, Point, Project, Session, SheetTarget, schematic::SymbolPlacement};
use serde_json::{Value, json};

/// Preview schematic reference designators without publishing any edit.
pub const PREVIEW: &str = "annotate.preview";

mod apply;
pub use apply::APPLY;
pub(crate) use apply::ReviewedPreview;

#[cfg(test)]
pub(crate) mod tests;

/// One representative unit for the shared designator of a part instance.
struct Target<'a> {
    part: &'a PartInstance,
    unit: &'a SymbolPlacement,
    sheet: Uuid,
    occurrence: Uuid,
}

#[derive(Serialize)]
struct Row<'a> {
    #[serde(skip)]
    ordinal: u64,
    part_instance: String,
    symbol: String,
    sheet: Uuid,
    occurrence: Uuid,
    at: Point,
    locked: bool,
    current: &'a str,
    proposed: String,
}

#[derive(Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
enum Finding<'a> {
    DuplicateReference { reference: &'a str, part_instances: Vec<String> },
    NoAvailableReference { part_instance: String },
}

#[derive(Serialize)]
struct Preview<'a> {
    rows: Vec<Row<'a>>,
    errors: Vec<Finding<'a>>,
}

/// Collect one representative per in-scope part instance, in sheet order and then by increasing
/// Y and X of the symbol center. A stable unit ID breaks coincident-position ties.
fn targets(project: &Project, scope: Option<SheetTarget>) -> Vec<Target<'_>> {
    let occurrences: BTreeMap<_, _> =
        project.occurrence_instances().sheet_occurrences.into_iter().map(|occurrence| (occurrence.definition, occurrence.id)).collect();
    #[cfg(test)]
    tests::count_work(|work| work.occurrences += occurrences.len());
    let mut seen = BTreeSet::new();
    let mut targets = Vec::new();
    for sheet in project.sheets.iter().filter(|sheet| scope.is_none_or(|scope| scope.sheet == sheet.sheet_id)) {
        let mut units: Vec<_> = sheet.symbols.iter().collect();
        units.sort_unstable_by_key(|unit| {
            #[cfg(test)]
            tests::count_work(|work| work.sort_keys += 1);
            (unit.position.y_nm, unit.position.x_nm, unit.ordinal)
        });
        for unit in units {
            #[cfg(test)]
            tests::count_work(|work| work.units += 1);
            if seen.insert(unit.component)
                && let Some(part) = project.components.get(&unit.component)
            {
                targets.push(Target { part, unit, sheet: sheet.sheet_id, occurrence: occurrences[&sheet.sheet_id] });
            }
        }
    }
    targets
}

/// Find the first free canonical designator in the engine's allowed number range. Valid symbol
/// prefixes comprise uppercase letters only, so two distinct prefixes cannot generate the same reference.
fn first_free_reference(prefix: &str, start: usize, occupied: &BTreeMap<&str, Vec<&PartInstance>>) -> Option<String> {
    (start..=MAX_REFERENCE_NUMBER).map(|number| format!("{prefix}{number}")).find(|reference| {
        #[cfg(test)]
        tests::count_work(|work| work.probes += 1);
        !occupied.contains_key(reference.as_str())
    })
}

/// Preserve every existing designator except an unlocked exact `prefix?` placeholder. All parts
/// reserve their displayed strings, even outside the scope. A duplicate involving the scope is
/// reported with all claimants and is not repaired here.
fn plan<'a>(targets: &[Target<'a>], parts: impl Iterator<Item = &'a PartInstance>) -> Preview<'a> {
    let mut occupied: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for part in parts {
        #[cfg(test)]
        tests::count_work(|work| work.references += 1);
        occupied.entry(part.reference()).or_default().push(part);
    }
    let scoped: BTreeSet<_> = targets.iter().map(|target| target.part.ordinal).collect();
    let mut errors: Vec<_> = occupied
        .iter()
        .filter(|(_, parts)| parts.len() > 1 && parts.iter().any(|part| scoped.contains(&part.ordinal)))
        .map(|(&reference, parts)| Finding::DuplicateReference { reference, part_instances: parts.iter().map(|part| part.id()).collect() })
        .collect();
    let mut rows = Vec::with_capacity(targets.len());
    for target in targets {
        #[cfg(test)]
        tests::count_work(|work| work.rows += 1);
        let part = target.part;
        let current = part.reference();
        let prefix = part.definition.reference_prefix();
        let mut proposed = current.to_owned();
        if part.can_annotate_designator() && current.strip_prefix(prefix) == Some("?") && occupied.get(current).is_some_and(|parts| parts.len() == 1)
        {
            // A unique placeholder contributes at most one proposal per prefix.
            if let Some(reference) = first_free_reference(prefix, 1, &occupied) {
                proposed = reference;
            } else {
                errors.push(Finding::NoAvailableReference { part_instance: part.id() });
            }
        }
        rows.push(Row {
            ordinal: part.ordinal,
            part_instance: part.id(),
            symbol: target.unit.id(),
            sheet: target.sheet,
            occurrence: target.occurrence,
            at: target.unit.position,
            locked: part.designator_locked(),
            current,
            proposed,
        });
    }
    Preview { rows, errors }
}

/// Produce the same proposal from a resolved scope for both review and publication.
fn draft(project: &Project, scope: Option<SheetTarget>) -> Preview<'_> {
    plan(&targets(project, scope), project.components.values())
}

impl Session {
    /// Resolve the catalog-validated sheet scope and return proposals at the current revision.
    /// Omitted scope means the whole project, regardless of the active sheet. The returned
    /// revision is preview-only and grants no Apply authority. Only the separate session-local
    /// token authorizes Apply; replacing it does not touch the authored snapshot or history.
    pub(super) fn preview_annotation(&mut self, params: &Value) -> Result<Value, CommandError> {
        let project = self.snapshot().ok_or_else(|| CommandError::Unavailable("no project is open".into()))?;
        let scope = params.get("sheet").and_then(Value::as_str).map(|sheet| project.resolve_sheet(Some(sheet))).transpose()?;
        let revision = self.revision();
        let token = crate::fresh_id()?;
        let reply = json!({"revision":revision, "token":token, "annotation_preview":draft(&project, scope)});
        self.annotation_preview = Some(ReviewedPreview { token, revision, scope, project });
        Ok(reply)
    }
}
