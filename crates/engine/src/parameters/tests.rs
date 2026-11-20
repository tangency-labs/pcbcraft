use super::*;
use crate::persistence::{ENTRY_POINT, decode_documents, decode_packed, encode_documents, encode_packed};
use crate::{CommandError, Session};

/// Start a schematic-only project, as parameters must work before any board exists.
fn schematic() -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    session
}

/// Submit an edit at the current revision through the shared command path.
fn edit(session: &mut Session, command: &str, mut params: Value) -> Result<u64, CommandError> {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params)
}

/// The first sheet's UUID string.
fn main_sheet(session: &Session) -> String {
    session.project().unwrap().sheet_target().id().to_string()
}

/// Resolve a text on `sheet` through `parameter.resolve`.
fn resolve(session: &mut Session, sheet: &str, text: &str) -> Value {
    session.execute(RESOLVE, &json!({"sheet":sheet,"text":text})).unwrap()
}

#[test]
fn names_are_words_that_start_with_a_letter() {
    for good in ["Revision", "MPN", "drawn_by", "R2"] {
        assert!(valid_name(good), "{good}");
    }
    for bad in ["", "2nd", "_x", "Drawn by", "a.b", "Ω", &"x".repeat(65)] {
        assert!(!valid_name(bad), "{bad}");
    }
    let mut list = Parameters::new();
    list.insert("Revision".into(), "B".into());
    assert_eq!(check(&list, Scope::Project), Ok(()));
    list.insert("bad name".into(), String::new());
    assert!(matches!(check(&list, Scope::Project), Err(ParameterError::Name(_))));
    list.clear();
    list.insert("Long".into(), "x".repeat(MAX_VALUE_LEN + 1));
    assert!(matches!(check(&list, Scope::Project), Err(ParameterError::Value(_))));
    let many: Parameters = (0..=MAX_PARAMETERS).map(|i| (format!("P{i}"), String::new())).collect();
    assert_eq!(check(&many, Scope::Project), Err(ParameterError::TooMany));
    // Editing ignores case, so a twin spelling of a standard or stored name could never be edited.
    for twins in [vec!["revision"], vec!["Gain", "gain"]] {
        let list: Parameters = twins.into_iter().map(|name| (name.to_owned(), "x".to_owned())).collect();
        assert!(matches!(check(&list, Scope::Project), Err(ParameterError::Spelling(_))));
    }
    let list: Parameters = [("title".to_owned(), "x".to_owned())].into();
    assert!(matches!(check(&list, Scope::Sheet), Err(ParameterError::Spelling(_))));
    assert_eq!(check(&list, Scope::Project), Ok(()), "only the list's own standard names count");
    // A built-in is read only, so a stored twin of it could never be edited.
    for scope in [Scope::Project, Scope::Sheet] {
        for name in ["Name", "name"] {
            let list: Parameters = [(name.to_owned(), "x".to_owned())].into();
            assert_eq!(check(&list, scope), Err(ParameterError::Builtin(name.to_owned())), "{scope:?} {name}");
        }
    }
}

#[test]
fn texts_split_into_plain_text_and_references() {
    assert_eq!(
        pieces("Rev @project.Revision, mail a@b.com @@x"),
        vec![
            Piece::Text("Rev "),
            Piece::Reference { scope: Scope::Project, name: "Revision", raw: "@project.Revision" },
            Piece::Text(", mail a@b.com @"),
            Piece::Text("x"),
        ]
    );
    assert!(has_references("@sheet.Title"));
    assert!(!has_references("@sheet."));
    assert!(!has_references("@part.MPN"));
    assert!(!has_references("@sheet.2nd"));
}

