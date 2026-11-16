use super::*;
use crate::{
    DRAW_WIRE, PLACE_RESISTOR, PLACE_SYMBOL, PLACE_UNIT, SheetTarget, WireEndpoint,
    hierarchy::{ADD_LABEL, CREATE_CHILD},
    persistence::{decode_documents, encode_documents},
    schematic::Point,
};
use std::{collections::BTreeSet, sync::Arc};

const SYMBOL: &str = include_str!("../../../../examples/native-parts/symbols/dual-amplifier.json");
const SCHEMA: &str = include_str!("../../../../docs/schemas/netlist-v1.schema.json");

/// Submit authored changes through the same revision-checked path as GUI and MCP.
fn apply(s: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(s.revision());
    s.apply(command, &params).unwrap();
}

/// A new project with the dual amplifier's unit `a` placed on the root sheet.
fn session() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    s.execute(
        "library.load_source",
        &json!({"source":"units", "documents":{
            "library.json":r#"{"format":"pcbcraft-library/1","id":"units","name":"Unit tests","license":"MIT OR Apache-2.0"}"#,
            "symbols/dual.json":SYMBOL,
        }}),
    )
    .unwrap();
    let definition = s.libraries.symbol("units", "dual-amplifier", 1).unwrap();
    let digest = crate::library::sources::symbol_digest(&definition).unwrap();
    apply(
        &mut s,
        PLACE_SYMBOL,
        json!({"library":"units","definition":"dual-amplifier","definition_revision":1,"definition_digest":digest,"x_nm":0,"y_nm":0}),
    );
    s
}

/// The ID of the most recently placed symbol on a sheet.
fn last_symbol(s: &Session, sheet: SheetTarget) -> String {
    s.project().unwrap().for_sheet(sheet).unwrap().symbols().last().unwrap().id()
}

/// One terminal of a placed symbol, addressed by its logical terminal number.
fn terminal(s: &Session, sheet: SheetTarget, symbol: &str, number: &str) -> WireEndpoint {
    let projection = s.project().unwrap().for_sheet(sheet).unwrap();
    let placed = projection.symbols().iter().find(|placed| placed.id() == symbol).unwrap();
    WireEndpoint::Terminal(placed.terminals()[placed.unit_definition().pin_index(number).unwrap()])
}

/// Draw one wire between two endpoints of a sheet, with a single bend when they are not aligned.
fn wire(s: &mut Session, sheet: SheetTarget, from: WireEndpoint, to: WireEndpoint) {
    let projection = s.project().unwrap().for_sheet(sheet).unwrap();
    let start = projection.endpoint_position(from).unwrap();
    let end = projection.endpoint_position(to).unwrap();
    let bends = if start.x_nm != end.x_nm && start.y_nm != end.y_nm { vec![Point { x_nm: start.x_nm, y_nm: end.y_nm }] } else { vec![] };
    apply(s, DRAW_WIRE, json!({"sheet":sheet.id(),"from":from.inspect(),"to":to.inspect(),"bends":bends,"auto_heal":false}));
}

/// Three nested sheets. U1 is the two-unit amplifier: unit `a` on the root sheet and unit `b`
/// on the child; terminals 3 and 4 are drawn on both units. R1 sits on the root sheet and R2
/// on the grandchild.
///
/// - U1.2 to R1.1 on the root sheet.
/// - U1.3 to R1.2 on the root sheet; unit `b` draws the same terminal 3 again.
/// - U1.1 on the root sheet to the child's `OUT` port, which reaches U1.5 on the child.
/// - U1.6 on the child to the grandchild's `IN` port, which reaches R2.1.
/// - U1.4 and R2.2 stay unconnected.
struct Fixture {
    session: Session,
    sheets: [SheetTarget; 3],
    u1: String,
    r1: String,
    r2: String,
    /// The expected nets as `(reference, terminal)` pairs, in export order: by first terminal.
    expected: Vec<BTreeSet<(&'static str, &'static str)>>,
}

