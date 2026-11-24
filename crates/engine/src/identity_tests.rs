//! Each object kind of the identity scheme carries a persistent UUID or an owner token, or is a
//! listed gap, and each record a command-built project saves is classified.
//!
//! The audit reads the production documents of a command-built project rather than a list of
//! types, so a newly persisted record without a classification fails here. The record and kind
//! tables below are the inventory described in `docs/formats/object-identity.md`.
use super::*;
use crate::{
    board::{setup::LayerPurpose, track::tests as tracks},
    nets::{Anchor, AnchorKind, ClassTarget},
    persistence::{Documents, decode_documents, encode_documents},
};
use std::collections::{BTreeMap, BTreeSet};

/// How a saved record names itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Form {
    /// A canonical UUID in its `id` field.
    Uuid,
    /// A positive project counter, saved as a number or as `prefix-N` text.
    Counter,
    /// Authored text such as a library item key.
    Name,
    /// No `id` field; the record is content of its owner.
    Owned,
    /// A plain value saved as a list element or under an id key, such as a wire end in a list.
    Value,
}

/// The object kinds that the identity scheme addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Component,
    SymbolUnit,
    Wire,
    Junction,
    NetLabel,
    PowerPort,
    Sheet,
    SheetInstance,
    Port,
    Net,
    NetClass,
    PartGroup,
    FootprintInstance,
    TrackSegment,
    Arc,
    Via,
    Zone,
    Pour,
    Keepout,
    BoardDrawing,
    Text,
    Dimension,
    Rule,
    Layer,
    Board,
    LibraryItem,
    Pin,
    Pad,
}

impl Kind {
    const ALL: [Kind; 28] = [
        Kind::Component,
        Kind::SymbolUnit,
        Kind::Wire,
        Kind::Junction,
        Kind::NetLabel,
        Kind::PowerPort,
        Kind::Sheet,
        Kind::SheetInstance,
        Kind::Port,
        Kind::Net,
        Kind::NetClass,
        Kind::PartGroup,
        Kind::FootprintInstance,
        Kind::TrackSegment,
        Kind::Arc,
        Kind::Via,
        Kind::Zone,
        Kind::Pour,
        Kind::Keepout,
        Kind::BoardDrawing,
        Kind::Text,
        Kind::Dimension,
        Kind::Rule,
        Kind::Layer,
        Kind::Board,
        Kind::LibraryItem,
        Kind::Pin,
        Kind::Pad,
    ];

    /// Where this kind's identity lives in the current engine. The match is exhaustive so a new
    /// kind cannot be added without deciding its address.
    fn address(self) -> Address {
        use Address::{Absent, Records, Token};
        match self {
            Kind::Component => Records(&["project.json:components[]"]),
            Kind::SymbolUnit => Records(&["sheets/*:symbols[]"]),
            // Power ports are placements of native power definitions.
            Kind::PowerPort => Records(&["sheets/*:symbols[]"]),
            Kind::Wire => Records(&["sheets/*:wires[]"]),
            Kind::Junction => Records(&["sheets/*:junctions[]"]),
            Kind::NetLabel => Records(&["sheets/*:net_labels[]"]),
            Kind::Sheet => Records(&["project.json:sheets[]", "sheets/*:"]),
            Kind::SheetInstance => Records(&["sheets/*:subsheets[]", "instances.json:sheet_occurrences[]"]),
            Kind::Port => Records(&["sheets/*:labels[]"]),
            Kind::Net => Records(&["instances.json:net_registry.records[]"]),
            Kind::NetClass => Records(&["project.json:net_classes.definitions[]"]),
            Kind::PartGroup => Records(&["project.json:part_groups.groups[]"]),
            Kind::FootprintInstance => Records(&["board.json:footprints[]"]),
            Kind::TrackSegment => Records(&["board.json:tracks[]"]),
            Kind::Via => Records(&["board.json:vias[]"]),
            Kind::Zone => Records(&["board.json:zones[]"]),
            Kind::BoardDrawing => Records(&["board.json:graphics.items[]"]),
            Kind::Rule => Records(&["board.json:rules"]),
            Kind::Layer => Records(&["board.json:setup.layers[]"]),
            Kind::Board => Records(&["project.json:boards[]"]),
            Kind::LibraryItem => Records(&["library/symbols/*:", "library/footprints/*:"]),
            Kind::Pin => Token(Kind::SymbolUnit),
            Kind::Pad => Token(Kind::FootprintInstance),
            Kind::Arc | Kind::Pour | Kind::Keepout | Kind::Text | Kind::Dimension => Absent,
        }
    }
}

