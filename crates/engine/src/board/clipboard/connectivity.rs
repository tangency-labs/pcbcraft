//! Export rejects a flattened group when that representation would lose an internal connection.
use super::*;
use crate::schematic::Terminal;

/// Compare selected terminals' original electrical groups with the copied flat islands. Native
/// global labels/power markers can link islands across fragments; a missing hierarchy boundary
/// cannot. Snapshot-local indices are used only for this validation, never as persisted IDs.
pub(super) fn validate(project: &Project, circuits: &[Circuit], components: &BTreeSet<u64>) -> Result<(), CommandError> {
    type Island = (usize, Option<usize>, Option<Terminal>);
    if components.is_empty() {
        return Ok(());
    }
    let selected: BTreeSet<_> = project.all_symbols().filter(|s| components.contains(&s.component)).flat_map(|s| s.terminals()).collect();
    let mut islands = BTreeMap::<Terminal, Island>::new();
    let mut globals = BTreeMap::<(usize, usize), BTreeSet<String>>::new();
    for (index, circuit) in circuits.iter().enumerate() {
        let p = &circuit.fragment.project;
        let graph = p.electrical_connectivity();
        for terminal in p.symbols().iter().flat_map(|s| s.terminals()).filter(|t| selected.contains(t)) {
            let group = graph.net_index_for(terminal);
            islands.insert(terminal, (index, group, group.is_none().then_some(terminal)));
        }
        for (endpoint, name) in p
            .net_labels()
            .iter()
            .filter(|l| l.style.global())
            .map(|l| (WireEndpoint::NetLabel(l.id), l.name.as_str()))
            .chain(p.power_terminals().map(|(end, name, _)| (end, name)))
        {
            if let Some(group) = graph.net_index_for_endpoint(endpoint) {
                globals.entry((index, group)).or_default().insert(name.into());
            }
        }
    }
    for original in &project.electrical_connectivity().nets {
        let terminals: Vec<_> = original.iter().filter(|t| selected.contains(t)).collect();
        if terminals.len() < 2 {
            continue;
        }
        let buckets = terminals
            .into_iter()
            .map(|t| islands.get(t).copied().ok_or_else(|| invalid("copied circuit is missing a selected terminal")))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if buckets.len() < 2 {
            continue;
        }
        let buckets: Vec<_> = buckets.into_iter().collect();
        let mut names = BTreeMap::<&str, Vec<usize>>::new();
        for (index, (circuit, group, _)) in buckets.iter().enumerate() {
            if let Some(group) = group
                && let Some(global) = globals.get(&(*circuit, *group))
            {
                for name in global {
                    names.entry(name).or_default().push(index);
                }
            }
        }
        let mut visited = BTreeSet::new();
        let mut queue = vec![0];
        let mut consumed = BTreeSet::new();
        while let Some(index) = queue.pop() {
            if !visited.insert(index) {
                continue;
            }
            let (circuit, group, _) = buckets[index];
            if let Some(group) = group
                && let Some(global) = globals.get(&(circuit, group))
            {
                for name in global {
                    if consumed.insert(name.as_str()) {
                        queue.extend(names.get(name.as_str()).into_iter().flatten().copied());
                    }
                }
            }
        }
        if visited.len() != buckets.len() {
            return Err(invalid(
                "PCB copy would lose an internal connection. Include the connecting components or copy the complete schematic subtree first.",
            ));
        }
    }
    Ok(())
}