fn fixture() -> Fixture {
    let mut s = session();
    let root = s.project().unwrap().sheet_target();
    let unit_a = last_symbol(&s, root);
    let u1 = s.project().unwrap().symbols()[0].component_id();
    apply(&mut s, PLACE_RESISTOR, json!({"sheet":root.id(),"x_nm":0,"y_nm":20_000_000}));
    let r1_symbol = last_symbol(&s, root);
    apply(&mut s, CREATE_CHILD, json!({"name":"Amplifier","x_nm":40_000_000,"y_nm":0}));
    let child = s.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut s, CREATE_CHILD, json!({"sheet":child.id(),"name":"Load","x_nm":40_000_000,"y_nm":0}));
    let grandchild = s.project().unwrap().sheets().nth(2).unwrap().target;
    apply(&mut s, ADD_LABEL, json!({"sheet":child.id(),"name":"OUT","direction":"input","x_nm":-20_000_000,"y_nm":0}));
    let out = s.project().unwrap().for_sheet(child).unwrap().labels[0].id;
    apply(&mut s, ADD_LABEL, json!({"sheet":grandchild.id(),"name":"IN","direction":"input","x_nm":0,"y_nm":0}));
    let input = s.project().unwrap().for_sheet(grandchild).unwrap().labels[0].id;
    apply(&mut s, PLACE_UNIT, json!({"sheet":child.id(),"component":u1,"unit":"b","x_nm":0,"y_nm":0}));
    let unit_b = last_symbol(&s, child);
    apply(&mut s, PLACE_RESISTOR, json!({"sheet":grandchild.id(),"x_nm":10_000_000,"y_nm":0}));
    let r2_symbol = last_symbol(&s, grandchild);

    let (from, to) = (terminal(&s, root, &unit_a, "2"), terminal(&s, root, &r1_symbol, "1"));
    wire(&mut s, root, from, to);
    let (from, to) = (terminal(&s, root, &unit_a, "3"), terminal(&s, root, &r1_symbol, "2"));
    wire(&mut s, root, from, to);
    let from = terminal(&s, root, &unit_a, "1");
    wire(&mut s, root, from, WireEndpoint::SheetPin(out));
    let to = terminal(&s, child, &unit_b, "5");
    wire(&mut s, child, WireEndpoint::Label(out), to);
    let from = terminal(&s, child, &unit_b, "6");
    wire(&mut s, child, from, WireEndpoint::SheetPin(input));
    let to = terminal(&s, grandchild, &r2_symbol, "1");
    wire(&mut s, grandchild, WireEndpoint::Label(input), to);

    let project = s.project().unwrap();
    let component = |sheet: SheetTarget, symbol: &str| {
        let projection = project.for_sheet(sheet).unwrap();
        projection.symbols().iter().find(|placed| placed.id() == symbol).unwrap().component_id()
    };
    let (r1, r2) = (component(root, &r1_symbol), component(grandchild, &r2_symbol));
    let expected = vec![
        BTreeSet::from([("U1", "1"), ("U1", "5")]),
        BTreeSet::from([("U1", "2"), ("R1", "1")]),
        BTreeSet::from([("U1", "3"), ("R1", "2")]),
        BTreeSet::from([("U1", "6"), ("R2", "1")]),
    ];
    Fixture { session: s, sheets: [root, child, grandchild], u1, r1, r2, expected }
}