/// The saved location of a kind's identity.
#[derive(Clone, Copy, Debug)]
enum Address {
    /// Saved records; the kind has a persistent UUID only when every one of them does.
    Records(&'static [&'static str]),
    /// An owner record plus a name, such as a pin number. Stable only while the owner is.
    Token(Kind),
    /// The engine has no saved record of this kind yet; its first record must be classified.
    Absent,
}

/// Every record path the fixture saves, the identity forms it is expected to show, and what it is.
/// Owned and value paths are listed too, so content cannot hide a new kind; see `Seen::walk`. A
/// list that is empty in the fixture is declared with no forms, so the first element a fixture
/// saves there has to be classified.
const RECORDS: &[(&str, &[Form], &str)] = &[
    ("project.json:", &[Form::Uuid], "project"),
    ("project.json:boards[]", &[Form::Uuid], "board registration"),
    ("project.json:sheets[]", &[Form::Uuid], "sheet reference"),
    ("project.json:components[]", &[Form::Counter], "component"),
    ("project.json:components[].metadata", &[Form::Owned], "component metadata"),
    ("project.json:components[].metadata.attributes", &[Form::Owned], "component attributes"),
    ("project.json:components[].metadata.fields", &[Form::Owned], "component fields"),
    ("project.json:net_classes", &[Form::Owned], "net class catalog"),
    ("project.json:net_classes.definitions[]", &[Form::Uuid], "net class"),
    ("project.json:net_classes.definitions[].rules", &[Form::Owned], "net class rule values"),
    ("project.json:part_groups", &[Form::Owned], "part group catalog"),
    ("project.json:part_groups.groups[]", &[Form::Uuid], "part group"),
    ("project.json:parameters", &[Form::Owned], "project parameters"),
    ("sheets/*:", &[Form::Uuid], "sheet document"),
    ("sheets/*:page", &[Form::Owned], "sheet page"),
    ("sheets/*:page.at_nm[]", &[Form::Value], "page origin coordinate"),
    ("sheets/*:page.template", &[Form::Owned], "sheet page template"),
    ("sheets/*:parameters", &[Form::Owned], "sheet parameters"),
    ("sheets/*:symbols[]", &[Form::Counter], "placed symbol unit"),
    ("sheets/*:symbols[].field_layout", &[Form::Owned], "symbol field layout"),
    ("sheets/*:symbols[].field_layout.reference", &[Form::Owned], "reference field placement"),
    ("sheets/*:symbols[].field_layout.value", &[Form::Owned], "value field placement"),
    ("sheets/*:symbols[].field_layout.value.offset", &[Form::Owned], "value field offset"),
    ("sheets/*:wires[]", &[Form::Counter], "wire"),
    ("sheets/*:wires[].from", &[Form::Owned], "wire start"),
    ("sheets/*:wires[].to", &[Form::Owned], "wire end"),
    ("sheets/*:wires[].bends[]", &[Form::Owned], "wire bend point"),
    ("sheets/*:no_connect_ends[]", &[Form::Value], "no-connect intent on a wire end"),
    ("sheets/*:junctions[]", &[Form::Counter], "junction"),
    ("sheets/*:net_labels[]", &[Form::Counter], "net label"),
    ("sheets/*:net_labels[].at", &[Form::Owned], "net label position"),
    ("sheets/*:labels[]", &[Form::Counter], "hierarchical label (port)"),
    ("sheets/*:labels[].at", &[Form::Owned], "hierarchical label position"),
    ("sheets/*:subsheets[]", &[Form::Counter], "sheet symbol"),
    ("sheets/*:subsheets[].at", &[Form::Owned], "sheet symbol position"),
    ("board.json:", &[Form::Owned], "board document; the board's id is its registration"),
    ("board.json:setup", &[Form::Owned], "board setup"),
    ("board.json:setup.layers[]", &[Form::Uuid], "layer"),
    ("board.json:setup.stackup", &[Form::Uuid], "stackup"),
    ("board.json:setup.stackup.strata[]", &[Form::Uuid, Form::Owned], "stratum; copper strata are addressed by their layer"),
    ("board.json:rules", &[Form::Uuid], "board rule set"),
    ("board.json:rules.values", &[Form::Owned], "board rule values"),
    ("board.json:baseline", &[Form::Owned], "accepted schematic baseline"),
    ("board.json:baseline.source", &[Form::Owned], "baseline source revision"),
    ("board.json:baseline.components", &[Form::Owned], "accepted components, keyed by component"),
    ("board.json:baseline.components.{id}", &[Form::Owned], "accepted component"),
    ("board.json:baseline.components.{id}.terminals[]", &[Form::Value], "accepted component terminal"),
    ("board.json:baseline.components.{id}.package", &[Form::Owned], "accepted package choice"),
    ("board.json:baseline.components.{id}.package.mapping", &[Form::Owned], "accepted pad mapping"),
    ("board.json:baseline.components.{id}.package.mapping.{id}", &[Form::Value], "accepted pad mapping entry"),
    ("board.json:baseline.nets[]", &[Form::Owned], "accepted connection"),
    ("board.json:baseline.nets[].names[]", &[Form::Value], "accepted connection name"),
    ("board.json:baseline.nets[].terminals[]", &[Form::Owned], "accepted connection terminal"),
    ("board.json:assignments[]", &[Form::Owned], "package choice, keyed by component"),
    ("board.json:assignments[].mapping", &[Form::Owned], "package pad mapping"),
    ("board.json:assignments[].mapping.{id}", &[Form::Value], "package pad mapping entry"),
    ("board.json:graphics", &[Form::Owned], "board drawing collection"),
    ("board.json:graphics.layers[]", &[], "board drawing layer list, empty in the fixture"),
    ("board.json:graphics.items[]", &[Form::Uuid], "board drawing"),
    ("board.json:graphics.items[].points[]", &[Form::Owned], "board drawing vertex"),
    ("board.json:tracks[]", &[Form::Uuid], "track segment"),
    ("board.json:tracks[].start", &[Form::Owned], "track start point"),
    ("board.json:tracks[].end", &[Form::Owned], "track end point"),
    ("board.json:vias[]", &[Form::Uuid], "via"),
    ("board.json:vias[].at", &[Form::Owned], "via position"),
    ("board.json:zones[]", &[Form::Uuid], "zone outline"),
    ("board.json:zones[].points[]", &[Form::Owned], "zone outline vertex"),
    ("board.json:zones[].options", &[Form::Owned], "zone fill intent"),
    ("board.json:footprints[]", &[Form::Counter], "footprint instance"),
    ("board.json:footprints[].mapping", &[Form::Owned], "footprint pad mapping"),
    ("board.json:footprints[].mapping.{id}", &[Form::Value], "footprint pad mapping entry"),
    ("instances.json:", &[Form::Owned], "instance document of the registered board"),
    ("instances.json:circuit_uses[]", &[Form::Uuid], "circuit use"),
    ("instances.json:sheet_occurrences[]", &[Form::Uuid], "sheet occurrence"),
    ("instances.json:part_group_memberships[]", &[Form::Owned], "membership addressed by part instance and group"),
    ("instances.json:compact_part_group_memberships", &[Form::Owned], "compressed membership envelope"),
    ("instances.json:compact_part_group_memberships.groups[]", &[Form::Value], "group UUID dictionary reference"),
    ("instances.json:compact_part_group_memberships.part_instances[]", &[Form::Owned], "membership set addressed by part instance"),
    ("instances.json:compact_part_group_memberships.part_instances[].groups[]", &[Form::Value], "local dictionary reference, not an object identity"),
    ("instances.json:net_registry", &[Form::Owned], "net registry"),
    ("instances.json:net_registry.records[]", &[Form::Uuid], "net"),
    ("instances.json:net_registry.records[].state", &[Form::Owned], "net state"),
    ("instances.json:net_registry.records[].primary", &[Form::Owned], "net primary anchor"),
    ("instances.json:net_registry.records[].primary.scope", &[Form::Uuid], "scope reference in a net anchor"),
    ("instances.json:net_registry.records[].fallbacks[]", &[Form::Owned], "net fallback anchor"),
    ("instances.json:net_registry.records[].fallbacks[].scope", &[Form::Uuid], "scope reference in a net anchor"),
    ("instances.json:net_registry.records[].class_bindings[]", &[Form::Owned], "net class binding"),
    ("instances.json:net_registry.records[].lineage[]", &[], "net lineage, empty in the fixture"),
    ("library/symbols/*:", &[Form::Name], "symbol definition"),
    ("library/footprints/*:", &[Form::Name], "footprint definition"),
];

