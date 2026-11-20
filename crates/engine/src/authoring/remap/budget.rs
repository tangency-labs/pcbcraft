//! Transient review limits apply equally to headless, native and browser callers.
use crate::{Project, schematic::WireEndpoint};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io::{self, Write},
};

/// Maximum encoded report payload, excluding its transport wrapper. This is not a heap limit.
pub const MAX_PREVIEW_BYTES: usize = 4 * 1024 * 1024;
/// Allowance for modeled whole-project graph inputs and fanout, not a bound on total operations.
/// Ordered-map comparisons, byte comparisons and other implementation constants are not counted.
pub const MAX_PREVIEW_GRAPH_WORK: usize = 1_048_576;
/// Conservative path/name input bytes before cold graph preparation, not peak graph heap.
/// Display preparation can retain multiple path copies; this bounds admitted text input only.
pub const MAX_PREVIEW_GRAPH_TEXT_BYTES: usize = 64 * 1024 * 1024;

/// Count conservative graph inputs before cold preparation, including unrelated sheets. Cached
/// access uses the same admission rule; model caches cannot change whether a review is accepted.
pub(super) fn check_graph(project: &Project) -> Result<(), String> {
    let sheet_count = project.sheets().count();
    // Each path can walk at most S parents, scanning at most S sheets and S-1 child slots
    // per hop. Also charge rebuilding the bounded occurrence projection for each sheet.
    let mut work = add_work(0, sheet_count.checked_mul(sheet_count).and_then(|n| n.checked_mul(2 * sheet_count + 3)))?;
    // Number planning reads current and retired records; it does not walk their lineage.
    work = add_work(work, Some(project.net_registry.records.len()))?;
    let mut text = 0usize;
    let mut frequencies = BTreeMap::<String, usize>::new();
    let routed = project.requires_project_graph();
    // Any path visits at most all sheets; each source byte escapes to at most three.
    // The suffix allowance covers a collision digest and its punctuation without building paths.
    let path_bytes = project
        .sheets()
        .try_fold(68usize, |total, sheet| total.checked_add(sheet.name.len().checked_mul(3)?.checked_add(1)?))
        .ok_or("Pin remap graph text preparation budget exceeded.")?;
    for sheet in project.sheets() {
        let Some(project) = project.for_sheet(sheet.target) else { return Err("Unknown sheet during pin remap preparation.".into()) };
        let pin_counts = project.symbols().iter().map(|symbol| symbol.unit_definition().pins().len());
        let max_pins = pin_counts.clone().max().unwrap_or(0);
        let pins: usize = pin_counts.sum();
        let blocks = project.sheet_symbols().len();
        let ports: usize = project.sheet_symbols().iter().map(|block| project.sheet_pins(block).count()).sum();
        let labels = project.net_labels();
        let hierarchy = project.hierarchical_labels();
        let interfaces = labels.len() + hierarchy.len() + ports;
        let contacts = pins + interfaces;
        let anchors = contacts + project.junctions().count();
        let powers: Vec<_> = project.power_terminals().map(|(_, name, _)| name).collect();
        let markers = labels.len() + powers.len();
        // `wire_path` may add an elbow per authored interval; legacy diagonals use up to
        // three segments. Interface endpoint resolution scans the sheet's hierarchy endpoints.
        let mut route_work = Some(0usize);
        let mut segments = Some(0usize);
        if routed {
            for wire in project.wires() {
                let count = wire.bends().map_or(3, |bends| 2 * (bends.len() + 1));
                segments = segments.and_then(|total| total.checked_add(count));
                for end in wire.endpoints() {
                    let lookup = match end {
                        WireEndpoint::Terminal(_) => max_pins,
                        WireEndpoint::Junction(_) => 1,
                        _ => 2 * interfaces + 2 * blocks * sheet_count,
                    };
                    route_work = route_work.and_then(|total| total.checked_add(lookup));
                }
            }
        }
        // Direct marker/endpoint links exist even without wires. Each candidate contact can also
        // emit an edge traversed in both directions; the units here count inputs, not edge visits.
        let added = segments.and_then(|segments| {
            (pins + anchors + project.wires().len() + 2 * (ports + blocks * sheet_count))
                .checked_add(route_work?)?
                .checked_add(segments)?
                .checked_add(markers.checked_mul(contacts.checked_add(segments)?)?)
        });
        work = add_work(work, added)?;
        let mut name_bytes = 0usize;
        for name in labels.iter().map(|label| label.name.as_str()).chain(hierarchy.iter().map(|label| label.name.as_str())).chain(powers) {
            name_bytes += name.len();
            if let Some(count) = frequencies.get_mut(name) {
                *count += 1;
            } else {
                frequencies.insert(name.to_owned(), 1);
            }
        }
        let added_text = (anchors + 1).checked_mul(path_bytes).and_then(|bytes| bytes.checked_add(name_bytes));
        text = bounded(text, added_text, MAX_PREVIEW_GRAPH_TEXT_BYTES).ok_or("Pin remap graph text preparation budget exceeded.")?;
    }
    // A qualified collision group shares its escaped path and primary. For each authored primary
    // allow one generated primary (net numbers are unique) and one unqualified raw name equal to
    // the qualified string. Fallbacks do not participate. There are 29 digest-prefix lengths.
    for occurrences in frequencies.values() {
        let group = occurrences + 2;
        work = add_work(work, group.checked_mul(group - 1).and_then(|pairs| pairs.checked_mul(29)))?;
    }
    Ok(())
}