/// Every terminal of every placed part appears exactly once, on the net the engine's own
/// connectivity gives it, across three sheets and both units of a two-unit part.
#[test]
fn hierarchical_multi_unit_netlist_lists_every_terminal_once_on_its_net() {
    let Fixture { session, sheets, u1, r1, r2, expected } = fixture();
    let project = session.project().unwrap();
    let Export { netlist, findings } = project.netlist().unwrap();
    let reference: BTreeMap<_, _> = netlist.components.iter().map(|part| (part.id.clone(), part.reference.clone())).collect();
    assert_eq!(reference.get(&u1).map(String::as_str), Some("U1"));
    assert_eq!(reference.get(&r1).map(String::as_str), Some("R1"));
    assert_eq!(reference.get(&r2).map(String::as_str), Some("R2"));
    assert_eq!(netlist.components.len(), 3);
    assert_eq!(netlist.format, FORMAT);
    assert_eq!(netlist.basis, Basis::Schematic);
    assert_eq!(netlist.project.id, project.id);

    // The amplifier lists its six logical terminals once, in native order, and both placed units.
    let amplifier = netlist.components.iter().find(|part| part.id == u1).unwrap();
    let numbers: Vec<_> = amplifier.terminals.iter().map(|terminal| terminal.terminal.as_str()).collect();
    let native: Vec<_> = project.components().find(|part| part.id() == u1).unwrap().definition.pins().iter().map(|pin| pin.number.clone()).collect();
    assert_eq!(numbers, native);
    assert_eq!(numbers.iter().collect::<BTreeSet<_>>().len(), 6);
    let units: Vec<_> = amplifier.units.iter().map(|unit| (unit.unit.as_str(), unit.sheet)).collect();
    assert_eq!(units, vec![("a", sheets[0].id()), ("b", sheets[1].id())]);
    let occurrences = project.occurrence_instances().sheet_occurrences;
    let occurrence = |sheet: SheetTarget| occurrences.iter().find(|record| record.definition == sheet.id()).unwrap().id;
    assert_eq!(amplifier.units[1].occurrence, vec![occurrence(sheets[0]), occurrence(sheets[1])]);
    let load = netlist.components.iter().find(|part| part.id == r2).unwrap();
    assert_eq!(load.units[0].occurrence, sheets.iter().map(|&sheet| occurrence(sheet)).collect::<Vec<_>>());

    // Each component terminal is on exactly one net or listed once as unconnected.
    let mut seen = BTreeSet::new();
    for terminal in netlist.nets.iter().flat_map(|net| &net.terminals).chain(&netlist.unconnected) {
        assert!(seen.insert((terminal.part.clone(), terminal.terminal.clone())), "{terminal:?} appears twice");
    }
    let every: BTreeSet<_> =
        netlist.components.iter().flat_map(|part| part.terminals.iter().map(|terminal| (part.id.clone(), terminal.terminal.clone()))).collect();
    assert_eq!(seen, every);

    // The nets are exactly the designed ones, in order; the open terminals are listed apart.
    let joined: Vec<_> = netlist.nets.iter().map(|net| named(&reference, net)).collect();
    assert_eq!(joined, expected);
    let open: Vec<_> = netlist.unconnected.iter().map(|t| (reference[&t.part].as_str(), t.terminal.as_str())).collect();
    assert_eq!(open, vec![("U1", "4"), ("R2", "2")]);

    // The same pairs as the engine's connectivity, mapping each unit's terminal to its component.
    let electrical = project.electrical_connectivity();
    let owner: BTreeMap<_, _> = project
        .sheets()
        .flat_map(|sheet| project.for_sheet(sheet.target).unwrap().symbols().iter().map(|s| (s.ordinal, s.component_id())).collect::<Vec<_>>())
        .collect();
    let engine: BTreeSet<BTreeSet<(String, String)>> = electrical
        .nets
        .iter()
        .map(|group| group.iter().map(|t| (owner[&t.symbol].clone(), t.number().to_owned())).collect::<BTreeSet<_>>())
        .filter(|group| !group.is_empty())
        .collect();
    let exported: BTreeSet<BTreeSet<(String, String)>> =
        netlist.nets.iter().map(|net| net.terminals.iter().map(|t| (t.part.clone(), t.terminal.clone())).collect()).collect();
    assert_eq!(exported, engine);
    for net in &netlist.nets {
        let first = (net.terminals[0].part.clone(), net.terminals[0].terminal.clone());
        let index =
            electrical.nets.iter().position(|group| group.iter().any(|t| (owner[&t.symbol].clone(), t.number().to_owned()) == first)).unwrap();
        assert_eq!(net.name, electrical.net_name(index));
        assert_eq!(net.names, electrical.net_names(index));
    }

    // Wired nets get persistent IDs from the registry; each net carries its own.
    let ids: BTreeSet<_> = netlist.nets.iter().map(|net| net.id.unwrap()).collect();
    assert_eq!(ids.len(), netlist.nets.len());
    let active: BTreeSet<_> = project.net_registry.records.iter().map(|record| record.id).collect();
    assert!(ids.is_subset(&active));
    assert_eq!(findings, Vec::<Value>::new());

    // The output validates against the published schema and reads back unchanged.
    let text = netlist.to_text().unwrap();
    validate(&serde_json::from_str(&text).unwrap());
    assert_eq!(Netlist::parse(&text).unwrap(), netlist);
}

