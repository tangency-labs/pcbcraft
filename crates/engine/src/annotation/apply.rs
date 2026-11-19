//! Preview authority and atomic designator publication.

use std::sync::Arc;

use super::*;

/// Apply exactly the most recently reviewed annotation plan in this session.
pub const APPLY: &str = "annotate.apply";

/// One session-local preview. Keeping the immutable snapshot lets stale results name the affected
/// parts, including when an intervening Undo removed the original state from ordinary history.
#[derive(Debug)]
pub(crate) struct ReviewedPreview {
    pub(super) token: Uuid,
    pub(super) revision: u64,
    pub(super) scope: Option<SheetTarget>,
    pub(super) project: Arc<Project>,
}

/// Identify authored designator, lock, and placement changes since the preview. Include every
/// placement of a shared part, not only its representative; outside-sheet references can also
/// change the proposals. Values, wiring and board-only edits need a fresh preview but do not
/// falsely identify a changed designator or placement.
fn changed_parts(before: &Project, after: &Project) -> Vec<String> {
    let units = |project: &Project| {
        let mut by_part: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for sheet in project.sheets.iter() {
            for unit in sheet.symbols.iter() {
                #[cfg(test)]
                super::tests::count_work(|work| work.stale_units += 1);
                by_part.entry(unit.component).or_default().push((unit.ordinal, sheet.sheet_id, unit.position));
            }
        }
        by_part
    };
    let old_units = units(before);
    let new_units = units(after);
    let ids = before.components.keys().chain(after.components.keys());
    #[cfg(test)]
    let ids = ids.inspect(|_| super::tests::count_work(|work| work.stale_part_keys += 1));
    ids.copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|id| {
            #[cfg(test)]
            super::tests::count_work(|work| work.stale_parts += 1);
            match (before.components.get(id), after.components.get(id)) {
                (Some(old), Some(new)) => {
                    old.reference() != new.reference() || old.designator_locked() != new.designator_locked() || old_units.get(id) != new_units.get(id)
                }
                _ => true,
            }
        })
        .map(|id| format!("component-{id}"))
        .collect()
}

/// Parse the number from a planner-generated designator, not from arbitrary user-supplied text.
fn proposed_number(part: &PartInstance, proposed: &str) -> Option<usize> {
    let number = proposed.strip_prefix(part.definition.reference_prefix())?.parse::<usize>().ok()?;
    (1..=MAX_REFERENCE_NUMBER).contains(&number).then_some(number)
}

/// Check final displayed uniqueness, not intermediate names; cycles and swaps are valid. Reserve
/// unchanged storage numbers first. A manually renamed part may occupy the desired storage
/// number without using its displayed string, so a rename uses another free number in that case.
fn with_designators(project: &Project, edits: &BTreeMap<u64, &str>) -> Result<Project, CommandError> {
    let mut shown = BTreeSet::new();
    let mut numbers: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
    let mut pending = Vec::new();
    for (&id, part) in project.components.iter() {
        #[cfg(test)]
        super::tests::count_work(|work| work.apply_parts += 1);
        let prefix = part.definition.reference_prefix();
        let proposed = edits.get(&id).copied().unwrap_or(part.reference());
        if !shown.insert(proposed) {
            return Err(CommandError::InvalidParams(format!("annotation would leave duplicate reference `{proposed}`")));
        }
        if proposed == part.reference() {
            numbers.entry(prefix).or_default().insert(part.reference_number);
        } else {
            if !part.can_annotate_designator() {
                return Err(CommandError::InvalidParams(format!("the designator for {} is locked", part.id())));
            }
            let number = proposed_number(part, proposed)
                .ok_or_else(|| CommandError::InvalidParams(format!("invalid proposed designator `{proposed}` for {}", part.id())))?;
            pending.push((id, part, proposed, number));
        }
    }

    let mut next = project.clone();
    #[cfg(test)]
    if Arc::strong_count(&next.components) > 1 {
        super::tests::count_work(|work| work.copied_parts += next.components.len());
    }
    let parts = Arc::make_mut(&mut next.components);
    let mut collisions = Vec::new();
    for (id, part, reference, number) in pending {
        if numbers.entry(part.definition.reference_prefix()).or_default().insert(number) {
            set_reference(parts, id, reference, number)?;
        } else {
            collisions.push((id, part, reference));
        }
    }
    for (id, part, reference) in collisions {
        let used = numbers.entry(part.definition.reference_prefix()).or_default();
        let number = if used.insert(part.reference_number) {
            part.reference_number
        } else {
            let free = (1..=MAX_REFERENCE_NUMBER)
                .find(|number| !used.contains(number))
                .ok_or_else(|| CommandError::CapExceeded("no unused internal reference number for annotation".into()))?;
            used.insert(free);
            free
        };
        set_reference(parts, id, reference, number)?;
    }
    Ok(next)
}

/// Change only the existing part's reference fields; even a multi-unit part keeps its identity.
fn set_reference(parts: &mut BTreeMap<u64, PartInstance>, id: u64, reference: &str, number: usize) -> Result<(), CommandError> {
    let part = parts.get_mut(&id).ok_or_else(|| CommandError::NotFound(format!("unknown part instance component-{id}")))?;
    #[cfg(test)]
    super::tests::count_work(|work| work.writes += 1);
    part.reference = reference.into();
    part.reference_number = number;
    Ok(())
}

impl Session {
    /// Accept the latest token only at its reviewed revision, replan with its recorded scope,
    /// and commit at most one edit. Stale or blocked requests never consume a current token.
    pub(crate) fn apply_annotation(&mut self, params: &Value) -> Result<(), CommandError> {
        let token = params["token"]
            .as_str()
            .and_then(|token| Uuid::parse_str(token).ok())
            .filter(|token| !token.is_nil())
            .ok_or_else(|| CommandError::InvalidParams("invalid annotation preview token".into()))?;
        let project = self.open_project()?;
        let reviewed = self
            .annotation_preview
            .as_ref()
            .filter(|reviewed| reviewed.token == token && reviewed.project.same_document(project))
            .ok_or_else(|| CommandError::Unavailable("unknown or replaced annotation preview; preview again".into()))?;
        if reviewed.revision != self.revision() {
            return Err(CommandError::StaleAnnotation { current: self.revision(), part_instances: changed_parts(&reviewed.project, project) });
        }
        let review = draft(project, reviewed.scope);
        if !review.errors.is_empty() {
            return Err(CommandError::Unavailable("resolve the annotation preview errors before applying".into()));
        }
        let edits: BTreeMap<_, _> =
            review.rows.iter().filter(|row| row.current != row.proposed).map(|row| (row.ordinal, row.proposed.as_str())).collect();
        if !edits.is_empty() {
            let next = with_designators(project, &edits)?;
            self.change(next)?;
        }
        self.annotation_preview = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