/// Kinds without a persistent UUID today. Shrink this list as each kind moves to UUIDs.
const EXPECTED_GAPS: &[Kind] = &[
    Kind::Component,
    Kind::SymbolUnit,
    Kind::Wire,
    Kind::Junction,
    Kind::NetLabel,
    Kind::PowerPort,
    Kind::SheetInstance,
    Kind::Port,
    Kind::FootprintInstance,
    Kind::LibraryItem,
    Kind::Pin,
    Kind::Pad,
];

/// Run one revision-checked catalog command, naming it if the fixture is refused.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    if let Err(error) = session.apply(command, &params) {
        panic!("{command}: {error}");
    }
}

/// A routed board plus a track, via, zone, drawing, rule set, net class and binding, power port, bent
/// wire, no-connect, net label, child sheet with a port, parameters and a page setting, built only
/// through shared commands.
fn fixture() -> Session {
    let mut s = tracks::routed_fixture();
    let project = s.project().unwrap().clone();
    let board_id = project.board_target().unwrap().board;
    let root = project.sheets().next().unwrap().target;
    let copper = tracks::copper_layers(&s);
    let silkscreen = tracks::view(&s).setup().unwrap().layers().iter().find(|layer| layer.purpose == LayerPurpose::Silkscreen).unwrap().id;
    let point = |x_nm: i64, y_nm: i64| json!({"x_nm":x_nm,"y_nm":y_nm});

    apply(
        &mut s,
        board::track::ADD_TRACKS,
        json!({"layer":copper[0],"width_nm":250_000,"points":[point(0, 50_000_000), point(10_000_000, 50_000_000)]}),
    );
    apply(&mut s, board::track::ADD_VIA, json!({"at":point(0, 40_000_000),"via_diameter_nm":600_000,"via_drill_nm":300_000}));
    apply(
        &mut s,
        board::drawing::ADD_DRAWING,
        json!({"layer":silkscreen,"points":[point(0, 60_000_000), point(10_000_000, 60_000_000)],"stroke_nm":150_000,"closed":false}),
    );
    let rules = board::rules::Values { clearance_nm: 400_000, ..board::rules::Values::default() };
    apply(&mut s, board::rules::SET_RULES, json!({"board":board_id,"rules":rules}));

    apply(&mut s, net_classes::CREATE, json!({"name":"Power","rules":{"clearance_nm":500_000}}));
    let project = s.project().unwrap().clone();
    let class = project.net_classes().definitions[0].id;
    let component = *project.components.keys().next().unwrap();
    let anchor = Anchor {
        kind: AnchorKind::Terminal,
        project: project.id,
        scope: project.net_scope(),
        occurrence: uuid::Uuid::nil(),
        object: component,
        terminal: "1".into(),
    };
    apply(&mut s, nets::SET_CLASSES, json!({"target":ClassTarget::Derived{anchor},"classes":[class]}));
    apply(&mut s, part_groups::CREATE, json!({"name":"Analog parts"}));
    let group = s.project().unwrap().part_groups().groups[0].id;
    apply(&mut s, part_groups::SET_GROUPS, json!({"part_instance":part_groups::part_instance_address(component),"groups":[group]}));

    let net = s.project().unwrap().net_registry.records.iter().find(|r| matches!(r.state, nets::State::Active {})).unwrap().id;
    apply(
        &mut s,
        board::zones::ADD_ZONE,
        json!({"board":board_id,"net":net,"layer":copper[0],"points":[point(0,70_000_000),point(10_000_000,70_000_000),point(0,80_000_000)]}),
    );

    let gnd = s.libraries.symbol("power", "gnd", 1).unwrap();
    let digest = library::sources::symbol_digest(&gnd).unwrap();
    apply(
        &mut s,
        PLACE_SYMBOL,
        json!({"library":"power","definition":"gnd","definition_revision":1,"definition_digest":digest,"x_nm":0,"y_nm":25_400_000}),
    );
    let project = s.project().unwrap().clone();
    let pin = project.symbols().last().unwrap().terminals()[0];
    let at = project.terminal_position(pin).unwrap();
    let (x_nm, y_nm) = (at.x_nm - 25_400_000, at.y_nm);
    apply(&mut s, DRAW_WIRE, json!({"from":pin.inspect(),"to":point(x_nm, y_nm + 25_400_000),"bends":[point(x_nm, y_nm)]}));
    let end = s.project().unwrap().wire_end_targets().next().unwrap().0;
    apply(&mut s, no_connect::SET, json!({"target":end,"marked":true}));
    apply(&mut s, net_labels::ADD, json!({"name":"SENSE","style":"global","x_nm":127_000_000,"y_nm":127_000_000}));

    apply(&mut s, hierarchy::CREATE_CHILD, json!({"sheet":root.id(),"name":"Child","x_nm":76_200_000,"y_nm":0}));
    let child = s.project().unwrap().sheets().find(|sheet| sheet.name == "Child").unwrap().target;
    apply(&mut s, hierarchy::ADD_LABEL, json!({"sheet":child.id(),"name":"EN","direction":"input","x_nm":0,"y_nm":0}));
    apply(&mut s, parameters::SET, json!({"scope":"project","name":"Revision","value":"B"}));
    apply(&mut s, parameters::SET, json!({"scope":"sheet","sheet":root.id(),"name":"Title","value":"Audit"}));
    apply(&mut s, sheets::page::SET_PAGE, json!({"sheet":root.id(),"paper":"a3","orientation":"landscape"}));
    s
}