/// Repeated exports, a save and reopen, and the execute and apply paths give the same bytes
/// without changing the project or its history.
#[test]
fn export_is_byte_identical_and_edits_nothing() {
    let Fixture { mut session, .. } = fixture();
    let first = session.export_netlist().unwrap().0;
    assert_eq!(session.export_netlist().unwrap().0, first);
    let reopened = decode_documents(&encode_documents(session.snapshot().unwrap()).unwrap()).unwrap();
    assert_eq!(reopened.netlist().unwrap().netlist.to_text().unwrap(), first.contents);
    assert!(first.name.ends_with("-netlist.json"));
    assert!(first.contents.ends_with("}\n"));

    let before = session.snapshot().unwrap();
    let revision = session.revision();
    let result = session.execute(EXPORT_NETLIST, &json!({})).unwrap();
    assert_eq!(result["revision"], json!(revision));
    assert_eq!(result["netlist"]["files"], json!([{"name":first.name,"contents":first.contents}]));
    assert_eq!(result["netlist"]["findings"], json!([]));
    let dry = session.execute(EXPORT_NETLIST, &json!({"dry_run":true})).unwrap();
    assert_eq!(dry["netlist"]["files"], json!([{"name":first.name}]));
    assert_eq!(dry["netlist"]["findings"], result["netlist"]["findings"]);
    let full = session.execute(EXPORT_NETLIST, &json!({"dry_run":false})).unwrap();
    assert_eq!(full["netlist"], result["netlist"], "dry_run false is the same as omitting it");
    assert_eq!(session.apply(EXPORT_NETLIST, &json!({})).unwrap(), revision);
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);
    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(!Arc::ptr_eq(&before, &session.snapshot().unwrap()), "undo steps back over the last wire, not the export");
}

/// Commands without a project, with unknown parameters or with malformed values fail the same
/// way through execute and apply and leave the session as it was.
#[test]
fn rejected_exports_change_nothing() {
    let mut empty = Session::default();
    assert!(empty.execute(EXPORT_NETLIST, &json!({})).unwrap_err().to_string().contains("no project is open"));
    assert!(empty.apply(EXPORT_NETLIST, &json!({})).is_err());
    let mut s = session();
    let before = s.snapshot().unwrap();
    for params in [json!({"unknown":true}), json!({"dry_run":"yes"}), json!({"dry_run":1}), json!({"dry_run":null})] {
        let applied = s.apply(EXPORT_NETLIST, &params).unwrap_err().to_string();
        assert_eq!(s.execute(EXPORT_NETLIST, &params).unwrap_err().to_string(), applied);
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
}

/// A unit that is not placed has no connections, so the export says so rather than inventing nets.
#[test]
fn unplaced_units_are_reported() {
    let s = session();
    let Export { netlist, findings } = s.project().unwrap().netlist().unwrap();
    let unplaced: Vec<_> = findings.iter().filter(|finding| finding["kind"] == "unplaced_unit").map(|finding| finding["unit"].clone()).collect();
    assert_eq!(unplaced, vec![json!("b")]);
    let listed: BTreeSet<_> = netlist.nets.iter().flat_map(|net| &net.terminals).chain(&netlist.unconnected).map(|t| t.terminal.as_str()).collect();
    assert_eq!(listed, BTreeSet::from(["1", "2", "3", "4"]));
    assert_eq!(netlist.components[0].terminals.len(), 6);
}

/// A net the registry does not identify is exported with a null ID and a finding, never with
/// its presentation index.
#[test]
fn nets_without_a_persistent_id_are_reported() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let root = s.project().unwrap().sheet_target();
    apply(&mut s, PLACE_RESISTOR, json!({"sheet":root.id(),"x_nm":0,"y_nm":0}));
    let first = last_symbol(&s, root);
    apply(&mut s, PLACE_RESISTOR, json!({"sheet":root.id(),"x_nm":0,"y_nm":20_000_000}));
    let second = last_symbol(&s, root);
    let (from, to) = (terminal(&s, root, &first, "2"), terminal(&s, root, &second, "1"));
    wire(&mut s, root, from, to);
    let mut older = (*s.snapshot().unwrap()).clone();
    older.net_registry = Arc::default();
    let Export { netlist, findings } = older.netlist().unwrap();
    assert_eq!(netlist.nets.len(), 1);
    assert_eq!(netlist.nets[0].id, None);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["kind"], "unregistered_net");
    assert_eq!(findings[0]["net"], json!(netlist.nets[0].name));
    validate(&serde_json::to_value(&netlist).unwrap());
}