/// Sheet values default to the project's, so one project value shows on every sheet until a
/// sheet overrides it; Title defaults to the sheet name. References nest, case need not match,
/// and loops or unknown names stay as typed.
#[test]
fn sheet_values_default_to_the_project_and_references_nest() {
    let mut s = schematic();
    let sheet = main_sheet(&s);
    let r = resolve(&mut s, &sheet, "@sheet.Title rev [@sheet.Revision]");
    assert_eq!(r["text"], "Main rev []");
    assert_eq!(r["unknown"], json!([]));
    edit(&mut s, SET, json!({"scope":"project","name":"Revision","value":"B"})).unwrap();
    edit(&mut s, SET, json!({"scope":"project","name":"Label","value":"@project.Name/@sheet.Title"})).unwrap();
    edit(&mut s, SET, json!({"scope":"project","name":"Loop","value":"@project.Loop"})).unwrap();
    assert_eq!(resolve(&mut s, &sheet, "@sheet.Revision @sheet.revision")["text"], "B B");
    edit(&mut s, SET, json!({"scope":"sheet","sheet":sheet,"name":"revision","value":"C"})).unwrap();
    assert_eq!(s.project().unwrap().sheet_parameters(s.project().unwrap().sheet_target()).unwrap().keys().collect::<Vec<_>>(), ["Revision"]);
    let r = resolve(&mut s, &sheet, "@sheet.Revision/@project.Revision @project.Label @project.Loop @sheet.Nope a@b @@");
    let name = s.project().unwrap().name.clone();
    assert_eq!(r["text"], format!("C/B {name}/Main @project.Loop @sheet.Nope a@b @"));
    assert_eq!(r["unknown"], json!(["@project.Loop", "@sheet.Nope"]));
    // Without a sheet, only @project resolves.
    let r = s.execute(RESOLVE, &json!({"text":"@project.Revision @sheet.Title"})).unwrap();
    assert_eq!((r["text"].as_str(), r["unknown"].clone()), (Some("B @sheet.Title"), json!(["@sheet.Title"])));
}

