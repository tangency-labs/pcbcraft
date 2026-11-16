//! Every saved object kind carries a persistent UUID, an owner token, or is a listed gap.
//!
//! The audit reads the production documents of a command-built project rather than a list of
//! types, so a newly persisted record without a classification fails here. The record and kind
//! tables below are the inventory described in `docs/object-identity.md`.
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
    FootprintInstance,
    TrackSegment,
    Arc,
    Via,
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
    const ALL: [Kind; 26] = [
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
        Kind::FootprintInstance,
        Kind::TrackSegment,
        Kind::Arc,
        Kind::Via,
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
            Kind::FootprintInstance => Records(&["board.json:footprints[]"]),
            Kind::TrackSegment => Records(&["board.json:tracks[]"]),
            Kind::Via => Records(&["board.json:vias[]"]),
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
const RECORDS: &[(&str, &[Form], &str)] = &[
    ("project.json:", &[Form::Uuid], "project"),
    ("project.json:boards[]", &[Form::Uuid], "board registration"),
    ("project.json:sheets[]", &[Form::Uuid], "sheet reference"),
    ("project.json:components[]", &[Form::Counter], "component"),
    ("project.json:net_classes.definitions[]", &[Form::Uuid], "net class"),
    ("sheets/*:", &[Form::Uuid], "sheet document"),
    ("sheets/*:symbols[]", &[Form::Counter], "placed symbol unit"),
    ("sheets/*:wires[]", &[Form::Counter], "wire"),
    ("sheets/*:wires[].bends[]", &[Form::Owned], "wire bend point"),
    ("sheets/*:junctions[]", &[Form::Counter], "junction"),
    ("sheets/*:net_labels[]", &[Form::Counter], "net label"),
    ("sheets/*:labels[]", &[Form::Counter], "hierarchical label (port)"),
    ("sheets/*:subsheets[]", &[Form::Counter], "sheet symbol"),
    ("board.json:", &[Form::Owned], "board document; the board's id is its registration"),
    ("board.json:setup.layers[]", &[Form::Uuid], "layer"),
    ("board.json:setup.stackup", &[Form::Uuid], "stackup"),
    ("board.json:setup.stackup.strata[]", &[Form::Uuid, Form::Owned], "stratum; copper strata are addressed by their layer"),
    ("board.json:rules", &[Form::Uuid], "board rule set"),
    ("board.json:baseline.nets[]", &[Form::Owned], "accepted connection"),
    ("board.json:baseline.nets[].terminals[]", &[Form::Owned], "accepted connection terminal"),
    ("board.json:assignments[]", &[Form::Owned], "package choice, keyed by component"),
    ("board.json:graphics.items[]", &[Form::Uuid], "board drawing"),
    ("board.json:graphics.items[].points[]", &[Form::Owned], "board drawing vertex"),
    ("board.json:tracks[]", &[Form::Uuid], "track segment"),
    ("board.json:vias[]", &[Form::Uuid], "via"),
    ("board.json:footprints[]", &[Form::Counter], "footprint instance"),
    ("instances.json:", &[Form::Owned], "instance document of the registered board"),
    ("instances.json:circuit_uses[]", &[Form::Uuid], "circuit use"),
    ("instances.json:sheet_occurrences[]", &[Form::Uuid], "sheet occurrence"),
    ("instances.json:net_registry.records[]", &[Form::Uuid], "net"),
    ("instances.json:net_registry.records[].fallbacks[]", &[Form::Owned], "net fallback anchor"),
    ("instances.json:net_registry.records[].primary.scope", &[Form::Uuid], "scope reference in a net anchor"),
    ("instances.json:net_registry.records[].fallbacks[].scope", &[Form::Uuid], "scope reference in a net anchor"),
    ("instances.json:net_registry.records[].class_bindings[]", &[Form::Owned], "net class binding"),
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

/// A routed board plus one of every other saved record, built only through shared commands.
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
    /// Record each document root, array element object and singleton object with an `id`. Library
    /// definitions are versioned as a whole, so only their root is an addressed record.
    fn document(&mut self, path: &str, bytes: &[u8]) {
        let value: Value = serde_json::from_slice(bytes).unwrap();
        let role = document_role(path);
        let library = role.starts_with("library/");
        self.walk(&format!("{role}:"), &value, true, !library);
    }

    /// Visit one JSON value. Map keys that are ids fold to `{id}` so paths name roles, not objects.
    fn walk(&mut self, path: &str, value: &Value, record: bool, descend: bool) {
        match value {
            Value::Object(object) => {
                if record || object.contains_key("id") {
                    self.forms.entry(path.into()).or_default().insert(form(object));
                    if let Some(id) = object.get("id") {
                        let id = id.as_str().map_or_else(|| id.to_string(), str::to_owned);
                        self.ids.entry(path.into()).or_default().insert(id);
                    }
                }
                if descend {
                    for (key, child) in object {
                        let key = if uuid::Uuid::parse_str(key).is_ok() || key.bytes().all(|b| b.is_ascii_digit()) { "{id}" } else { key };
                        let separator = if path.ends_with(':') { "" } else { "." };
                        self.walk(&format!("{path}{separator}{key}"), child, false, true);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.walk(&format!("{path}[]"), item, true, true);
                }
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

/// Every saved record is classified, every kind has an address, and the kinds without a
/// persistent UUID are exactly the listed gaps.
#[test]
fn every_saved_object_kind_has_a_stable_identity_or_is_a_listed_gap() {
    let session = fixture();
    let documents: Documents = encode_documents(session.snapshot().unwrap()).unwrap();
    let mut seen = Seen::default();
    for (path, bytes) in &documents {
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
    assert!(problems.is_empty(), "{}\nsaved records:\n{saved}", problems.join("\n"));
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
    let documents = encode_documents(fixture().snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
}