/// Reading refuses other formats and fields the format does not define.
#[test]
fn parse_rejects_foreign_documents() {
    let Fixture { session, .. } = fixture();
    let text = session.export_netlist().unwrap().0.contents;
    let mut value: Value = serde_json::from_str(&text).unwrap();
    value["format"] = json!("pcbcraft-netlist/2");
    assert!(Netlist::parse(&value.to_string()).unwrap_err().contains("unsupported"));
    let mut value: Value = serde_json::from_str(&text).unwrap();
    value.as_object_mut().unwrap().remove("format");
    assert!(Netlist::parse(&value.to_string()).is_err());
    assert!(check(&schema(), &schema(), &value, "$").is_err());
    let mut value: Value = serde_json::from_str(&text).unwrap();
    value["nets"][0]["extra"] = json!(1);
    assert!(Netlist::parse(&value.to_string()).is_err());
    assert!(check(&schema(), &schema(), &value, "$").is_err());
    let mut value: Value = serde_json::from_str(&text).unwrap();
    value["components"][0]["terminals"][0]["electrical_type"] = json!("analog");
    assert!(Netlist::parse(&value.to_string()).is_err());
    assert!(check(&schema(), &schema(), &value, "$").is_err());
}

/// The reader and the schema agree at each field edge: a net ID or terminal name may be null
/// but not omitted, UUIDs use the hyphenated form in either case, and a definition revision
/// must fit the reader's 32-bit field.
#[test]
fn reader_and_schema_agree_on_field_edges() {
    let Fixture { session, .. } = fixture();
    let text = session.export_netlist().unwrap().0.contents;
    let original: Value = serde_json::from_str(&text).unwrap();
    let agree = |value: &Value, accepted: bool| {
        assert_eq!(Netlist::parse(&value.to_string()).is_ok(), accepted, "reader: {value}");
        assert_eq!(check(&schema(), &schema(), value, "$").is_ok(), accepted, "schema: {value}");
    };
    agree(&original, true);
    assert!(original["components"].as_array().unwrap().iter().flat_map(|part| part["terminals"].as_array().unwrap()).any(|t| t["name"].is_null()));
    for pointer in ["/nets/0", "/components/0/terminals/0"] {
        let field = if pointer.starts_with("/nets") { "id" } else { "name" };
        let mut null = original.clone();
        null.pointer_mut(pointer).unwrap()[field] = Value::Null;
        agree(&null, true);
        let mut omitted = original.clone();
        omitted.pointer_mut(pointer).unwrap().as_object_mut().unwrap().remove(field);
        agree(&omitted, false);
    }
    // Every object kind in the document is closed on both sides.
    assert!(!original["unconnected"].as_array().unwrap().is_empty());
    for pointer in [
        "",
        "/project",
        "/components/0",
        "/components/0/definition",
        "/components/0/units/0",
        "/components/0/terminals/0",
        "/nets/0",
        "/nets/0/terminals/0",
        "/unconnected/0",
    ] {
        let mut value = original.clone();
        value.pointer_mut(pointer).unwrap().as_object_mut().unwrap().insert("extra".into(), json!(1));
        agree(&value, false);
    }
    let id = original["project"]["id"].as_str().unwrap().to_owned();
    let simple = id.replace('-', "");
    for spelling in [simple.clone(), format!("{{{id}}}"), format!("urn:uuid:{id}"), id.to_uppercase()] {
        let accepted = spelling == id.to_uppercase();
        for pointer in ["/project/id", "/components/0/units/0/sheet", "/components/0/units/0/occurrence/0", "/nets/0/id"] {
            let mut value = original.clone();
            *value.pointer_mut(pointer).unwrap() = json!(spelling);
            agree(&value, accepted);
        }
    }
    for (revision, accepted) in [(json!(u32::MAX), true), (json!(u64::from(u32::MAX) + 1), false), (json!(-1), false)] {
        let mut value = original.clone();
        value["components"][0]["definition"]["revision"] = revision;
        agree(&value, accepted);
    }
}

