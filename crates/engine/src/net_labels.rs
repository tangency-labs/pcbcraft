//! Named electrical anchors and shared revision-checked editing commands.
use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Place a local or global text label.
pub const ADD: &str = "schematic.place_net_label";
/// Change a named anchor's properties, orientation or position.
pub const SET: &str = "schematic.set_net_label";
/// Remove a named anchor and wires directly attached to its identity.
pub const DELETE: &str = "schematic.delete_net_label";
/// Per-sheet limit independent of component and wire counts.
pub const MAX_LABELS: usize = 256;

/// Text labels are either sheet-scoped or global; power symbols use native definitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    Local,
    Global,
}
impl Style {
    /// Whether matching names connect outside this sheet.
    pub fn global(self) -> bool {
        self != Self::Local
    }
}

/// A named electrical anchor, without a component reference, footprint or hidden supply pin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Label {
    pub id: u64,
    pub name: String,
    pub style: Style,
    pub at: Point,
    pub rotation: u16,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "text_layout")]
    pub text: Option<schematic::fields::Layout>,
}
/// Omission preserves legacy text; explicit null cannot erase an authored appearance.
fn text_layout<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<schematic::fields::Layout>, D::Error> {
    schematic::fields::Layout::deserialize(d).map(Some)
}
impl Label {
    /// Stable command identity independent of the displayed name.
    pub fn object_id(&self) -> String {
        format!("net-label-{}", self.id)
    }
    /// Editable properties and the endpoint accepted by draw-wire commands.
    pub fn inspect(&self) -> Value {
        let mut value = json!({"id":self.object_id(),"name":self.name,"style":self.style,"at":self.at,"rotation":self.rotation,"endpoint":WireEndpoint::NetLabel(self.id).inspect()});
        if let Some(text) = &self.text {
            value["text"] = json!(text);
        }
        value
    }
}
impl Project {
    /// Whether names, hierarchy or multi-unit parts require project-wide electrical resolution.
    pub(crate) fn requires_project_graph(&self) -> bool {
        self.sheets.iter().any(|sheet| {
            sheet.symbols.iter().any(|symbol| symbol.definition.units.len() > 1)
                || !sheet.net_labels.is_empty()
                || !sheet.labels.is_empty()
                || sheet.symbols.iter().any(|s| s.definition.power_terminal().is_some())
        })
    }
    /// Explicit definition roles name ordinary terminals from the owning component's Value.
    /// Library IDs and drawings never participate in electrical interpretation.
    pub fn power_terminals(&self) -> impl Iterator<Item = (WireEndpoint, &str, Point)> {
        self.symbols().iter().filter_map(|symbol| {
            let number = symbol.definition.power_terminal()?;
            let index = symbol.unit_definition().pin_index(number)?;
            let component = self.component(symbol)?;
            Some((WireEndpoint::Terminal(symbol.terminals()[index]), component.value(), symbol.pins()[index]))
        })
    }

    /// Named anchors on the addressed schematic sheet.
    pub fn net_labels(&self) -> &[Label] {
        &self.net_labels
    }
    /// Preview an existing marker's geometry without changing its identity or history. Attached
    /// wire endpoints use the preview position; unknown IDs or invalid geometry are rejected.
    pub fn preview_net_label(&self, id: u64, at: Point, rotation: u16) -> Result<Self, CommandError> {
        let mut next = self.clone();
        let index = self
            .net_labels
            .iter()
            .position(|label| label.id == id)
            .ok_or_else(|| CommandError::InvalidParams("unknown net label on this sheet".into()))?;
        let label = &mut Arc::make_mut(&mut next.sheet_mut().net_labels)[index];
        label.at = at;
        label.rotation = rotation;
        next.validate_net_labels().map_err(CommandError::InvalidParams)?;
        next.sheet_mut().connectivity = Arc::default();
        next.named_graph = Arc::default();
        Ok(next)
    }