/// Fold a stored document path to its role: one name for every sheet, board folder and library item.
fn document_role(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    if path.starts_with("library/symbols/") {
        "library/symbols/*".into()
    } else if path.starts_with("library/footprints/") {
        "library/footprints/*".into()
    } else if path.starts_with("sheets/") {
        "sheets/*".into()
    } else {
        file.into()
    }
}

/// Classify a record by its own `id` field.
fn form(record: &Map<String, Value>) -> Form {
    match record.get("id") {
        None => Form::Owned,
        Some(Value::Number(_)) => Form::Counter,
        Some(Value::String(text)) if uuid::Uuid::parse_str(text).is_ok_and(|id| id.hyphenated().to_string() == *text) => Form::Uuid,
        Some(Value::String(text))
            if text.rsplit_once('-').is_some_and(|(prefix, _)| schematic::wiring::parse_id(text, &format!("{prefix}-")).is_some()) =>
        {
            Form::Counter
        }
        Some(_) => Form::Name,
    }
}

/// Saved records by path: the forms seen, and the `id` values for owner-token checks.
#[derive(Default)]
struct Seen {
    forms: BTreeMap<String, BTreeSet<Form>>,
    ids: BTreeMap<String, BTreeSet<String>>,
}

impl Seen {
    /// Record one stored document. Library definitions are versioned as a whole, so only their
    /// root is an addressed record and their content is not walked.
    fn document(&mut self, path: &str, bytes: &[u8]) {
        let value: Value = serde_json::from_slice(bytes).unwrap();
        let role = document_role(path);
        let library = role.starts_with("library/");
        self.walk(&format!("{role}:"), &value, false, !library);
    }

