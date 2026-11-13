//! Intentional wire terminations retain endpoint identity without changing the electrical graph.
use super::*;
use crate::schematic::WireEndpoint;

/// A project-unique free junction belonging to a wire, independent of coordinates or net names.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireTarget {
    pub junction: String,
}

impl WireTarget {
    /// Resolve the canonical endpoint identity accepted by commands and reports.
    pub(crate) fn ordinal(&self) -> Option<u64> {
        crate::schematic::wiring::parse_id(&self.junction, "junction-")
    }
}

/// Marker targets keep the existing part/terminal vocabulary and add a distinct wire-end form.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Attachment {
    Pin(Target),
    WireEnd(WireTarget),
}

impl From<Target> for Attachment {
    fn from(target: Target) -> Self {
        Self::Pin(target)
    }
}

impl Attachment {
    /// Parse either exact target shape, rejecting unknown fields and malformed identities.
    pub fn parse(value: &Value) -> Option<Self> {
        if let Some(target) = Target::parse(value) {
            return Some(Self::Pin(target));
        }
        let target: WireTarget = serde_json::from_value(value.clone()).ok()?;
        target.ordinal()?;
        Some(Self::WireEnd(target))
    }
}

/// A navigable wire endpoint on an explicitly addressed sheet.
#[derive(Clone, Debug, Serialize)]
pub struct WireAppearance {
    pub sheet: SheetTarget,
    pub wire: String,
    pub at: Point,
}

/// Saved termination intent and any conflict with the current endpoint topology.
#[derive(Clone, Debug, Serialize)]
pub struct WireMarker {
    pub target: WireTarget,
    pub appearance: WireAppearance,
    pub issues: Vec<&'static str>,
}

/// A dangling wire warning or a previously marked end that is no longer a free end.
#[derive(Clone, Debug, Serialize)]
pub struct WireFinding {
    pub key: String,
    pub rule: &'static str,
    pub severity: &'static str,
    pub target: WireTarget,
    pub appearance: WireAppearance,
}

/// Snapshot-derived validation shared by editing and cursor target caches. Keeping only
/// marked coordinates avoids rescanning pins for each merge or pointer movement.
#[derive(Clone, Debug, Default)]
pub struct WireAttachments {
    blocked: BTreeSet<(i64, i64)>,
}

impl WireAttachments {
    /// New attachments at intentional terminations require removing the mark first.
    /// This checks coordinates as well as identity aliases and never changes the snapshot.
    pub fn check(&self, at: Point) -> Result<(), CommandError> {
        if self.blocked.contains(&(at.x_nm, at.y_nm)) { Err(CommandError::InvalidParams(WIRE_BLOCKED.into())) } else { Ok(()) }
    }
}

impl Project {
    /// Count all authored pin and wire-end markers against one project-wide budget.
    pub(crate) fn no_connect_count(&self) -> usize {
        self.components.values().map(|part| part.no_connects.len()).sum::<usize>()
            + self.sheets.iter().map(|sheet| sheet.no_connect_ends.len()).sum::<usize>()
    }

    /// Count explicit incident edges; geometric crossings alone never create an endpoint.
    pub fn wire_end_degree(&self, id: u64) -> usize {
        self.wires().iter().filter(|wire| wire.endpoints().contains(&WireEndpoint::Junction(id))).count()
    }

    /// Build endpoint degree once for a sheet scan rather than rescanning every wire per end.
    fn wire_end_degrees(&self) -> BTreeMap<u64, usize> {
        let mut degree = BTreeMap::new();
        for end in self.wires().iter().flat_map(|wire| wire.endpoints()) {
            if let WireEndpoint::Junction(id) = end {
                *degree.entry(id).or_insert(0usize) += 1;
            }
        }
        degree
    }

    /// Exact named/interface contacts are distinct from a name elsewhere on the same net.
    fn wire_named_contacts(&self) -> BTreeSet<(i64, i64)> {
        self.net_labels()
            .iter()
            .map(|label| label.at)
            .chain(self.power_terminals().map(|(_, _, at)| at))
            .chain(self.hierarchy_endpoints().map(|(_, at)| at))
            .map(|at| (at.x_nm, at.y_nm))
            .collect()
    }