    /// Validate records before adoption; codecs also check IDs against every other object type.
    pub(crate) fn validate_net_labels(&self) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        for sheet in self.sheets.iter() {
            if sheet.net_labels.len() > MAX_LABELS {
                return Err("sheet exceeds the named-label limit".into());
            }
            for label in sheet.net_labels.iter() {
                persistence::validate_id(&mut ids, label.id)?;
                if label.name.is_empty()
                    || label.name.trim() != label.name
                    || label.name.chars().count() > MAX_NAME
                    || label.name.chars().any(char::is_control)
                {
                    return Err("net name must contain 1–200 characters without surrounding whitespace or control characters".into());
                }
                if let Some(text) = &label.text {
                    schematic::fields::validate_layouts(&BTreeMap::from([("value".into(), text.clone())]), &schematic::fields::Metadata::default())?;
                }
                if !wire::valid_point(&label.at) || !matches!(label.rotation, 0 | 90 | 180 | 270) {
                    return Err("invalid named-label position or quarter-turn rotation".into());
                }
            }
        }
        Ok(())
    }
    /// Resolve names and hierarchy project-wide before projecting back to a sheet. Global
    /// names take display priority, while all aliases remain visible in inspection.
    pub(crate) fn named_connectivity(&self, whole_project: bool) -> Connectivity {
        let mut graph = self.named_graph.get_or_init(|| self.build_named_graph()).clone();
        if !whole_project {
            let own: BTreeSet<_> = self
                .symbols()
                .iter()
                .flat_map(|s| s.terminals().into_iter().map(WireEndpoint::Terminal))
                .chain(self.hierarchy_endpoints().map(|(end, _)| end))
                .chain(self.junctions().map(|(id, _)| WireEndpoint::Junction(id)))
                .collect();
            graph.retain_endpoints(&own);
        }
        graph
    }

    /// Build the full graph once per immutable snapshot. Cache pin positions and routes once
    /// per sheet rather than resolving each wire's geometry again for every label.
    fn build_named_graph(&self) -> Connectivity {
        let symbols: Vec<_> = self.all_symbols().cloned().collect();
        let wires: Vec<_> = self.sheets.iter().flat_map(|s| s.wires.iter().cloned()).collect();
        let mut links = Vec::new();
        let mut names = BTreeMap::new();
        let mut globals = BTreeMap::new();
        for sheet in self.sheets() {
            let Some(project) = self.for_sheet(sheet.target) else {
                continue;
            };
            let mut contacts: BTreeMap<(i64, i64), Vec<WireEndpoint>> = BTreeMap::new();
            for (end, at) in project
                .symbols()
                .iter()
                .flat_map(|s| s.terminals().into_iter().map(WireEndpoint::Terminal).zip(s.pins()))
                .chain(project.hierarchy_endpoints())
            {
                contacts.entry((at.x_nm, at.y_nm)).or_default().push(end);
            }
            let routes: Vec<_> = project.wires().iter().map(|wire| (wire.endpoints()[0], project.wire_path(wire))).collect();
            let mut local = BTreeMap::new();
            for label in project.hierarchical_labels() {
                let end = WireEndpoint::Label(label.id);
                names.insert(end, (1, label.name.clone()));
                local.insert(label.name.clone(), end);
            }
            for block in project.sheet_symbols() {
                links.extend(project.sheet_pins(block).map(|pin| [WireEndpoint::SheetPin(pin.label.id), WireEndpoint::Label(pin.label.id)]));
            }
            let markers = project
                .net_labels()
                .iter()
                .map(|label| (WireEndpoint::NetLabel(label.id), label.name.as_str(), label.at, label.style.global()))
                .chain(project.power_terminals().map(|(end, name, at)| (end, name, at, true)));
            for (end, name, at, global) in markers {
                links.push([end, end]);
                names.insert(end, (u8::from(!global), name.to_owned()));
                if let Some(previous) = local.insert(name.to_owned(), end) {
                    links.push([previous, end]);
                }
                if global && let Some(previous) = globals.insert(name.to_owned(), end) {
                    links.push([previous, end]);
                }
                // A label is an explicit contact marker. Ordinary coincident endpoints and
                // wire crossings have no electrical effect away from these authored anchors.
                if let Some(ends) = contacts.get(&(at.x_nm, at.y_nm)) {
                    links.extend(ends.iter().copied().filter(|other| *other != end).map(|other| [end, other]));
                }
                for (target, path) in &routes {
                    if wire::path_contains_point(path, at) {
                        links.push([end, *target]);
                    }
                }
            }
        }
        let mut graph = Connectivity::build_linked(&symbols, &wires, links);
        graph.assign_names(&names);

        graph
    }
}
impl Session {
    /// Validate an isolated candidate before publishing one undoable edit. Unknown/foreign
    /// IDs and stale revisions leave state intact; unchanged properties do not advance history.
    /// With keep_connected, moving captures the old marker's wire/pin contacts before updating
    /// metadata, committing the connected geometry and properties in one undo step.
    pub(crate) fn edit_net_label(&mut self, command: &str, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let mut project = self.open_project()?.clone();
        let index = if command == ADD {
            None
        } else {
            let id = params["id"].as_str().and_then(|s| schematic::wiring::parse_id(s, "net-label-"));
            Some(
                project
                    .net_labels
                    .iter()
                    .position(|l| Some(l.id) == id)
                    .ok_or_else(|| CommandError::InvalidParams("unknown net label on this sheet".into()))?,
            )
        };
        if command == DELETE {
            if let Some(index) = index {
                let id = Arc::make_mut(&mut project.sheet_mut().net_labels).remove(index).id;
                Arc::make_mut(&mut project.sheet_mut().wires).retain(|w| !w.endpoints.contains(&WireEndpoint::NetLabel(id)));
                project.prune_junctions();
            }
        } else {
            if index.is_none() && project.net_labels.len() >= MAX_LABELS {
                return Err(CommandError::Unavailable("sheet already has 256 named labels".into()));
            }
            let id = match index {
                Some(index) => project.net_labels[index].id,
                None => self.next_object_id()?,
            };
            let label = Label {
                id,
                name: params["name"].as_str().unwrap_or_default().into(),
                style: serde_json::from_value(params["style"].clone()).map_err(|_| CommandError::InvalidParams("unknown label style".into()))?,
                at: Point { x_nm: params["x_nm"].as_i64().unwrap_or_default(), y_nm: params["y_nm"].as_i64().unwrap_or_default() },
                rotation: params["rotation"].as_u64().unwrap_or(0) as u16,
                text: params
                    .get("text")
                    .map(|v| serde_json::from_value(v.clone()))
                    .transpose()
                    .map_err(|e| CommandError::InvalidParams(format!("invalid text appearance: {e}")))?
                    .or_else(|| index.and_then(|i| project.net_labels[i].text.clone())),
            };
            match index {
                Some(index) if project.net_labels[index] == label => return Ok(()),
                Some(index) => {
                    if params["keep_connected"].as_bool().unwrap_or(false) && project.net_labels[index].at != label.at {
                        project = project.dragged_net_label(label.id, label.at, label.rotation, self.revision(), wire::auto_heal(params))?;
                    }
                    Arc::make_mut(&mut project.sheet_mut().net_labels)[index] = label;
                }
                None => Arc::make_mut(&mut project.sheet_mut().net_labels).push(label),
            }
            project.validate_net_labels().map_err(CommandError::InvalidParams)?;
        }
        project.sheet_mut().connectivity = Arc::default();
        self.change(project);
        Ok(())
    }
}
#[cfg(test)]
mod tests;