    /// Visit one JSON value and register it under its path when it could hold an object of some
    /// kind: every JSON object, wherever it is nested, and every value that is an element of a
    /// list or sits under an id key. An empty list registers its element path with no forms, so a
    /// list that the fixture leaves empty is still declared. Only named scalar fields are left out,
    /// because a field holds one value of its owner rather than a collection of objects. Map keys
    /// that are ids fold to `{id}` so paths name roles, not objects.
    fn walk(&mut self, path: &str, value: &Value, element: bool, descend: bool) {
        match value {
            Value::Object(object) => {
                self.forms.entry(path.into()).or_default().insert(form(object));
                if let Some(id) = object.get("id") {
                    let id = id.as_str().map_or_else(|| id.to_string(), str::to_owned);
                    self.ids.entry(path.into()).or_default().insert(id);
                }
                if descend {
                    for (key, child) in object {
                        let keyed = uuid::Uuid::parse_str(key).is_ok() || key.bytes().all(|b| b.is_ascii_digit());
                        let key = if keyed { "{id}" } else { key };
                        let separator = if path.ends_with(':') { "" } else { "." };
                        self.walk(&format!("{path}{separator}{key}"), child, keyed, true);
                    }
                }
            }
            Value::Array(items) => {
                if element {
                    self.forms.entry(path.into()).or_default().insert(Form::Value);
                }
                if items.is_empty() {
                    self.forms.entry(format!("{path}[]")).or_default();
                }
                for item in items {
                    self.walk(&format!("{path}[]"), item, true, true);
                }
            }
            _ if element => {
                self.forms.entry(path.into()).or_default().insert(Form::Value);
            }
            _ => {}
        }
    }
}