/// The schema's closed vocabularies are exactly the ones the code writes.
#[test]
fn schema_vocabularies_match_the_types() {
    let schema = schema();
    let names = |values: Vec<Value>| values.iter().map(Value::to_string).collect::<BTreeSet<_>>();
    let listed = |pointer: &str| names(schema.pointer(pointer).unwrap().as_array().unwrap().clone());
    use ElectricalType::*;
    let types = [Passive, Input, Output, Bidirectional, PowerIn, PowerOut, Unspecified, TriState, OpenCollector, OpenEmitter, NoConnect];
    assert_eq!(listed("/$defs/terminal/properties/electrical_type/enum"), names(types.iter().map(|t| json!(t)).collect()));
    assert_eq!(listed("/$defs/component/properties/kind/enum"), names(vec![json!(ComponentKind::Part), json!(ComponentKind::Power)]));
    assert_eq!(listed("/properties/basis/enum"), names(vec![json!(Basis::Schematic)]));
    assert_eq!(schema["properties"]["format"]["const"], json!(FORMAT));
}

/// A net's terminals as `(reference, terminal)` pairs.
fn named<'a>(reference: &'a BTreeMap<String, String>, net: &'a Net) -> BTreeSet<(&'a str, &'a str)> {
    net.terminals.iter().map(|t| (reference[&t.part].as_str(), t.terminal.as_str())).collect()
}

/// The published schema as JSON.
fn schema() -> Value {
    serde_json::from_str(SCHEMA).unwrap()
}

/// Panic unless the document satisfies the published schema.
fn validate(value: &Value) {
    let schema = schema();
    check(&schema, &schema, value, "$").unwrap();
}

