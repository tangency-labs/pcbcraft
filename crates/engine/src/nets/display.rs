//! Read-only net presentation. Neither generated text nor collision suffixes are electrical authority.
use super::{Anchor, AnchorKind};
use crate::{
    Project,
    schematic::{Connectivity, WireEndpoint},
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Encode a presentation token byte-for-byte, keeping the separator punctuation unambiguous.
pub fn token(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut result = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+') {
            result.push(char::from(byte));
        } else {
            result.push('%');
            result.push(char::from(HEX[usize::from(byte >> 4)]));
            result.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    result
}

/// Select a unique trimmed logical pin name across the component's complete multi-unit table.
/// Ambiguous, empty or unavailable metadata falls back to the electrical terminal designator.
pub(crate) fn terminal_name(reference: &str, terminal: &str, pins: &[crate::schematic::PinDefinition]) -> String {
    let name = pins.iter().find(|pin| pin.number == terminal).and_then(|pin| pin.name.as_deref()).map(str::trim).filter(|name| !name.is_empty());
    let name = name.filter(|name| pins.iter().filter(|pin| pin.name.as_deref().map(str::trim) == Some(*name)).count() == 1).unwrap_or(terminal);
    format!("{}.{}", token(reference), token(name))
}

/// Full current occurrence path. The supported hierarchy has one occurrence per sheet.
fn path(project: &Project, mut sheet: uuid::Uuid) -> String {
    let mut segments = Vec::new();
    while let Some(own) = project.sheets.iter().find(|candidate| candidate.sheet_id == sheet) {
        segments.push(token(&own.sheet_name));
        let Some(parent) = project.sheets.iter().find(|candidate| candidate.subsheets.iter().any(|block| block.child == sheet)) else { break };
        sheet = parent.sheet_id;
    }
    segments.reverse();
    segments.join("/")
}

/// Derive all primary names together so qualification is project-wide, before sheet projection.
/// Canonical anchors select endpoints; geometry, references and iteration order never do.
pub(crate) fn assign(project: &Project, graph: &mut Connectivity) {
    let mut endpoints = BTreeMap::new();
    let connected_symbols: BTreeSet<_> = graph.nets.iter().flatten().map(|terminal| terminal.symbol).collect();
    let mut named = BTreeMap::new();
    let scope = project.net_scope();
    for sheet in project.sheets() {
        let Some(own) = project.for_sheet(sheet.target) else { continue };
        let Some(occurrence) = project.sheet_occurrence(sheet.target) else { continue };
        let sheet_path = path(project, sheet.target.id());
        named.extend(own.net_labels().iter().map(|label| (WireEndpoint::NetLabel(label.id), label.name.clone())));
        named.extend(own.hierarchical_labels().iter().map(|label| (WireEndpoint::Label(label.id), label.name.clone())));
        named.extend(own.power_terminals().map(|(end, name, _)| (end, name.to_owned())));
        for symbol in own.symbols().iter().filter(|symbol| connected_symbols.contains(&symbol.ordinal)) {
            if !project.components.contains_key(&symbol.component) {
                continue;
            }
            for terminal in symbol.terminals() {
                if graph.net_index_for(terminal).is_none() {
                    continue;
                }
                let anchor = Anchor {
                    kind: AnchorKind::Terminal,
                    project: project.id,
                    scope,
                    occurrence: uuid::Uuid::nil(),
                    object: symbol.component,
                    terminal: terminal.number().into(),
                };
                endpoints.insert(WireEndpoint::Terminal(terminal), (anchor.clone(), sheet_path.clone()));
            }
        }
        for (end, _) in own.hierarchy_endpoints().chain(own.junctions().map(|(id, at)| (WireEndpoint::Junction(id), at))) {
            let (kind, object) = match end {
                WireEndpoint::NetLabel(id) => (AnchorKind::Label, id),
                WireEndpoint::Label(id) => (AnchorKind::Port, id),
                WireEndpoint::SheetPin(_) => continue,
                WireEndpoint::Junction(id) => (AnchorKind::Junction, id),
                WireEndpoint::Terminal(_) => continue,
            };
            endpoints.entry(end).or_insert_with(|| {
                (Anchor { kind, project: project.id, scope, occurrence: occurrence.id, object, terminal: String::new() }, sheet_path.clone())
            });
        }
    }
    // A parent sheet pin is another drawing of its child's port, not a second scoped anchor.
    for sheet in project.sheets.iter() {
        for label in sheet.labels.iter() {
            if let Some(port) = endpoints.get(&WireEndpoint::Label(label.id)).cloned() {
                endpoints.insert(WireEndpoint::SheetPin(label.id), port);
            }
        }
    }
    let mut addresses = Vec::new();
    let mut paths = Vec::new();
    let mut primaries = Vec::new();
    let mut fallback = Vec::new();
    for index in 0..graph.nets.len() {
        let anchors =
            graph.endpoints(index).filter_map(|end| endpoints.get(&end)).fold(BTreeMap::<Anchor, String>::new(), |mut anchors, (anchor, path)| {
                anchors
                    .entry(anchor.clone())
                    .and_modify(|current| {
                        if path < current {
                            *current = path.clone();
                        }
                    })
                    .or_insert_with(|| path.clone());
                anchors
            });
        let terminal = anchors.keys().find(|anchor| anchor.kind == AnchorKind::Terminal && project.components.contains_key(&anchor.object));
        fallback.push(graph.net_names(index).is_empty() && terminal.is_none());
        let name = graph
            .net_names(index)
            .iter()
            .min()
            .cloned()
            .or_else(|| {
                terminal.and_then(|anchor| {
                    let component = project.components.get(&anchor.object)?;
                    Some(terminal_name(component.reference(), &anchor.terminal, component.definition.pins()))
                })
            })
            .unwrap_or_else(|| "Unconnected".into());
        // Explicit local names retain the occurrence that declares the chosen name, even when
        // hierarchy brings a lower component terminal onto the same resolved network.
        let named_anchor = graph.net_names(index).first().and_then(|name| {
            graph.endpoints(index).filter(|end| named.get(end) == Some(name)).filter_map(|end| endpoints.get(&end).map(|(anchor, _)| anchor)).min()
        });
        let selected = named_anchor.or(terminal).or_else(|| anchors.keys().next());
        paths.push(selected.and_then(|key| anchors.get(key)).cloned().unwrap_or_default());
        // A derived scoped address is its canonical supported anchor; it is never saved as a net ID.
        let address = anchors.keys().next().and_then(|a| a.canonical_key().ok()).unwrap_or_default();
        addresses.push(format!("{:x}", Sha256::digest(address)));
        primaries.push(name);
    }
    graph.display_names = qualify(primaries, paths, addresses, &fallback);
}

/// Qualify every colliding primary, then disambiguate equal paths with bounded digest prefixes.
fn qualify(primary: Vec<String>, paths: Vec<String>, digests: Vec<String>, fallback: &[bool]) -> Vec<String> {
    let counts = primary.iter().enumerate().filter(|(i, _)| !fallback[*i]).fold(BTreeMap::<&str, usize>::new(), |mut counts, (_, name)| {
        *counts.entry(name).or_default() += 1;
        counts
    });
    let names: Vec<_> = primary
        .iter()
        .zip(paths)
        .enumerate()
        .map(|(index, (name, path))| if !fallback[index] && counts[name.as_str()] > 1 { format!("{path}::{name}") } else { name.clone() })
        .collect();
    let mut groups = BTreeMap::<&str, Vec<usize>>::new();
    for (index, name) in names.iter().enumerate() {
        if !fallback[index] {
            groups.entry(name).or_default().push(index);
        }
    }
    let mut result = names.clone();
    let mut used = BTreeSet::new();
    for (name, indices) in &groups {
        if indices.len() < 2 {
            continue;
        }
        let mut ordered = indices.clone();
        ordered.sort_by(|a, b| digests[*a].cmp(&digests[*b]));
        for index in ordered {
            let mut length = 8;
            let candidate = loop {
                let candidate = format!("{name}~{}", &digests[index][..length]);
                let shared_prefix = indices.iter().any(|other| *other != index && digests[*other][..length] == digests[index][..length]);
                if length == 64 || !shared_prefix && !groups.contains_key(candidate.as_str()) && !used.contains(&candidate) {
                    break candidate;
                }
                length += 2;
            };
            used.insert(candidate.clone());
            result[index] = candidate;
        }
    }
    result
}

#[cfg(test)]
mod tests;