/// Set, rename and remove are each one undo step; equal values, bad names, built-ins, stale
/// revisions and missing targets change nothing.
#[test]
fn edits_are_undoable_and_invalid_ones_change_nothing() {
    let mut s = schematic();
    let sheet = main_sheet(&s);
    let before = s.snapshot().unwrap();
    edit(&mut s, SET, json!({"scope":"sheet","sheet":sheet,"name":"Gain","value":"10"})).unwrap();
    let revision = s.revision();
    edit(&mut s, SET, json!({"scope":"sheet","sheet":sheet,"name":"Gain","value":"10"})).unwrap();
    assert_eq!(s.revision(), revision);
    edit(&mut s, SET, json!({"scope":"sheet","sheet":sheet,"name":"Gain","rename":"Gain_dB"})).unwrap();
    let listed = s.execute(LIST, &json!({"scope":"sheet","sheet":sheet})).unwrap();
    assert_eq!(listed["parameters"], json!({"Gain_dB":"10"}));
    let rows = listed["effective"].as_array().unwrap();
    assert_eq!(rows[0], json!({"name":"Name","value":"Main","source":"built_in"}));
    assert_eq!(rows[1], json!({"name":"Title","value":"@sheet.Name","source":"default"}));
    assert_eq!(rows.last().unwrap(), &json!({"name":"Gain_dB","value":"10","source":"own"}));
    let snapshot = s.snapshot().unwrap();
    for params in [
        json!({"scope":"sheet","sheet":sheet,"name":"Name","value":"x"}),
        json!({"scope":"project","name":"name","value":"x"}),
        json!({"scope":"sheet","sheet":sheet,"name":"2nd","value":"x"}),
        json!({"scope":"sheet","sheet":sheet,"name":"Gain"}),
        json!({"scope":"sheet","sheet":sheet,"name":"Missing","rename":"Other"}),
        // null removes, so it can't also keep the value under a new name.
        json!({"scope":"sheet","sheet":sheet,"name":"Gain_dB","rename":"Other","value":null}),
        json!({"scope":"sheet","sheet":uuid::Uuid::nil(),"name":"X","value":"x"}),
        json!({"scope":"board","name":"X","value":"x"}),
        json!({"scope":"sheet","sheet":sheet,"name":"X","value":"x".repeat(MAX_VALUE_LEN + 1)}),
    ] {
        assert!(edit(&mut s, SET, params.clone()).is_err(), "{params}");
        assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    }
    let mut stale = json!({"scope":"project","name":"Revision","value":"A","expected_revision":s.revision() + 1});
    assert!(s.apply(SET, &stale).is_err());
    stale["expected_revision"] = json!(s.revision());
    edit(&mut s, SET, json!({"scope":"sheet","sheet":sheet,"name":"Gain_dB","value":null})).unwrap();
    assert!(s.project().unwrap().sheet_parameters(s.project().unwrap().sheet_target()).unwrap().is_empty());
    for _ in 0..3 {
        s.apply("edit.undo", &Value::Null).unwrap();
    }
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// `@instance.Page` counts sheets in Browser order: roots in project order, children after
/// their parent.
#[test]
fn page_numbers_follow_browser_order() {
    let mut s = schematic();
    let main = main_sheet(&s);
    edit(&mut s, crate::sheets::CREATE_SHEET, json!({"name":"Other"})).unwrap();
    edit(&mut s, crate::hierarchy::CREATE_CHILD, json!({"sheet":main,"name":"Child","x_nm":0,"y_nm":0})).unwrap();
    let project = s.project().unwrap();
    let id = |name: &str| project.sheets().find(|sheet| sheet.name == name).unwrap().target.id().to_string();
    let (child, other) = (id("Child"), id("Other"));
    assert_eq!(resolve(&mut s, &main, "@instance.Page/@instance.Pages")["text"], "1/3");
    assert_eq!(resolve(&mut s, &child, "@instance.Name @instance.Page")["text"], "Child 2");
    assert_eq!(resolve(&mut s, &other, "@instance.Page")["text"], "3");
}

/// Parameters round-trip byte for byte in folders and archives. Sheet values live in sheet/9 and
/// project values make the entry point project/22; a project without any keeps its old bytes.
#[test]
fn parameters_are_stored_one_per_line_and_round_trip() {
    let mut s = schematic();
    let sheet = main_sheet(&s);
    let untouched = encode_documents(s.snapshot().unwrap()).unwrap();
    let entry: Value = serde_json::from_slice(&untouched[ENTRY_POINT]).unwrap();
    assert!(entry.get("parameters").is_none());
    assert_eq!(entry["format"], "pcbcraft-project/52");
    edit(&mut s, SET, json!({"scope":"sheet","sheet":sheet,"name":"Title","value":"Power supply"})).unwrap();
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(documents[ENTRY_POINT], untouched[ENTRY_POINT]);
    let sheet_file = format!("sheets/{sheet}.json");
    let text = String::from_utf8(documents[&sheet_file].clone()).unwrap();
    assert!(text.contains("  \"parameters\": {\n    \"Title\": \"Power supply\"\n  },\n"), "{text}");
    edit(&mut s, SET, json!({"scope":"project","name":"Revision","value":"B"})).unwrap();
    edit(&mut s, SET, json!({"scope":"project","name":"Company","value":"Tangency"})).unwrap();
    let project = s.snapshot().unwrap();
    let documents = encode_documents(project.clone()).unwrap();
    let entry: Value = serde_json::from_slice(&documents[ENTRY_POINT]).unwrap();
    assert_eq!(entry["format"], "pcbcraft-project/52");
    assert_eq!(entry["parameters"], json!({"Company":"Tangency","Revision":"B"}));
    let reopened = decode_documents(&documents).unwrap();
    assert_eq!(reopened.parameters(), project.parameters());
    assert_eq!(reopened.resolve_text(Some(reopened.sheet_target()), "@sheet.Title @sheet.Revision").text, "Power supply B");
    assert_eq!(documents, encode_documents(reopened).unwrap());
    let archive = encode_packed(project).unwrap();
    assert_eq!(archive, encode_packed(decode_packed(&archive).unwrap().project).unwrap());
    // Changing one value changes one line.
    edit(&mut s, SET, json!({"scope":"project","name":"Revision","value":"C"})).unwrap();
    let changed = encode_documents(s.snapshot().unwrap()).unwrap();
    let lines = |bytes: &[u8]| String::from_utf8(bytes.to_vec()).unwrap().lines().map(str::to_owned).collect::<Vec<_>>();
    let (old, new) = (lines(&documents[ENTRY_POINT]), lines(&changed[ENTRY_POINT]));
    assert_eq!(old.iter().zip(&new).filter(|(a, b)| a != b).count(), 1);
    // Removing the last project value returns to the previous format.
    edit(&mut s, SET, json!({"scope":"project","name":"Revision","value":null})).unwrap();
    edit(&mut s, SET, json!({"scope":"project","name":"Company","value":null})).unwrap();
    let entry: Value = serde_json::from_slice(&encode_documents(s.snapshot().unwrap()).unwrap()[ENTRY_POINT]).unwrap();
    assert!(entry.get("parameters").is_none());
}

/// Decoding refuses parameter lists in the wrong format, empty or invalid lists, and keeps
/// project/22's other extensions optional.
#[test]
fn decoding_rejects_misplaced_or_invalid_lists() {
    let mut s = schematic();
    let sheet = main_sheet(&s);
    edit(&mut s, SET, json!({"scope":"project","name":"Revision","value":"B"})).unwrap();
    edit(&mut s, SET, json!({"scope":"sheet","sheet":sheet,"name":"Title","value":"T"})).unwrap();
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let sheet_file = format!("sheets/{sheet}.json");
    let mutate = |path: &str, change: &dyn Fn(&mut Value)| {
        let mut documents = documents.clone();
        let mut value: Value = serde_json::from_slice(&documents[path]).unwrap();
        change(&mut value);
        documents.insert(path.to_owned(), serde_json::to_vec(&value).unwrap());
        decode_documents(&documents)
    };
    assert!(mutate(ENTRY_POINT, &|v| v["format"] = json!("pcbcraft-project/21")).is_err());
    assert!(mutate(ENTRY_POINT, &|v| v["parameters"] = json!({})).is_err());
    assert!(mutate(ENTRY_POINT, &|v| v["parameters"] = json!({"bad name":"x"})).is_err());
    // Project/52 makes the list optional; project/22 cannot omit it.
    assert!(mutate(ENTRY_POINT, &|v| drop(v.as_object_mut().unwrap().remove("parameters"))).unwrap().parameters().is_empty());
    assert!(
        mutate(ENTRY_POINT, &|v| {
            v["format"] = json!("pcbcraft-project/22");
            v.as_object_mut().unwrap().remove("parameters");
        })
        .is_err()
    );
    assert!(mutate(&sheet_file, &|v| v["parameters"] = json!({})).is_err());
    assert!(mutate(&sheet_file, &|v| v["parameters"] = json!({"Title":"x".repeat(MAX_VALUE_LEN + 1)})).is_err());
    assert!(mutate(&sheet_file, &|v| v["parameters"] = json!({"Title": 3})).is_err());
    assert!(mutate(ENTRY_POINT, &|v| v["parameters"] = json!({"Revision":"B","revision":"C"})).is_err());
    assert!(mutate(&sheet_file, &|v| v["parameters"] = json!({"title":"x"})).is_err());
    for name in ["Name", "name"] {
        assert!(mutate(ENTRY_POINT, &|v| v["parameters"] = json!({name:"stored"})).is_err(), "project {name}");
        assert!(mutate(&sheet_file, &|v| v["parameters"] = json!({name:"stored"})).is_err(), "sheet {name}");
    }
    assert!(mutate(&sheet_file, &|_| {}).is_ok());
}

/// A value naming itself many times branches at every level; the shared expansion budget stops
/// it quickly, leaves the remaining references as typed and reports bounded highlight spans.
#[test]
fn branching_self_references_stay_bounded() {
    let mut s = schematic();
    let sheet = main_sheet(&s);
    let looping = vec!["@project.Loop"; 50].join(" ");
    assert!(looping.len() <= MAX_VALUE_LEN);
    edit(&mut s, SET, json!({"scope":"project","name":"Loop","value":looping})).unwrap();
    let r = resolve(&mut s, &sheet, "@project.Loop");
    assert!(r["text"].as_str().unwrap().len() <= MAX_RESOLVED_LEN);
    assert_eq!(r["unknown"], json!(["@project.Loop"]));
    let project = s.project().unwrap();
    let resolved = project.resolve_text(Some(project.sheet_target()), "@project.Loop");
    let spans: Vec<_> = resolved.text.match_indices("@project.Loop").map(|(start, reference)| start..start + reference.len()).collect();
    assert!(!spans.is_empty());
    assert!(spans.len() <= MAX_RESOLVED_LEN / "@project.Loop".len());
    assert_eq!(resolved.unknown_spans, spans, "every unresolved reference has exactly its own valid byte span");
}

/// The output limit is strict: many references to a long value expand only while the whole
/// result still fits, and the rest stay as typed.
#[test]
fn resolved_text_never_passes_the_output_limit() {
    let mut s = schematic();
    let sheet = main_sheet(&s);
    edit(&mut s, SET, json!({"scope":"project","name":"Long","value":"x".repeat(MAX_VALUE_LEN)})).unwrap();
    // 70 × 1000 bytes would pass 64 KiB; the text after the last reference must still fit.
    let r = resolve(&mut s, &sheet, &("@project.Long ".repeat(70) + "tail"));
    let resolved = r["text"].as_str().unwrap();
    assert!(resolved.len() <= MAX_RESOLVED_LEN, "{}", resolved.len());
    assert!(resolved.starts_with(&"x".repeat(MAX_VALUE_LEN)), "earlier references still expand");
    assert!(resolved.ends_with("@project.Long tail"), "later references stay as typed");
    assert_eq!(r["unknown"], json!(["@project.Long"]));
}

/// A single value lookup reads the same sources as the rows, exact spelling first.
#[test]
fn value_lookup_matches_the_listed_rows() {
    let mut s = schematic();
    let sheet = main_sheet(&s);
    edit(&mut s, SET, json!({"scope":"project","name":"Revision","value":"B"})).unwrap();
    edit(&mut s, SET, json!({"scope":"project","name":"Gain","value":"10"})).unwrap();
    edit(&mut s, SET, json!({"scope":"sheet","sheet":sheet,"name":"Title","value":"Power"})).unwrap();
    let project = s.project().unwrap();
    let target = Some(project.sheet_target());
    for scope in Scope::ALL {
        for row in project.parameter_rows(scope, target) {
            assert_eq!(project.parameter_value(scope, target, &row.name), Some(row.value.clone()), "{scope:?} {}", row.name);
            assert_eq!(project.parameter_value(scope, target, &row.name.to_lowercase()), Some(row.value), "{scope:?} ignoring case");
        }
        assert_eq!(project.parameter_value(scope, target, "Missing"), None);
    }
}

/// Setting a standard parameter to its default stores nothing, so an empty project Revision
/// adopts no storage layout; a custom parameter keeps its empty value.
#[test]
fn standard_values_equal_to_the_default_are_not_stored() {
    let mut s = schematic();
    let sheet = main_sheet(&s);
    let revision = s.revision();
    edit(&mut s, SET, json!({"scope":"project","name":"Revision","value":""})).unwrap();
    edit(&mut s, SET, json!({"scope":"sheet","sheet":sheet,"name":"revision","value":"@project.Revision"})).unwrap();
    assert_eq!(s.revision(), revision);
    edit(&mut s, SET, json!({"scope":"sheet","sheet":sheet,"name":"Revision","value":"B"})).unwrap();
    edit(&mut s, SET, json!({"scope":"sheet","sheet":sheet,"name":"Revision","value":"@project.Revision"})).unwrap();
    let project = s.project().unwrap();
    assert!(project.parameters().is_empty());
    assert!(project.sheet_parameters(project.sheet_target()).unwrap().is_empty());
    edit(&mut s, SET, json!({"scope":"project","name":"Checked","value":""})).unwrap();
    assert_eq!(s.project().unwrap().parameters().get("Checked").map(String::as_str), Some(""));
    // Renaming onto a standard name follows the same rule, and a custom empty value survives a
    // rename to another custom name.
    edit(&mut s, SET, json!({"scope":"sheet","sheet":sheet,"name":"Old","value":"@project.Revision"})).unwrap();
    edit(&mut s, SET, json!({"scope":"sheet","sheet":sheet,"name":"Old","rename":"revision"})).unwrap();
    let project = s.project().unwrap();
    assert!(project.sheet_parameters(project.sheet_target()).unwrap().is_empty());
    edit(&mut s, SET, json!({"scope":"project","name":"Checked","rename":"Approved"})).unwrap();
    assert_eq!(s.project().unwrap().parameters().get("Approved").map(String::as_str), Some(""));
}

/// A sheet without paper can't store parameters, so editing one is refused and changes nothing.
#[test]
fn sheet_parameters_need_paper() {
    let mut s = schematic();
    let sheet = main_sheet(&s);
    let mut project = (*s.project().unwrap()).clone();
    for info in Arc::make_mut(&mut project.sheets) {
        info.page = None;
    }
    s.open_snapshot(Arc::new(project));
    let before = s.snapshot().unwrap();
    let error = edit(&mut s, SET, json!({"scope":"sheet","sheet":sheet,"name":"Gain","value":"10"})).unwrap_err();
    assert!(error.to_string().contains("needs paper"), "{error}");
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    edit(&mut s, SET, json!({"scope":"project","name":"Gain","value":"10"})).expect("project parameters don't need paper");
}

/// The legacy diagnostic encoding refuses sheet parameters rather than dropping them, even on a
/// sheet without paper.
#[test]
fn diagnostic_encoding_refuses_sheet_parameters() {
    let mut s = schematic();
    let sheet = main_sheet(&s);
    edit(&mut s, SET, json!({"scope":"sheet","sheet":sheet,"name":"Gain","value":"10"})).unwrap();
    let mut project = (*s.project().unwrap()).clone();
    project.circuit_use = None;
    for info in Arc::make_mut(&mut project.sheets) {
        info.page = None;
    }
    let error = crate::persistence::encode(Arc::new(project)).unwrap_err();
    assert!(error.contains("sheet parameters"), "{error}");
}