/// Kinds whose identity is not a persistent UUID: records with any other form, and tokens whose
/// owner is itself such a kind.
fn gaps(forms: &BTreeMap<&str, &[Form]>) -> BTreeSet<Kind> {
    let mut gaps = BTreeSet::new();
    for kind in Kind::ALL {
        if let Address::Records(paths) = kind.address()
            && paths.iter().any(|path| forms.get(path).is_none_or(|forms| *forms != [Form::Uuid]))
        {
            gaps.insert(kind);
        }
    }
    for kind in Kind::ALL {
        if let Address::Token(owner) = kind.address()
            && gaps.contains(&owner)
        {
            gaps.insert(kind);
        }
    }
    gaps
}

/// The inventory problems in a set of saved documents, followed by the registered paths and their
/// forms for the failure message. An empty problem list means every registered path is declared
/// with the forms it shows, every declared path is saved, every kind has a declared address and
/// the kinds without a persistent UUID are exactly the listed gaps.
fn inventory_problems(documents: &Documents) -> (Vec<String>, String) {
    let mut seen = Seen::default();
    for (path, bytes) in documents {
        seen.document(path, bytes);
    }

    let declared: BTreeMap<&str, &[Form]> = RECORDS.iter().map(|(path, forms, _)| (*path, *forms)).collect();
    let mut problems = Vec::new();
    for (path, forms) in &seen.forms {
        match declared.get(path.as_str()) {
            None => problems.push(format!("unclassified record {path}: {forms:?}")),
            Some(expected) if !expected.iter().copied().eq(forms.iter().copied()) => {
                problems.push(format!("record {path} shows {forms:?}, declared {expected:?}"))
            }
            Some(_) => {}
        }
    }
    for path in declared.keys().filter(|path| !seen.forms.contains_key(**path)) {
        let compact = path.starts_with("instances.json:compact_part_group_memberships");
        let alternate = if compact { "instances.json:part_group_memberships[]" } else { "instances.json:compact_part_group_memberships" };
        if (compact || *path == "instances.json:part_group_memberships[]") && seen.forms.contains_key(alternate) {
            continue;
        }
        problems.push(format!("fixture saves no {path} record"));
    }
    for kind in Kind::ALL {
        if let Address::Records(paths) = kind.address() {
            for path in paths.iter().filter(|path| !declared.contains_key(*path)) {
                problems.push(format!("{kind:?} names undeclared record {path}"));
            }
        }
    }
    let found = gaps(&declared);
    let expected: BTreeSet<Kind> = EXPECTED_GAPS.iter().copied().collect();
    for kind in found.difference(&expected) {
        problems.push(format!("{kind:?} has no persistent UUID and is not a listed gap"));
    }
    for kind in expected.difference(&found) {
        problems.push(format!("{kind:?} now has a persistent UUID; remove it from the gap list"));
    }
    let saved = seen.forms.iter().map(|(path, forms)| format!("  {path}: {forms:?}")).collect::<Vec<_>>().join("\n");
    (problems, saved)
}