    /// Pickable physical wire ends on this sheet, independent of the net's naming or membership.
    pub fn wire_end_targets(&self) -> impl Iterator<Item = (WireTarget, Point)> + '_ {
        let degree = self.wire_end_degrees();
        self.junctions
            .iter()
            .filter(move |(id, _)| degree.get(id) == Some(&1))
            .map(|(&id, &at)| (WireTarget { junction: format!("junction-{id}") }, at))
    }

    /// Authored markers on this sheet, including conflicts after an end acquires another edge.
    pub fn wire_end_marks(&self) -> impl Iterator<Item = (WireTarget, Point, bool)> + '_ {
        let degree = if self.no_connect_ends.is_empty() { BTreeMap::new() } else { self.wire_end_degrees() };
        self.no_connect_ends.iter().filter_map(move |&id| {
            let at = *self.junctions.get(&id)?;
            Some((WireTarget { junction: format!("junction-{id}") }, at, degree.get(&id) != Some(&1)))
        })
    }

    /// Inspect either marker kind without changing coordinates or electrical connectivity.
    pub fn has_no_connect_attachment(&self, target: &Attachment) -> bool {
        match target {
            Attachment::Pin(target) => {
                target.ordinal().and_then(|id| self.components.get(&id)).is_some_and(|part| part.no_connects.contains_key(&target.terminal))
            }
            Attachment::WireEnd(target) => target.ordinal().is_some_and(|id| self.sheets.iter().any(|sheet| sheet.no_connect_ends.contains(&id))),
        }
    }

    /// Capture this sheet's no-connect attachment rule once for an edit or view target cache.
    /// The immutable policy uses exact model positions, including aliases of an endpoint.
    pub fn wire_attachment_policy(&self) -> WireAttachments {
        WireAttachments { blocked: self.no_connect_appearances().map(|(_, at)| (at.x_nm, at.y_nm)).collect() }
    }

    /// Every visible authored mark on the projected sheet, resolved from its current owner.
    /// Repeated drawings share a target identity but may supply more than one position.
    pub fn no_connect_appearances(&self) -> impl Iterator<Item = (Attachment, Point)> + '_ {
        let pins =
            self.symbols().iter().filter(|symbol| self.component(symbol).is_some_and(|part| !part.no_connects.is_empty())).flat_map(move |symbol| {
                symbol
                    .terminals()
                    .into_iter()
                    .zip(symbol.pins())
                    .filter(move |(pin, _)| self.has_no_connect(*pin))
                    .map(move |(pin, at)| (Attachment::Pin(Target { part: symbol.component_id(), terminal: pin.number().into() }), at))
            });
        let ends = self
            .no_connect_ends
            .iter()
            .filter_map(|&id| Some((Attachment::WireEnd(WireTarget { junction: format!("junction-{id}") }), *self.junctions.get(&id)?)));
        pins.chain(ends)
    }

    /// Clear validated sheet-local intent in a disposable candidate before group publication.
    /// Its owner and actual electrical connectivity are preserved.
    pub(crate) fn remove_no_connect_attachment(&mut self, target: &Attachment) {
        match target {
            Attachment::Pin(pin) => {
                if let Some(id) = pin.ordinal()
                    && let Some(part) = Arc::make_mut(&mut self.components).get_mut(&id)
                {
                    part.no_connects.remove(&pin.terminal);
                }
            }
            Attachment::WireEnd(end) => {
                if let Some(id) = end.ordinal() {
                    Arc::make_mut(&mut self.sheet_mut().no_connect_ends).remove(&id);
                }
            }
        }
    }

    /// Report each unmarked free end separately. Marking an end suppresses only that warning;
    /// actual pin connectivity and other ERC/DRC rules remain authoritative.
    pub(super) fn check_wire_no_connects(&self, report: &mut Report) {
        for sheet in self.sheets() {
            let Some(view) = self.for_sheet(sheet.target) else { continue };
            let degree = view.wire_end_degrees();
            let named = view.wire_named_contacts();
            let mut owners = BTreeMap::new();
            for wire in view.wires() {
                for end in wire.endpoints() {
                    if let WireEndpoint::Junction(id) = end {
                        owners.entry(id).or_insert_with(|| wire.id());
                    }
                }
            }
            for (&id, &at) in view.junctions.iter() {
                let marked = view.no_connect_ends.contains(&id);
                let free = degree.get(&id) == Some(&1);
                let dangling = free && !named.contains(&(at.x_nm, at.y_nm));
                if !marked && !dangling {
                    continue;
                }
                let Some(wire) = owners.get(&id) else { continue };
                let target = WireTarget { junction: format!("junction-{id}") };
                let appearance = WireAppearance { sheet: sheet.target, wire: wire.clone(), at };
                let rule = if marked { (!free).then_some("no_connect_not_dangling") } else { Some("dangling_wire") };
                if marked {
                    report.total_markers += 1;
                    if report.markers.len() + report.wire_markers.len() < MAX_FINDINGS {
                        report.wire_markers.push(WireMarker {
                            target: target.clone(),
                            appearance: appearance.clone(),
                            issues: rule.into_iter().collect(),
                        });
                    }
                }
                if let Some(rule) = rule {
                    report.total_findings += 1;
                    if report.findings.len() + report.wire_findings.len() < MAX_FINDINGS {
                        report.wire_findings.push(WireFinding {
                            key: format!("{rule}/{}", target.junction),
                            rule,
                            severity: if marked { "error" } else { "warning" },
                            target,
                            appearance,
                        });
                    }
                }
            }
        }
    }
}

impl Session {
    /// Mark a live dangling endpoint or remove its existing intent atomically. New wire
    /// attachments are blocked while marked; retained saved conflicts remain inspectable.
    pub(super) fn set_wire_no_connect(&mut self, target: &WireTarget, params: &Value) -> Result<(), CommandError> {
        let id = target.ordinal().ok_or_else(|| CommandError::InvalidParams("Invalid wire endpoint.".into()))?;
        let marked = params["marked"].as_bool().ok_or_else(|| CommandError::InvalidParams("marked must be a boolean".into()))?;
        let original = self.open_project()?;
        let index = original
            .sheets
            .iter()
            .position(|sheet| sheet.junctions.contains_key(&id))
            .ok_or_else(|| CommandError::InvalidParams("Unknown wire endpoint.".into()))?;
        let present = original.sheets[index].no_connect_ends.contains(&id);
        if present == marked {
            return Ok(());
        }
        let mut next = original.clone();
        next.sheet_index = index;
        if marked && next.wire_end_degree(id) != 1 {
            return Err(CommandError::InvalidParams("A wire no-connect mark requires a dangling wire end.".into()));
        }
        if marked && next.no_connect_count() >= MAX_MARKERS {
            return Err(CommandError::Unavailable("No-connect marker limit reached.".into()));
        }
        if marked {
            Arc::make_mut(&mut next.sheet_mut().no_connect_ends).insert(id);
        } else {
            Arc::make_mut(&mut next.sheet_mut().no_connect_ends).remove(&id);
        }
        next.sheet_index = original.sheet_index;
        self.change(next)
    }
}

#[cfg(test)]
mod tests;