/// Admit the last input-work unit and refuse overflow or the first unit beyond the allowance.
fn add_work(current: usize, added: Option<usize>) -> Result<usize, String> {
    bounded(current, added, MAX_PREVIEW_GRAPH_WORK).ok_or_else(|| "Pin remap graph preparation budget exceeded.".into())
}

/// Check arithmetic before either counter is published; missing counts include upstream overflow.
fn bounded(current: usize, added: Option<usize>, limit: usize) -> Option<usize> {
    current.checked_add(added?).filter(|total| *total <= limit)
}

/// Charge exact encoded bytes of borrowed values before allocating their JSON representation.
/// Empty array/object delimiters are charged by the enclosing value; additions charge commas.
pub(super) struct Budget {
    remaining: usize,
}
impl Budget {
    /// Begin a report under the common payload limit, independently of any native reply limit.
    pub(super) fn new() -> Self {
        Self { remaining: MAX_PREVIEW_BYTES }
    }

    /// Refuse the first over-limit byte without retaining a partial row or truncating evidence.
    pub(super) fn charge(&mut self, value: &impl Serialize, comma: bool) -> Result<(), String> {
        let mut next = Self { remaining: self.remaining };
        let result =
            if comma { next.write_all(b",") } else { Ok(()) }.and_then(|_| serde_json::to_writer(&mut next, value).map_err(io::Error::other));
        result.map_err(|_| format!("Pin remap report exceeds {MAX_PREVIEW_BYTES} encoded bytes; review fewer logical pins at once."))?;
        self.remaining = next.remaining;
        Ok(())
    }
}
impl Write for Budget {
    /// Serde counts escaped UTF-8 JSON, not characters or an allocated intermediate encoding.
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.remaining = self.remaining.checked_sub(bytes.len()).ok_or_else(|| io::Error::other("pin remap report byte limit"))?;
        Ok(bytes.len())
    }
    /// The counter buffers no output, so flushing cannot publish or allocate a report.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Arithmetic admission is independent of cache state and refuses missing/overflowed counts.
    #[test]
    fn graph_work_last_unit_and_overflow_refuse_atomically() {
        assert_eq!(add_work(MAX_PREVIEW_GRAPH_WORK - 1, Some(1)).unwrap(), MAX_PREVIEW_GRAPH_WORK);
        for (current, added) in [(MAX_PREVIEW_GRAPH_WORK, Some(1)), (usize::MAX, Some(1)), (0, None)] {
            assert_eq!(add_work(current, added).unwrap_err(), "Pin remap graph preparation budget exceeded.");
        }
        assert_eq!(bounded(MAX_PREVIEW_GRAPH_TEXT_BYTES - 1, Some(1), MAX_PREVIEW_GRAPH_TEXT_BYTES), Some(MAX_PREVIEW_GRAPH_TEXT_BYTES));
        assert_eq!(bounded(MAX_PREVIEW_GRAPH_TEXT_BYTES, Some(1), MAX_PREVIEW_GRAPH_TEXT_BYTES), None);
    }

    /// Nested array delimiters and commas match the final report without charging them twice.
    #[test]
    fn incremental_report_charge_matches_final_encoding() {
        let mut report = serde_json::json!({"item":{},"instances":[]});
        let mut instance = serde_json::json!({"pins":[]});
        let row = serde_json::json!({"current_nets":["\"界\n"]});
        let mut budget = Budget::new();
        budget.charge(&report, false).unwrap();
        budget.charge(&instance, false).unwrap();
        budget.charge(&row, false).unwrap();
        budget.charge(&row, true).unwrap();
        instance["pins"] = serde_json::json!([row, row]);
        report["instances"] = serde_json::json!([instance]);
        assert_eq!(MAX_PREVIEW_BYTES - budget.remaining, serde_json::to_vec(&report).unwrap().len());
    }

    /// Match exact encoded JSON bytes, including quotes, comma and escaped multibyte strings.
    #[test]
    fn report_budget_accepts_last_byte_and_refuses_next() {
        let mut budget = Budget::new();
        budget.charge(&"x".repeat(MAX_PREVIEW_BYTES - 2), false).unwrap();
        assert_eq!(budget.remaining, 0);
        assert!(budget.charge(&0, false).unwrap_err().contains("encoded bytes"));
        let mut budget = Budget::new();
        let value = "\"界\n";
        let bytes = serde_json::to_vec(value).unwrap().len();
        budget.charge(&value, true).unwrap();
        assert_eq!(budget.remaining, MAX_PREVIEW_BYTES - bytes - 1);
        let before = budget.remaining;
        assert!(budget.charge(&"x".repeat(before), false).unwrap_err().contains("encoded bytes"));
        assert_eq!(budget.remaining, before);
    }
}