/// A small JSON Schema checker covering exactly the keywords the netlist schema uses. Any
/// other keyword fails, so the schema cannot quietly rely on something this test ignores.
fn check(root: &Value, schema: &Value, value: &Value, at: &str) -> Result<(), String> {
    let schema = schema.as_object().ok_or(format!("{at}: schema is not an object"))?;
    for (keyword, rule) in schema {
        match keyword.as_str() {
            "$schema" | "$id" | "title" | "description" | "$defs" => {}
            "$ref" => {
                let pointer = rule.as_str().and_then(|r| r.strip_prefix('#')).ok_or(format!("{at}: only local references"))?;
                check(root, root.pointer(pointer).ok_or(format!("{at}: missing {pointer}"))?, value, at)?;
            }
            "type" => {
                let allowed: Vec<_> = rule.as_array().cloned().unwrap_or_else(|| vec![rule.clone()]);
                let actual = match value {
                    Value::Null => "null",
                    Value::Bool(_) => "boolean",
                    Value::Number(n) if n.is_u64() || n.is_i64() => "integer",
                    Value::Number(_) => "number",
                    Value::String(_) => "string",
                    Value::Array(_) => "array",
                    Value::Object(_) => "object",
                };
                if !allowed.iter().any(|t| t == actual || (t == "number" && actual == "integer")) {
                    return Err(format!("{at}: {actual} is not {rule}"));
                }
            }
            "const" if value != rule => return Err(format!("{at}: not {rule}")),
            "const" => {}
            "enum" if !rule.as_array().unwrap().contains(value) => return Err(format!("{at}: {value} not in {rule}")),
            "enum" => {}
            "minimum" if value.as_f64().is_some_and(|n| n < rule.as_f64().unwrap()) => return Err(format!("{at}: below {rule}")),
            "minimum" => {}
            "maximum" if value.as_f64().is_some_and(|n| n > rule.as_f64().unwrap()) => return Err(format!("{at}: above {rule}")),
            "maximum" => {}
            "format" if rule == "uuid" => {
                if let Some(text) = value.as_str() {
                    super::parse_hyphenated(text).ok_or_else(|| format!("{at}: not a hyphenated UUID"))?;
                }
            }
            "required" => {
                for name in rule.as_array().unwrap() {
                    if value.is_object() && value.get(name.as_str().unwrap()).is_none() {
                        return Err(format!("{at}: missing {name}"));
                    }
                }
            }
            "properties" => {
                if let Some(object) = value.as_object() {
                    for (name, property) in rule.as_object().unwrap() {
                        if let Some(child) = object.get(name) {
                            check(root, property, child, &format!("{at}.{name}"))?;
                        }
                    }
                }
            }
            "additionalProperties" if rule == false => {
                if let Some(object) = value.as_object() {
                    let known = schema.get("properties").and_then(Value::as_object);
                    if let Some(extra) = object.keys().find(|name| !known.is_some_and(|known| known.contains_key(*name))) {
                        return Err(format!("{at}: unexpected {extra}"));
                    }
                }
            }
            "items" => {
                for (index, item) in value.as_array().into_iter().flatten().enumerate() {
                    check(root, rule, item, &format!("{at}[{index}]"))?;
                }
            }
            "anyOf" => {
                if !rule.as_array().unwrap().iter().any(|option| check(root, option, value, at).is_ok()) {
                    return Err(format!("{at}: matches no alternative"));
                }
            }
            other => return Err(format!("{at}: unsupported schema keyword {other}")),
        }
    }
    Ok(())
}

/// A terminal drawn on two units is one logical terminal: wiring its second appearance joins
/// that net to the first appearance's net, so the terminal still appears in only one net.
#[test]
fn shared_terminal_wired_on_two_units_stays_on_one_net() {
    let Fixture { mut session, sheets, r1, r2, u1, .. } = fixture();
    let child = sheets[1];
    let unit_b = last_symbol(&session, child);
    let (from, to) = (terminal(&session, child, &unit_b, "3"), terminal(&session, child, &unit_b, "4"));
    wire(&mut session, child, from, to);
    let Export { netlist, findings } = session.project().unwrap().netlist().unwrap();
    let reference = BTreeMap::from([(u1, "U1".to_owned()), (r1, "R1".to_owned()), (r2, "R2".to_owned())]);
    let joined: Vec<_> = netlist.nets.iter().map(|net| named(&reference, net)).collect();
    assert_eq!(
        joined,
        vec![
            BTreeSet::from([("U1", "1"), ("U1", "5")]),
            BTreeSet::from([("U1", "2"), ("R1", "1")]),
            BTreeSet::from([("U1", "3"), ("U1", "4"), ("R1", "2")]),
            BTreeSet::from([("U1", "6"), ("R2", "1")]),
        ]
    );
    let open: Vec<_> = netlist.unconnected.iter().map(|t| (reference[&t.part].as_str(), t.terminal.as_str())).collect();
    assert_eq!(open, vec![("R2", "2")]);
    assert_eq!(findings, Vec::<Value>::new());
    validate(&serde_json::from_str(&netlist.to_text().unwrap()).unwrap());
}