/// Every record the fixture saves is classified, every kind has an address, and the kinds without
/// a persistent UUID are exactly the listed gaps.
#[test]
fn every_saved_object_kind_has_a_stable_identity_or_is_a_listed_gap() {
    let project = fixture().snapshot().unwrap();
    // Vary only storage preference, as reopening the compact envelope does; authored fixture
    // contents still come exclusively from the shared commands above.
    for compact in [false, true] {
        let project = Arc::new(Project { prefer_compact_part_groups: compact, ..Project::clone(&project) });
        let documents = encode_documents(project).unwrap();
        let (problems, saved) = inventory_problems(&documents);
        assert!(problems.is_empty(), "{}\nsaved records:\n{saved}", problems.join("\n"));
    }
}

/// A new kind is reported whatever shape it is saved in: a nested object without an id, a list of
/// objects, a list of plain values, an empty list, plain values under id keys, or a new document.
#[test]
fn an_unclassified_record_of_any_shape_is_reported() {
    let documents: Documents = encode_documents(fixture().snapshot().unwrap()).unwrap();
    let cases = [
        ("board.json", "text", json!({"content":"R1","at":{"x_nm":0,"y_nm":0}}), "board.json:text"),
        ("board.json", "texts", json!([{"content":"R1"}]), "board.json:texts[]"),
        ("board.json", "marks", json!(["wire-1:from"]), "board.json:marks[]"),
        ("board.json", "dimensions", json!([]), "board.json:dimensions[]"),
        ("board.json", "notes", json!({"00000000-0000-4000-8000-000000000001":"R1"}), "board.json:notes.{id}"),
    ];
    for (document, key, value, path) in cases {
        let mut changed = documents.clone();
        let name = changed.keys().find(|name| name.rsplit('/').next() == Some(document)).unwrap().clone();
        let mut root: Value = serde_json::from_slice(&changed[&name]).unwrap();
        root[key] = value;
        changed.insert(name, serde_json::to_vec(&root).unwrap());
        let (problems, _) = inventory_problems(&changed);
        assert!(problems.iter().any(|problem| problem.starts_with(&format!("unclassified record {path}:"))), "{path}: {problems:?}");
    }

    let mut changed = documents;
    changed.insert("dimensions.json".into(), br#"{"items":[]}"#.to_vec());
    let (problems, _) = inventory_problems(&changed);
    assert!(problems.iter().any(|problem| problem.starts_with("unclassified record dimensions.json::")), "{problems:?}");
}

/// Pin and pad tokens round-trip through the shared command vocabulary and name a saved owner.
#[test]
fn pin_and_pad_tokens_name_their_saved_owners() {
    let session = fixture();
    let project = session.snapshot().unwrap();
    let mut seen = Seen::default();
    for (path, bytes) in &encode_documents(project.clone()).unwrap() {
        seen.document(path, bytes);
    }
    let owner_ids = |path: &str| seen.ids.get(path).cloned().unwrap_or_default();

    let terminal = project.symbols()[0].terminals()[0];
    let token = terminal.inspect();
    assert_eq!(schematic::wiring::Terminal::parse(&token), Some(terminal));
    let symbol = token["symbol"].as_str().unwrap().strip_prefix("symbol-").unwrap();
    assert!(owner_ids("sheets/*:symbols[]").contains(symbol));

    let pad = project.pad_for_terminal(terminal).unwrap();
    let token = pad.inspect();
    let footprint = token["footprint"].as_str().unwrap();
    let parsed = board::Pad::parse(footprint, token["pad"].as_str().unwrap()).unwrap();
    assert_eq!(parsed.on_board(project.board_target().unwrap().board), pad);
    assert!(owner_ids("board.json:footprints[]").contains(footprint.strip_prefix("footprint-").unwrap()));
}

/// Saving the audited project again after reopening it gives identical documents, so every id
/// shown by the inventory survives a save and reopen unchanged.
#[test]
fn audited_identities_survive_save_and_reopen() {
    let project = fixture().snapshot().unwrap();
    for compact in [false, true] {
        let project = Arc::new(Project { prefer_compact_part_groups: compact, ..Project::clone(&project) });
        let documents = encode_documents(project).unwrap();
        assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    }
}
