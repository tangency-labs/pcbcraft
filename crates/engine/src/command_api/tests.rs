use serde_json::{Value, json};

use super::*;

const SNAPSHOT: &str = include_str!("../../../../docs/schemas/command-contracts.json");

/// Deprecations invalidate discovery, preserve the locked contract, and warn only on use.
#[test]
fn deprecations_change_discovery_not_the_contract_and_warn_on_explicit_use() {
    let original = json!({"id": "demo", "params": {"properties": {
        "deprecated": {"type": "boolean"}, "data": {"default": {"deprecated": "still part of the default"}}
    }}});
    let deprecations = [
        Deprecation { command: "demo", parameter: None, since: "0.2.0", replacement: Some("other") },
        Deprecation { command: "demo", parameter: Some("deprecated"), since: "0.2.0", replacement: None },
    ];
    let mut marked = original.clone();
    mark_deprecations(&mut marked, &deprecations);
    assert_eq!(marked["deprecated"], true);
    assert_eq!(marked["params"]["properties"]["deprecated"]["deprecated"], true);
    assert_eq!(contract_of(&original), contract_of(&marked));
    assert_eq!(contract_of(&marked)["params"]["properties"]["deprecated"]["type"], "boolean");
    assert_eq!(contract_of(&marked)["params"]["properties"]["data"]["default"]["deprecated"], "still part of the default");
    assert_ne!(catalog_revision(&[original]), catalog_revision(&[marked]));
    let warnings = deprecation_warnings("demo", &json!({"deprecated": null}), &deprecations);
    assert_eq!(warnings.len(), 2);
    assert!(warnings[0]["parameter"].is_null());
    assert_eq!(warnings[0]["replacement"], "other");
    assert_eq!(warnings[1]["parameter"], "deprecated");
    assert_eq!(deprecation_warnings("demo", &json!({}), &deprecations).len(), 1);
    assert!(deprecation_warnings("other", &json!({}), &deprecations).is_empty());
}

fn described() -> Vec<Value> {
    crate::catalog().iter().map(crate::Command::to_json).collect()
}

/// A snapshot of `described` at API version `api`, with only the `commands` section.
fn snapshot_of(described: &[Value], api: u32) -> Value {
    serde_json::from_str(&format!("{{\"command_api_version\":{api},\"commands\":{}}}", snapshot_section(described))).unwrap()
}

/// A synthetic command with this parameter schema.
fn command(params: Value) -> Value {
    json!({"id": "schematic.synthetic", "title": "Synthetic", "summary": "A test command.", "params": params})
}

/// The labelled differences between two parameter schemas of the synthetic command.
fn differences(old: Value, new: Value) -> Vec<String> {
    compare("schematic.synthetic", &contract_of(&command(old)), &contract_of(&command(new))).iter().map(ToString::to_string).collect()
}

/// Guarded schema differences are breaking against a retained snapshot, including after a head
/// rewrite. Check exact diagnostics, both sections and the raised-version route through the real
/// comparator; this helper does not validate instances against JSON Schema.
fn assert_unevaluated_comparison(old: &Value, new: &Value) -> Vec<Change> {
    let old_command = command(old.clone());
    let new_command = command(new.clone());
    let changes = compare("schematic.synthetic", &contract_of(&old_command), &contract_of(&new_command));
    assert!(!changes.is_empty(), "{old} -> {new}: no diagnostic");
    assert!(changes.iter().all(|change| change.breaking), "{changes:?}");
    assert!(changes.iter().any(|change| change.what.ends_with("evaluation context is unknown)")), "{changes:?}");
    assert!(compare("schematic.synthetic", &contract_of(&old_command), &contract_of(&old_command)).is_empty());
    let expected: Vec<String> = changes.iter().map(ToString::to_string).collect();
    let mut expected_problems = expected.clone();
    expected_problems.push("raise COMMAND_API_VERSION above 1 if this break is intended, or keep the old contract".to_owned());
    for section in SNAPSHOT_SECTIONS {
        let mut base = json!({"command_api_version": 1, "commands": {}, "window_commands": {}});
        base[*section] = json!({"schematic.synthetic": contract_of(&old_command)});
        assert_eq!(check_snapshot(&base, section, std::slice::from_ref(&old_command), 1), Ok(()));
        let Err(SnapshotMismatch::Breaking(problems)) = check_snapshot(&base, section, std::slice::from_ref(&new_command), 1) else {
            panic!("the unchanged snapshot did not refuse the annotation-context change")
        };
        assert_eq!(problems, expected_problems);
        let mut head = base.clone();
        head[*section] = json!({"schematic.synthetic": contract_of(&new_command)});
        assert_eq!(check_snapshot(&head, section, std::slice::from_ref(&new_command), 1), Ok(()));
        assert_eq!(compare_snapshots(&base, &head).unwrap_err(), expected_problems);
        head["command_api_version"] = json!(2);
        assert_eq!(compare_snapshots(&base, &head).unwrap(), expected);
    }
    changes
}

/// Against an unchanged committed snapshot, a detected breaking difference requires a higher
/// `COMMAND_API_VERSION`; other detected differences request a refreshed snapshot. A same-commit
/// rewrite needs the separate comparison against a retained prior snapshot.
#[test]
fn every_command_matches_the_committed_snapshot() {
    let snapshot: Value = serde_json::from_str(SNAPSHOT).unwrap();
    match check_snapshot(&snapshot, "commands", &described(), COMMAND_API_VERSION) {
        Ok(()) => {}
        Err(SnapshotMismatch::Breaking(problems)) => panic!("{}", problems.join("\n")),
        Err(SnapshotMismatch::Stale { changes, section, api_version }) => panic!(
            "{}\nin docs/schemas/command-contracts.json set \"command_api_version\": {api_version} and the \"commands\" section to:\n  \"commands\": {section}",
            changes.join("\n")
        ),
    }
    assert_eq!(snapshot["command_api_version"], json!(COMMAND_API_VERSION));
    assert!(CAPABILITIES.contains(&"command_api"));
}

/// On the real catalog, a new optional parameter only refreshes the snapshot, while a new required
/// one fails until the API version rises; after the raise and the refresh the check passes.
#[test]
fn a_breaking_change_needs_a_raised_api_version() {
    let original = described();
    let snapshot = snapshot_of(&original, 1);
    assert_eq!(check_snapshot(&snapshot, "commands", &original, 1), Ok(()));

    let mut optional = original.clone();
    let undo = optional.iter_mut().find(|command| command["id"] == "edit.undo").unwrap();
    undo["params"]["properties"]["note"] = json!({"type": "string"});
    let Err(SnapshotMismatch::Stale { changes, section, api_version: 1 }) = check_snapshot(&snapshot, "commands", &optional, 1) else {
        panic!("a new optional parameter was not a plain refresh")
    };
    assert_eq!(changes, ["compatible: `edit.undo` params.properties.note: added"]);
    assert_eq!(check_snapshot(&snapshot_of(&optional, 1), "commands", &optional, 1), Ok(()));
    assert!(section.contains("\n    \"edit.undo\": {\n      \"params\": {\n"));

    let mut required = optional.clone();
    let undo = required.iter_mut().find(|command| command["id"] == "edit.undo").unwrap();
    undo["params"]["required"].as_array_mut().unwrap().push(json!("note"));
    let refreshed = snapshot_of(&optional, 1);
    let Err(SnapshotMismatch::Breaking(problems)) = check_snapshot(&refreshed, "commands", &required, 1) else {
        panic!("a new required parameter passed")
    };
    assert_eq!(
        problems,
        [
            "breaking: `edit.undo` params.required: now requires `note`",
            "raise COMMAND_API_VERSION above 1 if this break is intended, or keep the old contract"
        ]
    );
    let Err(SnapshotMismatch::Stale { changes, api_version: 2, .. }) = check_snapshot(&refreshed, "commands", &required, 2) else {
        panic!("a raised API version was not a refresh")
    };
    assert_eq!(changes, ["breaking: `edit.undo` params.required: now requires `note`", "the snapshot's command_api_version is 1, not 2"]);
    assert_eq!(check_snapshot(&snapshot_of(&required, 2), "commands", &required, 2), Ok(()));
}

/// A removed command is breaking; an added one only refreshes the snapshot.
#[test]
fn a_removed_command_is_breaking_and_an_added_one_is_not() {
    let mut commands = described();
    let snapshot = snapshot_of(&commands, 1);
    let removed = commands.pop().unwrap();
    let id = removed["id"].as_str().unwrap().to_owned();
    let Err(SnapshotMismatch::Breaking(problems)) = check_snapshot(&snapshot, "commands", &commands, 1) else { panic!("a removal passed") };
    assert_eq!(problems[0], format!("breaking: `{id}`: removed"));

    let mut added = removed.clone();
    added["id"] = json!("schematic.synthetic_example");
    let mut with_added = described();
    with_added.push(added);
    let Err(SnapshotMismatch::Stale { changes, .. }) = check_snapshot(&snapshot, "commands", &with_added, 1) else { panic!("an addition passed") };
    assert_eq!(changes, ["compatible: `schematic.synthetic_example`: added"]);
}

/// The API version never falls, and the snapshot's must be a whole number of at least 1 in any
/// spelling JSON allows; another spelling is refused, never read as 0 or 1.
#[test]
fn the_api_version_never_falls_and_is_a_whole_number() {
    let commands = described();
    let Err(SnapshotMismatch::Breaking(problems)) = check_snapshot(&snapshot_of(&commands, 3), "commands", &commands, 2) else {
        panic!("a falling API version passed")
    };
    assert_eq!(problems, ["COMMAND_API_VERSION is 2, below the snapshot's 3; it never falls"]);
    for spelling in ["1.0", "1e0", "\"1\"", "0", "-1", "18446744073709551616", "null"] {
        let text = format!("{{\"command_api_version\":{spelling},\"commands\":{}}}", snapshot_section(&commands));
        let snapshot: Value = serde_json::from_str(&text).unwrap();
        let Err(SnapshotMismatch::Breaking(problems)) = check_snapshot(&snapshot, "commands", &commands, 1) else {
            panic!("{spelling} was read as a version")
        };
        assert_eq!(problems.len(), 1, "{spelling}");
        assert!(
            problems[0].starts_with("the snapshot's command_api_version is ") && problems[0].ends_with("not a whole number of at least 1"),
            "{spelling}: {problems:?}"
        );
    }
    let missing: Value = serde_json::from_str(&format!("{{\"commands\":{}}}", snapshot_section(&commands))).unwrap();
    let Err(SnapshotMismatch::Breaking(problems)) = check_snapshot(&missing, "commands", &commands, 1) else { panic!("a missing version was read") };
    assert_eq!(problems, ["the snapshot's command_api_version is absent, not a whole number of at least 1"]);
}

/// Each change that accepts every call the old schema accepted is labelled compatible, at the
/// top of the parameters and nested inside a parameter.
#[test]
fn compatible_changes_are_labelled_compatible() {
    let p = |schema: Value| json!({"type": "object", "properties": {"p": schema}, "required": ["p"], "additionalProperties": false});
    let nested = |schema: Value| p(json!({"type": "object", "properties": {"q": schema}, "required": ["q"]}));
    for (old, new, expected) in [
        (
            p(json!({"type": "integer"})),
            json!({"type": "object", "properties": {"p": {"type": "integer"}, "r": {"type": "string"}}, "required": ["p"], "additionalProperties": false}),
            "params.properties.r: added",
        ),
        (
            p(json!({"type": "integer"})),
            json!({"type": "object", "properties": {"p": {"type": "integer"}}, "required": [], "additionalProperties": false}),
            "params.required: no longer requires `p`",
        ),
        (
            p(json!({"type": "string", "enum": ["a"]})),
            p(json!({"type": "string", "enum": ["a", "b"]})),
            "params.properties.p.enum: changed from [\"a\"] to [\"a\",\"b\"]",
        ),
        (p(json!({"type": "string", "enum": ["a"]})), p(json!({"type": "string"})), "params.properties.p.enum: changed from [\"a\"] to absent"),
        (
            p(json!({"type": "string"})),
            p(json!({"type": ["string", "null"]})),
            "params.properties.p.type: changed from \"string\" to [\"string\",\"null\"]",
        ),
        (p(json!({"type": "integer"})), p(json!({"type": "number"})), "params.properties.p.type: changed from \"integer\" to \"number\""),
        (p(json!({"type": "integer"})), p(json!({})), "params.properties.p.type: changed from \"integer\" to absent"),
        (p(json!({"minimum": 1})), p(json!({"minimum": 0})), "params.properties.p.minimum: changed from 1 to 0"),
        (p(json!({"maxLength": 8})), p(json!({"maxLength": 9})), "params.properties.p.maxLength: changed from 8 to 9"),
        (
            p(json!({"maximum": i64::MAX - 1})),
            p(json!({"maximum": i64::MAX})),
            "params.properties.p.maximum: changed from 9223372036854775806 to 9223372036854775807",
        ),
        (p(json!({"type": "boolean"})), p(json!(true)), "params.properties.p: changed from {\"type\":\"boolean\"} to true"),
        (p(json!({"$defs": {}})), p(json!({"$defs": {"d": {"type": "string"}}})), "params.properties.p.$defs.d: added"),
        (p(json!({"maxItems": 8})), p(json!({})), "params.properties.p.maxItems: changed from 8 to absent"),
        (p(json!({"pattern": "^a$"})), p(json!({})), "params.properties.p.pattern: changed from \"^a$\" to absent"),
        (p(json!({"multipleOf": 2})), p(json!({})), "params.properties.p.multipleOf: changed from 2 to absent"),
        (p(json!({"uniqueItems": true})), p(json!({"uniqueItems": false})), "params.properties.p.uniqueItems: changed from true to false"),
        (p(json!({"examples": [1]})), p(json!({"examples": [2]})), "params.properties.p.examples: changed from [1] to [2]"),
        (
            p(json!({"type": "object", "additionalProperties": false})),
            p(json!({"type": "object"})),
            "params.properties.p.additionalProperties: changed from false to absent",
        ),
        (p(json!({"items": {"maximum": 3}})), p(json!({"items": {"maximum": 4}})), "params.properties.p.items.maximum: changed from 3 to 4"),
        (
            p(json!({"anyOf": [{"type": "string"}]})),
            p(json!({"anyOf": [{"type": "string"}, {"type": "null"}]})),
            "params.properties.p.anyOf: 1 alternative(s) appended",
        ),
        (
            nested(json!({"type": "integer"})),
            p(json!({"type": "object", "properties": {"q": {"type": "integer"}}, "required": []})),
            "params.properties.p.required: no longer requires `q`",
        ),
        (nested(json!({"maximum": 3})), nested(json!({"maximum": 4})), "params.properties.p.properties.q.maximum: changed from 3 to 4"),
        (
            p(json!({"type": "object", "properties": {}, "additionalProperties": false})),
            p(json!({"type": "object", "properties": {"q": {"type": "integer", "default": 0}}, "additionalProperties": false})),
            "params.properties.p.properties.q: added",
        ),
    ] {
        assert_eq!(differences(old, new), [format!("compatible: `schematic.synthetic` {expected}")]);
    }
}

/// Each change that may refuse a call the old schema accepted, or change its meaning, is labelled
/// breaking with its path, at the top of the parameters, nested, and inside alternatives. A
/// property added to an object that accepted unknown names (open, a schema for them, or pattern
/// names) is breaking, since a call may already pass that name with another value.
#[test]
fn breaking_changes_are_labelled_breaking() {
    let p = |schema: Value| json!({"type": "object", "properties": {"p": schema}, "required": [], "additionalProperties": false});
    let nested = |schema: Value| p(json!({"type": "object", "properties": {"q": schema}, "required": []}));
    for (old, new, expected) in [
        (
            p(json!({"type": "integer"})),
            json!({"type": "object", "properties": {}, "required": [], "additionalProperties": false}),
            "params.properties.p: removed",
        ),
        (
            p(json!({"type": "integer"})),
            json!({"type": "object", "properties": {"p": {"type": "integer"}}, "required": ["p"], "additionalProperties": false}),
            "params.required: now requires `p`",
        ),
        (p(json!({"enum": ["a", "b"]})), p(json!({"enum": ["a"]})), "params.properties.p.enum: changed from [\"a\",\"b\"] to [\"a\"]"),
        (p(json!({})), p(json!({"enum": ["a"]})), "params.properties.p.enum: changed from absent to [\"a\"]"),
        (p(json!({"type": "number"})), p(json!({"type": "integer"})), "params.properties.p.type: changed from \"number\" to \"integer\""),
        (p(json!({})), p(json!({"type": "integer"})), "params.properties.p.type: changed from absent to \"integer\""),
        (p(json!({"minimum": 0})), p(json!({"minimum": 1})), "params.properties.p.minimum: changed from 0 to 1"),
        (p(json!({"maximum": 4})), p(json!({"maximum": 3})), "params.properties.p.maximum: changed from 4 to 3"),
        (
            p(json!({"maximum": i64::MAX})),
            p(json!({"maximum": i64::MAX - 1})),
            "params.properties.p.maximum: changed from 9223372036854775807 to 9223372036854775806",
        ),
        (
            p(json!({"type": "boolean", "default": true})),
            p(json!(true)),
            "params.properties.p: changed from {\"default\":true,\"type\":\"boolean\"} to true",
        ),
        (
            p(json!({"items": {"type": "string", "default": "a"}})),
            p(json!({})),
            "params.properties.p.items: changed from {\"default\":\"a\",\"type\":\"string\"} to absent",
        ),
        (
            p(json!({"$defs": {"d": {"minimum": 1}}, "not": {"$ref": "#/$defs/d"}})),
            p(json!({"$defs": {"d": {"minimum": 0}}, "not": {"$ref": "#/$defs/d"}})),
            "params.properties.p.$defs.d: changed from {\"minimum\":1} to {\"minimum\":0}",
        ),
        (p(json!({"$defs": {"d": {}}})), p(json!({"$defs": {}})), "params.properties.p.$defs.d: removed"),
        (p(json!({})), p(json!({"maxItems": 3})), "params.properties.p.maxItems: changed from absent to 3"),
        (p(json!({})), p(json!({"pattern": "^a$"})), "params.properties.p.pattern: changed from absent to \"^a$\""),
        (p(json!({"format": "uuid"})), p(json!({"format": "date"})), "params.properties.p.format: changed from \"uuid\" to \"date\""),
        (p(json!({"const": 1})), p(json!({"const": 2})), "params.properties.p.const: changed from 1 to 2"),
        (p(json!({"multipleOf": 2})), p(json!({"multipleOf": 4})), "params.properties.p.multipleOf: changed from 2 to 4"),
        (p(json!({"default": 1})), p(json!({"default": 2})), "params.properties.p.default: changed from 1 to 2"),
        (p(json!({})), p(json!({"default": 1})), "params.properties.p.default: changed from absent to 1"),
        (p(json!({"default": 1})), p(json!({})), "params.properties.p.default: changed from 1 to absent"),
        (
            p(json!({"type": "object", "properties": {}})),
            p(json!({"type": "object", "properties": {"q": {"type": "integer"}}})),
            "params.properties.p.properties.q: added where the old schema accepted any name",
        ),
        (
            p(json!({"type": "object", "properties": {}, "additionalProperties": {"type": "string"}})),
            p(json!({"type": "object", "properties": {"q": {"type": "integer"}}, "additionalProperties": {"type": "string"}})),
            "params.properties.p.properties.q: added where the old schema accepted any name",
        ),
        (
            p(json!({"type": "object", "properties": {}, "patternProperties": {"^x": {}}, "additionalProperties": false})),
            p(json!({"type": "object", "properties": {"q": {"type": "integer"}}, "patternProperties": {"^x": {}}, "additionalProperties": false})),
            "params.properties.p.properties.q: added where the old schema accepted any name",
        ),
        (p(json!({})), p(json!({"uniqueItems": true})), "params.properties.p.uniqueItems: changed from absent to true"),
        (
            p(json!({"type": "object"})),
            p(json!({"type": "object", "additionalProperties": false})),
            "params.properties.p.additionalProperties: changed from absent to false",
        ),
        (p(json!({"items": {"maximum": 4}})), p(json!({"items": {"maximum": 3}})), "params.properties.p.items.maximum: changed from 4 to 3"),
        (
            p(json!({"anyOf": [{"type": "string"}, {"type": "null"}]})),
            p(json!({"anyOf": [{"type": "string"}]})),
            "params.properties.p.anyOf: 1 alternative(s) removed",
        ),
        (
            p(json!({"oneOf": [{"type": "string"}]})),
            p(json!({"oneOf": [{"type": "string"}, {"type": "null"}]})),
            "params.properties.p.oneOf: 1 alternative(s) appended",
        ),
        (
            p(json!({"oneOf": [{"type": "string", "maxLength": 3}, {"type": "integer"}]})),
            p(json!({"oneOf": [{"type": "string", "maxLength": 4}, {"type": "integer"}]})),
            "params.properties.p.oneOf[0].maxLength: changed from 3 to 4 (inside oneOf, a value may now match two alternatives)",
        ),
        (
            p(json!({"prefixItems": [{"type": "string"}]})),
            p(json!({"prefixItems": [{"type": "string"}, {"type": "string"}]})),
            "params.properties.p.prefixItems: 1 alternative(s) appended",
        ),
        (p(json!({"$ref": "#/$defs/a"})), p(json!({"$ref": "#/$defs/b"})), "params.properties.p.$ref: changed from \"#/$defs/a\" to \"#/$defs/b\""),
        (p(json!({})), p(json!({"allOf": [{"type": "string"}]})), "params.properties.p.allOf: changed from absent to [{\"type\":\"string\"}]"),
        (p(json!({})), p(json!({"not": {"type": "null"}})), "params.properties.p.not: changed from absent to {\"type\":\"null\"}"),
        (
            p(json!({"not": {"type": "null"}})),
            p(json!({"not": {"type": "string"}})),
            "params.properties.p.not: changed from {\"type\":\"null\"} to {\"type\":\"string\"}",
        ),
        (p(json!({"not": {"type": "null"}})), p(json!({})), "params.properties.p.not: changed from {\"type\":\"null\"} to absent"),
        (
            p(json!({"type": "object", "dependentRequired": {"a": ["b"]}})),
            p(json!({"type": "object", "dependentRequired": {"a": ["b", "c"]}})),
            "params.properties.p.dependentRequired: changed from {\"a\":[\"b\"]} to {\"a\":[\"b\",\"c\"]}",
        ),
        (
            p(json!({"type": "object", "dependentRequired": {"a": ["b"]}})),
            p(json!({"type": "object"})),
            "params.properties.p.dependentRequired: changed from {\"a\":[\"b\"]} to absent",
        ),
        (
            nested(json!({"type": "integer"})),
            p(json!({"type": "object", "properties": {}, "required": []})),
            "params.properties.p.properties.q: removed",
        ),
        (
            nested(json!({"type": "integer"})),
            p(json!({"type": "object", "properties": {"q": {"type": "integer"}}, "required": ["q"]})),
            "params.properties.p.required: now requires `q`",
        ),
        (
            p(json!({"anyOf": [{"type": "object", "properties": {"q": {}}, "required": []}]})),
            p(json!({"anyOf": [{"type": "object", "properties": {"q": {}}, "required": ["q"]}]})),
            "params.properties.p.anyOf[0].required: now requires `q`",
        ),
    ] {
        assert_eq!(differences(old, new), [format!("breaking: `schematic.synthetic` {expected}")]);
    }
    let scoped = |scope: &str| json!({"scope": scope, "params": {"type": "object"}});
    let changes = compare("window.synthetic", &scoped("window"), &scoped("pane"));
    assert_eq!(changes, [Change { breaking: true, what: "`window.synthetic` scope: changed from \"window\" to \"pane\"".to_owned() }]);
}

/// Every row of the rule table in `compare`'s documentation (and docs/automation/command-api.md), tried at the
/// parameter root and the nested positions listed below. Local labels hold in positive positions;
/// `oneOf`, negation, conditionals and `allOf` make changes breaking, as does the conservative
/// whole-parameter guard for references or unevaluated keywords.
#[test]
fn every_rule_holds_in_the_listed_schema_positions() {
    let open = |properties: Value| json!({"type": "object", "properties": properties});
    let closed = |properties: Value| json!({"type": "object", "properties": properties, "additionalProperties": false});
    let mut rows: Vec<_> = vec![
        ("optional parameter added to a closed object", closed(json!({})), closed(json!({"q": {"type": "string", "default": "a"}})), false),
        ("required parameter made optional", json!({"type": "object", "required": ["q"]}), json!({"type": "object", "required": []}), false),
        ("enum value added", json!({"enum": ["a"]}), json!({"enum": ["a", "b"]}), false),
        ("enum dropped", json!({"enum": ["a"]}), json!({}), false),
        ("type widened", json!({"type": "string"}), json!({"type": ["string", "null"]}), false),
        ("integer to number", json!({"type": "integer"}), json!({"type": "number"}), false),
        ("type dropped", json!({"type": "integer"}), json!({}), false),
        ("lower bound loosened", json!({"minimum": 1}), json!({"minimum": 0}), false),
        ("upper bound loosened at the end of i64", json!({"maximum": i64::MAX - 1}), json!({"maximum": i64::MAX}), false),
        ("upper bound loosened at the end of u64", json!({"maximum": u64::MAX - 1}), json!({"maximum": u64::MAX}), false),
        ("lower bound loosened at the end of i64", json!({"minimum": i64::MIN + 1}), json!({"minimum": i64::MIN}), false),
        ("bound dropped", json!({"maxItems": 3}), json!({}), false),
        ("pattern dropped", json!({"pattern": "^a$"}), json!({}), false),
        ("format dropped", json!({"format": "uuid"}), json!({}), false),
        ("const dropped", json!({"const": 1}), json!({}), false),
        ("multipleOf dropped", json!({"multipleOf": 2}), json!({}), false),
        ("uniqueItems dropped", json!({"uniqueItems": true}), json!({}), false),
        ("uniqueItems set false", json!({"uniqueItems": true}), json!({"uniqueItems": false}), false),
        ("uniqueItems false added", json!({}), json!({"uniqueItems": false}), false),
        ("uniqueItems false dropped", json!({"uniqueItems": false}), json!({}), false),
        ("uniqueItems false made true", json!({"uniqueItems": false}), json!({"uniqueItems": true}), true),
        ("additionalProperties loosened", closed(json!({})), open(json!({})), false),
        ("items loosened", json!({"items": {"maximum": 3}}), json!({"items": {"maximum": 4}}), false),
        ("anyOf alternative appended", json!({"anyOf": [{"type": "string"}]}), json!({"anyOf": [{"type": "string"}, {"type": "null"}]}), false),
        ("prefixItems child loosened", json!({"prefixItems": [{"maximum": 3}]}), json!({"prefixItems": [{"maximum": 4}]}), false),
        ("prefixItems child tightened", json!({"prefixItems": [{"maximum": 4}]}), json!({"prefixItems": [{"maximum": 3}]}), true),
        ("prefixItems child appended", json!({"prefixItems": [{}]}), json!({"prefixItems": [{}, {}]}), true),
        ("prefixItems child removed", json!({"prefixItems": [{}, {}]}), json!({"prefixItems": [{}]}), true),
        ("prefixItems keyword added", json!({}), json!({"prefixItems": [{}]}), true),
        ("prefixItems keyword dropped", json!({"prefixItems": [{}]}), json!({}), true),
        ("definition added", json!({"$defs": {}}), json!({"$defs": {"d": {}}}), false),
        ("examples changed", json!({"examples": [1]}), json!({"examples": [2]}), false),
        ("schema without a default made to accept anything", json!({"type": "boolean"}), json!(true), false),
        ("false made to accept something", json!(false), json!({"type": "string"}), false),
        ("parameter removed", closed(json!({"q": {}})), closed(json!({})), true),
        ("parameter newly required", json!({"type": "object", "required": []}), json!({"type": "object", "required": ["q"]}), true),
        ("enum value removed", json!({"enum": ["a", "b"]}), json!({"enum": ["a"]}), true),
        ("enum added", json!({}), json!({"enum": ["a"]}), true),
        ("type narrowed", json!({"type": "number"}), json!({"type": "integer"}), true),
        ("type added", json!({}), json!({"type": "integer"}), true),
        ("lower bound tightened", json!({"minimum": 0}), json!({"minimum": 1}), true),
        ("upper bound tightened at the end of i64", json!({"maximum": i64::MAX}), json!({"maximum": i64::MAX - 1}), true),
        ("upper bound tightened at the end of u64", json!({"maximum": u64::MAX}), json!({"maximum": u64::MAX - 1}), true),
        ("lower bound tightened at the end of i64", json!({"minimum": i64::MIN}), json!({"minimum": i64::MIN + 1}), true),
        (
            "integer bound tightened to a fraction just below it",
            json!({"maximum": 9_007_199_254_740_993_u64}),
            json!({"maximum": 9_007_199_254_740_992.0}),
            true,
        ),
        ("fraction bound tightened to an integer just inside it", json!({"minimum": 0.5}), json!({"minimum": 1}), true),
        ("bound added", json!({}), json!({"maxItems": 3}), true),
        ("pattern added", json!({}), json!({"pattern": "^a$"}), true),
        ("format changed", json!({"format": "uuid"}), json!({"format": "date"}), true),
        ("const changed", json!({"const": 1}), json!({"const": 2}), true),
        ("multipleOf changed", json!({"multipleOf": 2}), json!({"multipleOf": 4}), true),
        ("uniqueItems added", json!({}), json!({"uniqueItems": true}), true),
        ("default added", json!({}), json!({"default": 1}), true),
        ("default changed", json!({"default": 1}), json!({"default": 2}), true),
        ("default dropped", json!({"default": 1}), json!({}), true),
        ("default dropped by accepting anything", json!({"type": "boolean", "default": true}), json!(true), true),
        ("nested default dropped by accepting anything", closed(json!({"q": {"default": 1}})), json!(true), true),
        ("definition removed", json!({"$defs": {"d": {}}}), json!({"$defs": {}}), true),
        ("definition loosened", json!({"$defs": {"d": {"minimum": 1}}}), json!({"$defs": {"d": {"minimum": 0}}}), true),
        ("property added to an open object", open(json!({})), open(json!({"q": {}})), true),
        ("additionalProperties tightened", open(json!({})), closed(json!({})), true),
        ("anyOf alternative removed", json!({"anyOf": [{"type": "string"}, {"type": "null"}]}), json!({"anyOf": [{"type": "string"}]}), true),
        ("oneOf alternative appended", json!({"oneOf": [{"type": "string"}]}), json!({"oneOf": [{"type": "string"}, {"type": "null"}]}), true),
        ("oneOf alternative loosened", json!({"oneOf": [{"maxLength": 3}]}), json!({"oneOf": [{"maxLength": 4}]}), true),
        ("not changed", json!({"not": {"type": "null"}}), json!({"not": {"type": "string"}}), true),
        ("dependentRequired changed", json!({"dependentRequired": {"a": ["b"]}}), json!({"dependentRequired": {"a": ["b", "c"]}}), true),
        ("unknown keyword added", json!({}), json!({"contains": {"type": "string"}}), true),
        ("schema made to refuse everything", json!({"type": "string"}), json!(false), true),
    ]
    .into_iter()
    .map(|(name, old, new, breaking)| (name.to_owned(), old, new, breaking))
    .collect();
    for keyword in ["items", "additionalProperties"] {
        for (route, old, new, breaking) in [
            ("false to schema", Some(json!(false)), Some(json!({"maximum": 3})), false),
            ("schema to true", Some(json!({"maximum": 3})), Some(json!(true)), false),
            ("schema to absent", Some(json!({"maximum": 3})), None, false),
            ("true to schema", Some(json!(true)), Some(json!({"maximum": 3})), true),
            ("absent to schema", None, Some(json!({"maximum": 3})), true),
            ("false to true", Some(json!(false)), Some(json!(true)), false),
            ("true to false", Some(json!(true)), Some(json!(false)), true),
            ("false to absent", Some(json!(false)), None, false),
            ("absent to false", None, Some(json!(false)), true),
            ("schema to false", Some(json!({"maximum": 3})), Some(json!(false)), true),
            ("true to absent", Some(json!(true)), None, false),
            ("absent to true", None, Some(json!(true)), false),
            ("default to true", Some(json!({"default": 1})), Some(json!(true)), true),
            ("default to absent", Some(json!({"default": 1})), None, true),
            ("nested default to true", Some(json!({"properties": {"q": {"default": 1}}})), Some(json!(true)), true),
        ] {
            let wrap = |value: Option<Value>| value.map_or_else(|| json!({}), |inner| json!({keyword: inner}));
            rows.push((format!("{keyword} {route}"), wrap(old), wrap(new), breaking));
        }
    }
    for keyword in ["anyOf", "oneOf"] {
        for (route, old, new, breaking) in [
            ("child loosened", json!({keyword: [{"maximum": 3}]}), json!({keyword: [{"maximum": 4}]}), keyword == "oneOf"),
            ("child tightened", json!({keyword: [{"maximum": 4}]}), json!({keyword: [{"maximum": 3}]}), true),
            ("child appended", json!({keyword: [{"type": "string"}]}), json!({keyword: [{"type": "string"}, {"type": "null"}]}), keyword == "oneOf"),
            ("child removed", json!({keyword: [{"type": "string"}, {"type": "null"}]}), json!({keyword: [{"type": "string"}]}), true),
            ("keyword added", json!({}), json!({keyword: [{}]}), true),
            ("keyword dropped", json!({keyword: [{}]}), json!({}), true),
        ] {
            rows.push((format!("{keyword} {route}"), old, new, breaking));
        }
    }
    for keyword in ["$defs", "definitions"] {
        for (route, old, new, breaking) in [
            ("entry added", json!({keyword: {}}), json!({keyword: {"d": {}}}), false),
            ("entry removed", json!({keyword: {"d": {}}}), json!({keyword: {}}), true),
            ("entry changed", json!({keyword: {"d": {"maximum": 3}}}), json!({keyword: {"d": {"maximum": 4}}}), true),
        ] {
            rows.push((format!("{keyword} {route}"), old, new, breaking));
        }
    }
    for &keyword in LOWER_BOUNDS.iter().chain(UPPER_BOUNDS.iter()) {
        let (narrow, wide) =
            if LOWER_BOUNDS.contains(&keyword) { (json!({keyword: 2}), json!({keyword: 1})) } else { (json!({keyword: 1}), json!({keyword: 2})) };
        rows.push((format!("{keyword} loosened"), narrow.clone(), wide.clone(), false));
        rows.push((format!("{keyword} tightened"), wide.clone(), narrow.clone(), true));
        rows.push((format!("{keyword} added"), json!({}), narrow.clone(), true));
        rows.push((format!("{keyword} dropped"), narrow, json!({}), false));
    }
    for (keyword, before, after) in [
        ("pattern", json!("^a$"), json!("^b$")),
        ("format", json!("uuid"), json!("date")),
        ("const", json!(1), json!(2)),
        ("multipleOf", json!(2), json!(4)),
    ] {
        rows.push((format!("{keyword} added"), json!({}), json!({keyword: before.clone()}), true));
        rows.push((format!("{keyword} changed"), json!({keyword: before.clone()}), json!({keyword: after}), true));
        rows.push((format!("{keyword} dropped"), json!({keyword: before}), json!({}), false));
    }
    for (keyword, before, after) in [
        ("not", json!({"type": "string"}), json!({"type": "null"})),
        ("allOf", json!([{ "type": "string" }]), json!([{ "type": "null" }])),
        ("$ref", json!("#/$defs/a"), json!("#/$defs/b")),
        ("if", json!({"type": "string"}), json!({"type": "null"})),
        ("then", json!({"type": "string"}), json!({"type": "null"})),
        ("else", json!({"type": "string"}), json!({"type": "null"})),
        ("patternProperties", json!({"^q$": {"type": "string"}}), json!({"^q$": {"type": "null"}})),
        ("dependentRequired", json!({"a": ["b"]}), json!({"a": ["c"]})),
        ("dependentSchemas", json!({"a": {"required": ["b"]}}), json!({"a": {"required": ["c"]}})),
        ("dependencies", json!({"a": ["b"]}), json!({"a": ["c"]})),
    ] {
        rows.push((format!("{keyword} added"), json!({}), json!({keyword: before.clone()}), true));
        rows.push((format!("{keyword} changed"), json!({keyword: before.clone()}), json!({keyword: after}), true));
        rows.push((format!("{keyword} dropped"), json!({keyword: before}), json!({}), true));
    }
    rows.push(("unknown keyword changed".to_owned(), json!({"contains": {"type": "string"}}), json!({"contains": {"type": "null"}}), true));
    rows.push(("unknown keyword dropped".to_owned(), json!({"contains": {"type": "string"}}), json!({}), true));
    rows.push(("examples added".to_owned(), json!({}), json!({"examples": [1]}), false));
    rows.push(("examples dropped".to_owned(), json!({"examples": [1]}), json!({}), false));
    // A place: its name, how it wraps a schema, and whether any change there is breaking.
    type Place = (&'static str, fn(Value) -> Value, bool);
    let places: Vec<Place> = vec![
        ("as the parameter schema", |s| s, false),
        ("inside prefixItems", |s| json!({"type": "array", "prefixItems": [s]}), false),
        ("as a parameter", |s| json!({"type": "object", "properties": {"p": s}, "additionalProperties": false}), false),
        ("inside items", |s| json!({"type": "object", "properties": {"p": {"type": "array", "items": s}}, "additionalProperties": false}), false),
        (
            "inside additionalProperties",
            |s| json!({"type": "object", "properties": {"p": {"type": "object", "additionalProperties": s}}, "additionalProperties": false}),
            false,
        ),
        ("as an anyOf alternative", |s| json!({"type": "object", "properties": {"p": {"anyOf": [s]}}, "additionalProperties": false}), false),
        ("as a oneOf alternative", |s| json!({"type": "object", "properties": {"p": {"oneOf": [s]}}, "additionalProperties": false}), true),
        ("with unevaluated properties", |s| json!({"anyOf": [s], "unevaluatedProperties": false}), true),
        ("with unevaluated items", |s| json!({"anyOf": [s], "unevaluatedItems": false}), true),
        ("with nested unevaluated properties", |s| json!({"properties": {"p": {"anyOf": [s], "unevaluatedProperties": false}}}), true),
        ("with nested unevaluated items", |s| json!({"items": {"anyOf": [s], "unevaluatedItems": false}}), true),
        ("under not", |s| json!({"type": "object", "properties": {"p": {"not": s}}, "additionalProperties": false}), true),
        ("under if", |s| json!({"type": "object", "properties": {"p": {"if": s, "then": {}}}, "additionalProperties": false}), true),
        ("under then", |s| json!({"type": "object", "properties": {"p": {"if": {}, "then": s}}, "additionalProperties": false}), true),
        ("under else", |s| json!({"type": "object", "properties": {"p": {"if": {}, "else": s}}, "additionalProperties": false}), true),
        ("under allOf", |s| json!({"type": "object", "properties": {"p": {"allOf": [s]}}, "additionalProperties": false}), true),
        (
            "in a $defs entry reached by $ref",
            |s| json!({"type": "object", "$defs": {"d": s}, "properties": {"p": {"$ref": "#/$defs/d"}}, "additionalProperties": false}),
            true,
        ),
        (
            "in a $defs entry reached by $ref under not",
            |s| json!({"type": "object", "$defs": {"d": s}, "properties": {"p": {"not": {"$ref": "#/$defs/d"}}}, "additionalProperties": false}),
            true,
        ),
        (
            "in a property reached by $ref under not",
            |s| json!({"type": "object", "properties": {"p": s, "q": {"not": {"$ref": "#/properties/p"}}}}),
            true,
        ),
        (
            "in items reached by $ref under not",
            |s| json!({"type": "object", "properties": {"p": {"type": "array", "items": s}, "q": {"not": {"$ref": "#/properties/p/items"}}}}),
            true,
        ),
        (
            "in prefixItems reached by $ref under not",
            |s| json!({"type": "object", "properties": {"p": {"type": "array", "prefixItems": [s]}, "q": {"not": {"$ref": "#/properties/p/prefixItems/0"}}}}),
            true,
        ),
        (
            "in an alternative reached by $ref under not",
            |s| json!({"type": "object", "properties": {"p": {"anyOf": [s]}, "q": {"not": {"$ref": "#/properties/p/anyOf/0"}}}}),
            true,
        ),
    ];
    for (row, old, new, breaking) in &rows {
        for (place, wrap, always_breaking) in &places {
            let changes = compare("schematic.synthetic", &json!({"params": wrap(old.clone())}), &json!({"params": wrap(new.clone())}));
            assert!(!changes.is_empty(), "{row} {place}: no change found");
            let expected = *breaking || *always_breaking;
            assert!(changes.iter().any(|change| change.breaking == expected), "{row} {place}: {changes:?}");
            assert!(changes.iter().all(|change| change.breaking == expected), "{row} {place}: {changes:?}");
        }
    }
}

/// A locally looser alternative can lose evaluated-property or evaluated-item annotations. The
/// unchanged unevaluated restriction makes these changes breaking, also in nested schemas and
/// against a retained base after either head snapshot section is rewritten.
#[test]
fn unevaluated_contexts_make_annotation_changes_breaking() {
    for (old, new) in [
        (
            json!({"type": "object", "anyOf": [{"properties": {"p": true}}], "unevaluatedProperties": false}),
            json!({"type": "object", "anyOf": [true], "unevaluatedProperties": false}),
        ),
        (
            json!({"type": "array", "anyOf": [{"prefixItems": [true]}], "unevaluatedItems": false}),
            json!({"type": "array", "anyOf": [true], "unevaluatedItems": false}),
        ),
    ] {
        for container in [None, Some("items"), Some("additionalProperties"), Some("properties")] {
            let wrap = |schema: &Value| match container {
                None => schema.clone(),
                Some("properties") => json!({"properties": {"p": schema}}),
                Some(key) => json!({key: schema}),
            };
            let changes = assert_unevaluated_comparison(&wrap(&old), &wrap(&new));
            assert_eq!(changes.len(), 1, "{container:?}: {changes:?}");
            assert!(changes[0].what.contains(".anyOf[0]: changed from"), "{changes:?}");
        }
    }
}

/// The conservative unevaluated guard inspects either endpoint, keeps unequal empty-map changes
/// visible and includes nested/data occurrences rather than promising annotation-aware precision.
#[test]
fn unevaluated_contexts_keep_empty_maps_and_local_relaxations_breaking() {
    for keyword in ["unevaluatedProperties", "unevaluatedItems"] {
        for (old, new) in [
            (json!({keyword: false, "required": ["p"]}), json!({"required": []})),
            (json!({"required": ["p"]}), json!({keyword: false, "required": []})),
        ] {
            let changes = assert_unevaluated_comparison(&old, &new);
            assert_eq!(changes.len(), 2, "{keyword}: {changes:?}");
            assert!(changes.iter().any(|change| change.what.contains("params.required: no longer requires `p`")), "{changes:?}");
        }
        for empty_map in ["properties", "$defs", "definitions"] {
            for container in [None, Some("items"), Some("additionalProperties")] {
                let wrap = |schema: Value| match container {
                    None => schema,
                    Some(key) => json!({key: schema}),
                };
                let with = wrap(json!({keyword: false, empty_map: {}}));
                let without = wrap(json!({keyword: false}));
                for (old, new) in [(&with, &without), (&without, &with)] {
                    let changes = assert_unevaluated_comparison(old, new);
                    assert_eq!(changes.len(), 1, "{keyword} {empty_map} {container:?}: {changes:?}");
                    assert_eq!(
                        changes[0].what,
                        format!(
                            "`schematic.synthetic` params: changed from {old} to {new} (schema contains unevaluatedProperties or unevaluatedItems; evaluation context is unknown)"
                        )
                    );
                }
            }
        }
        let old = json!({"type": "string", "examples": [{keyword: false}]});
        let new = json!({"type": ["string", "null"], "examples": [{keyword: false}]});
        let changes = assert_unevaluated_comparison(&old, &new);
        assert_eq!(changes.len(), 1, "{keyword}: {changes:?}");
        assert!(changes[0].what.contains("params.type:"), "{changes:?}");
    }
}

/// An unchanged reference can target a schema outside definitions, under negation elsewhere.
/// Both a loosened string bound and a loosened integer bound must therefore fail base comparison
/// at the same API version, in either section, even after the head snapshot is rewritten.
#[test]
fn unchanged_references_make_target_changes_breaking() {
    // These changes reject previously valid {p: "a", q: "abcd"} and {p: 1, q: 0} respectively.
    for (old_target, new_target, path) in [
        (json!({"type": "string", "maxLength": 3}), json!({"type": "string", "maxLength": 4}), "maxLength"),
        (json!({"type": "integer", "minimum": 1}), json!({"type": "integer", "minimum": 0}), "minimum"),
    ] {
        for reference in ["$ref", "$dynamicRef", "$recursiveRef"] {
            let params = |target: Value| json!({"type": "object", "properties": {"p": target, "q": {"not": {reference: "#/properties/p"}}}});
            let old = params(old_target.clone());
            let new = params(new_target.clone());
            let changes = compare("schematic.synthetic", &contract_of(&command(old.clone())), &contract_of(&command(new.clone())));
            assert_eq!(changes.len(), 1, "{reference}: {changes:?}");
            assert!(changes[0].breaking, "{reference}: {changes:?}");
            assert!(changes[0].what.contains(&format!("params.properties.p.{path}:")), "{reference}: {changes:?}");
            assert!(changes[0].what.ends_with("schema contains a reference; target context is unknown)"), "{reference}: {changes:?}");
            for section in SNAPSHOT_SECTIONS {
                let mut base = json!({"command_api_version": 1, "commands": {}, "window_commands": {}});
                base[*section] = json!({"schematic.synthetic": contract_of(&command(old.clone()))});
                let mut head = base.clone();
                head[*section] = json!({"schematic.synthetic": contract_of(&command(new.clone()))});
                assert_eq!(check_snapshot(&head, section, &[command(new.clone())], 1), Ok(()));
                let problems = compare_snapshots(&base, &head).unwrap_err();
                assert_eq!(problems[0], changes[0].to_string());
                assert!(problems[1].contains("raise COMMAND_API_VERSION above 1"), "{problems:?}");
                head["command_api_version"] = json!(2);
                assert_eq!(compare_snapshots(&base, &head), Ok(vec![changes[0].to_string()]));
            }
        }
    }
}

/// Empty maps can themselves be reference targets. Adding or removing their keyword produces
/// no entry difference, but the unequal referenced schema must still be breaking, also nested
/// through schema-valued keywords and after rewriting either snapshot section.
#[test]
fn empty_reference_targets_cannot_disappear_without_a_diagnostic() {
    for keyword in ["properties", "$defs", "definitions"] {
        for container in [None, Some("items"), Some("additionalProperties")] {
            let inner = json!({keyword: {}});
            let (mut with, mut without, pointer) = match container {
                Some(container) => (json!({container: inner}), json!({container: {}}), format!("#/{container}/{keyword}")),
                None => (inner, json!({}), format!("#/{keyword}")),
            };
            with["$ref"] = json!(pointer);
            without["$ref"] = json!(pointer);
            for (old, new) in [(&with, &without), (&without, &with)] {
                let changes = differences(old.clone(), new.clone());
                let expected = format!(
                    "breaking: `schematic.synthetic` params: changed from {old} to {new} (schema contains a reference; target context is unknown)"
                );
                assert_eq!(changes.as_slice(), std::slice::from_ref(&expected), "{keyword} {container:?}");
                for section in SNAPSHOT_SECTIONS {
                    let mut base = json!({"command_api_version": 1, "commands": {}, "window_commands": {}});
                    base[*section] = json!({"schematic.synthetic": contract_of(&command(old.clone()))});
                    let mut head = base.clone();
                    head[*section] = json!({"schematic.synthetic": contract_of(&command(new.clone()))});
                    assert_eq!(check_snapshot(&head, section, &[command(new.clone())], 1), Ok(()));
                    let problems = compare_snapshots(&base, &head).unwrap_err();
                    assert_eq!(problems[0], expected);
                    assert!(problems[1].contains("raise COMMAND_API_VERSION above 1"), "{problems:?}");
                    head["command_api_version"] = json!(2);
                    assert_eq!(compare_snapshots(&base, &head), Ok(vec![expected.clone()]));
                }
            }
        }
    }
}

/// Every keyword the real catalog uses has its own rule in `compare`; any other keyword falls to
/// "unknown, so breaking". This keeps the list in `compare`'s documentation true of the catalog.
#[test]
fn every_keyword_of_the_catalog_has_a_rule() {
    /// Collect the schema keywords used at every depth of `value` (a schema).
    fn keywords(value: &Value, found: &mut BTreeSet<String>) {
        let Some(object) = value.as_object() else { return };
        for (key, inner) in object {
            found.insert(key.clone());
            match key.as_str() {
                "properties" | "$defs" | "definitions" | "patternProperties" => {
                    inner.as_object().into_iter().flatten().for_each(|(_, schema)| keywords(schema, found))
                }
                "items" | "additionalProperties" => keywords(inner, found),
                "anyOf" | "oneOf" | "allOf" | "prefixItems" => inner.as_array().into_iter().flatten().for_each(|schema| keywords(schema, found)),
                _ => {}
            }
        }
    }
    let mut found = BTreeSet::new();
    for command in described() {
        keywords(&contract_of(&command)["params"], &mut found);
    }
    let ruled: BTreeSet<&str> = [
        "properties",
        "$defs",
        "definitions",
        "required",
        "type",
        "enum",
        "uniqueItems",
        "default",
        "examples",
        "items",
        "additionalProperties",
        "anyOf",
        "oneOf",
        "prefixItems",
    ]
    .into_iter()
    .chain(WHOLE.iter().copied())
    .chain(LOWER_BOUNDS.iter().copied())
    .chain(UPPER_BOUNDS.iter().copied())
    .chain(RESTRICTIONS.iter().copied())
    .collect();
    let unruled: Vec<&String> = found.iter().filter(|key| !ruled.contains(key.as_str())).collect();
    assert!(unruled.is_empty(), "keywords without a rule in compare (now breaking on any change): {unruled:?}");
}

/// The catalog revision is the same for the same commands in any order and key order, and changes
/// when a title is reworded, a parameter changes or a command is added or removed.
#[test]
fn the_catalog_revision_follows_every_visible_change() {
    let commands = described();
    let revision = catalog_revision(&commands);
    assert_eq!(revision.len(), 64);
    assert_eq!(catalog_revision(&commands), revision);
    let mut reversed: Vec<Value> = commands.iter().rev().map(|command| serde_json::from_str(&command.to_string()).unwrap()).collect();
    assert_eq!(catalog_revision(&reversed), revision);

    reversed[0]["title"] = json!("Reworded");
    assert_ne!(catalog_revision(&reversed), revision);
    let mut changed = commands.clone();
    changed[0]["params"]["properties"]["note"] = json!({"type": "string"});
    assert_ne!(catalog_revision(&changed), revision);
    assert_ne!(catalog_revision(&commands[1..]), revision);
    let mut added = commands.clone();
    added.push(command(json!({"type": "object"})));
    assert_ne!(catalog_revision(&added), revision);
}

/// Reworded prose keeps the contract; a parameter that happens to be called `title`, and a data
/// value with such a key, are part of it.
#[test]
fn prose_is_not_part_of_the_contract() {
    let original = crate::command(crate::SET_SYMBOL_FIELDS).unwrap().to_json();
    let mut reworded = original.clone();
    reworded["title"] = json!("Edit fields");
    reworded["summary"] = json!("Reworded summary.");
    for schema in reworded["params"]["properties"].as_object_mut().unwrap().values_mut() {
        schema["description"] = json!("Reworded description.");
    }
    assert_eq!(contract_of(&reworded), contract_of(&original));
    assert!(contract_of(&original).get("id").is_none());

    let mut named = original.clone();
    named["params"]["properties"]["title"] = json!({"type": "string"});
    assert_ne!(contract_of(&named), contract_of(&original));
    let mut retyped = named.clone();
    retyped["params"]["properties"]["title"] = json!({"type": "integer"});
    assert_ne!(contract_of(&retyped), contract_of(&named));

    for keyword in ["default", "const"] {
        let mut first = named.clone();
        first["params"]["properties"]["title"] = json!({"type": "object", keyword: {"title": "a"}});
        let mut second = first.clone();
        second["params"]["properties"]["title"][keyword] = json!({"title": "b"});
        assert_ne!(contract_of(&second), contract_of(&first), "a changed {keyword} value kept the contract");
    }
    for keyword in ["enum", "examples"] {
        let mut first = named.clone();
        first["params"]["properties"]["title"] = json!({"type": "object", keyword: [{"description": "a"}]});
        let mut second = first.clone();
        second["params"]["properties"]["title"][keyword] = json!([{"description": "b"}]);
        assert_ne!(contract_of(&second), contract_of(&first), "a changed {keyword} member kept the contract");
    }
}

/// The keys of `dependentRequired`, `dependentSchemas` and `dependencies` name parameters, so a
/// parameter called `title`, `summary` or `description` there is part of the contract, and a
/// changed constraint there is breaking under these keywords' whole-value comparison rule.
#[test]
fn dependency_maps_keep_parameter_names() {
    let original = crate::command(crate::SET_SYMBOL_FIELDS).unwrap().to_json();
    let id = original["id"].as_str().unwrap().to_owned();
    for (keyword, first, second) in [
        ("dependentRequired", json!(["metadata"]), json!(["value"])),
        ("dependentSchemas", json!({"required": ["metadata"]}), json!({"required": ["value"]})),
        ("dependencies", json!(["metadata"]), json!({"required": ["value"]})),
    ] {
        for name in ["title", "summary", "description"] {
            let mut with = original.clone();
            with["params"][keyword] = json!({name: first.clone()});
            assert_ne!(contract_of(&with), contract_of(&original), "{keyword}.{name} was dropped as prose");
            let mut changed = with.clone();
            changed["params"][keyword][name] = second.clone();
            let snapshot = snapshot_of(std::slice::from_ref(&with), 1);
            let Err(SnapshotMismatch::Breaking(problems)) = check_snapshot(&snapshot, "commands", std::slice::from_ref(&changed), 1) else {
                panic!("a changed {keyword}.{name} passed the snapshot")
            };
            let was = json!({name: first.clone()});
            let now = json!({name: second.clone()});
            assert_eq!(problems[0], format!("breaking: `{id}` params.{keyword}: changed from {was} to {now}"));
            let mut renamed = original.clone();
            renamed["params"][keyword] = json!({"other": first.clone()});
            assert_ne!(contract_of(&renamed), contract_of(&with), "{keyword} lost the name {name}");
        }
    }
}

/// Two snapshots: a breaking difference fails unless the new one raises the API version, in
/// either section; a removed command is breaking; compatible differences pass and are listed; a
/// version that falls or is not a whole number fails.
#[test]
fn snapshots_compare_against_the_base() {
    let snapshot =
        |version: Value, commands: Value, window: Value| json!({"command_api_version": version, "commands": commands, "window_commands": window});
    let narrow = json!({"params": {"type": "object", "properties": {"n": {"maximum": 3}}, "additionalProperties": false}});
    let wide = json!({"params": {"type": "object", "properties": {"n": {"maximum": 4}}, "additionalProperties": false}});
    let base = snapshot(json!(1), json!({"a.x": wide.clone()}), json!({"ui.y": wide.clone()}));

    assert_eq!(compare_snapshots(&base, &base), Ok(vec![]));
    let loosened = snapshot(json!(1), json!({"a.x": wide.clone(), "a.z": wide.clone()}), json!({"ui.y": wide.clone()}));
    assert_eq!(compare_snapshots(&base, &loosened), Ok(vec!["compatible: `a.z`: added".to_owned()]));

    let tightened = snapshot(json!(1), json!({"a.x": narrow.clone()}), json!({"ui.y": wide.clone()}));
    let problems = compare_snapshots(&base, &tightened).unwrap_err();
    assert_eq!(problems[0], "breaking: `a.x` params.properties.n.maximum: changed from 4 to 3");
    let window_tightened = snapshot(json!(1), json!({"a.x": wide.clone()}), json!({"ui.y": narrow.clone()}));
    assert_eq!(compare_snapshots(&base, &window_tightened).unwrap_err()[0], "breaking: `ui.y` params.properties.n.maximum: changed from 4 to 3");
    let removed = snapshot(json!(1), json!({}), json!({"ui.y": wide.clone()}));
    assert_eq!(compare_snapshots(&base, &removed).unwrap_err()[0], "breaking: `a.x`: removed");

    let raised = snapshot(json!(2), json!({"a.x": narrow.clone()}), json!({"ui.y": wide.clone()}));
    assert_eq!(compare_snapshots(&base, &raised), Ok(vec!["breaking: `a.x` params.properties.n.maximum: changed from 4 to 3".to_owned()]));
    let fallen = snapshot(json!(1), json!({"a.x": wide.clone()}), json!({"ui.y": wide.clone()}));
    assert!(
        compare_snapshots(&snapshot(json!(2), json!({"a.x": wide.clone()}), json!({"ui.y": wide.clone()})), &fallen).unwrap_err()[0]
            .contains("fell from 2 to 1")
    );
    for version in [json!(0), json!(1.0), json!("1")] {
        let bad = snapshot(version, json!({"a.x": wide.clone()}), json!({"ui.y": wide.clone()}));
        assert!(compare_snapshots(&base, &bad).unwrap_err()[0].contains("not a whole number"));
    }
}

/// Both inputs must be usable snapshots: malformed versions, sections and contracts are refused
/// with their field named, rather than compared as empty catalogs.
#[test]
fn snapshot_comparison_refuses_malformed_inputs() {
    let valid = json!({"command_api_version": 1, "commands": {}, "window_commands": {}});
    for version in [Value::Null, json!(0), json!(-1), json!(1.0), json!("1"), json!(u64::from(u32::MAX) + 1)] {
        let mut bad = valid.clone();
        bad["command_api_version"] = version;
        for (base, head, label) in [(&bad, &valid, "base"), (&valid, &bad, "new")] {
            assert!(compare_snapshots(base, head).unwrap_err()[0].starts_with(&format!("the {label} snapshot's command_api_version")));
        }
    }
    let largest = json!({"command_api_version": u32::MAX, "commands": {}, "window_commands": {}});
    assert_eq!(compare_snapshots(&largest, &largest), Ok(vec![]));
    for section in SNAPSHOT_SECTIONS {
        for value in [Value::Null, json!([]), json!(false), json!(1)] {
            let mut bad = valid.clone();
            bad[*section] = value;
            for (base, head, label) in [(&bad, &valid, "base"), (&valid, &bad, "new")] {
                assert_eq!(compare_snapshots(base, head), Err(vec![format!("the {label} snapshot's {section} must be an object")]));
            }
        }
        for contract in [Value::Null, json!({}), json!({"params": null}), json!({"params": []})] {
            let mut bad = valid.clone();
            bad[*section] = json!({"a.x": contract});
            assert_eq!(
                compare_snapshots(&valid, &bad),
                Err(vec![format!("the new snapshot's {section}.a.x must be a contract object with a parameter schema")])
            );
        }
    }
}

/// Refreshing the snapshot in the same commit hides a tightened real command schema from the
/// catalog test, but comparison with the prior snapshot still refuses it until the API rises.
#[test]
fn rewriting_the_snapshot_cannot_hide_a_breaking_change_from_the_base() {
    let original = crate::command(crate::SET_SYMBOL_FIELDS).unwrap().to_json();
    let mut changed = original.clone();
    changed["params"]["required"].as_array_mut().unwrap().push(json!("new_required_parameter"));
    let mut base = snapshot_of(&[original], 1);
    let mut rewritten = snapshot_of(std::slice::from_ref(&changed), 1);
    base["window_commands"] = json!({});
    rewritten["window_commands"] = json!({});
    assert_eq!(check_snapshot(&rewritten, "commands", std::slice::from_ref(&changed), 1), Ok(()));
    let problems = compare_snapshots(&base, &rewritten).unwrap_err();
    assert!(problems[0].contains("params.required: now requires `new_required_parameter`"), "{problems:?}");
    assert!(problems[1].contains("raise COMMAND_API_VERSION above 1"), "{problems:?}");
    rewritten["command_api_version"] = json!(2);
    let accepted = compare_snapshots(&base, &rewritten).unwrap();
    assert_eq!(accepted, vec![problems[0].clone()]);
}

/// The committed snapshot compared with itself passes: what the CI step checks on an unchanged
/// snapshot.
#[test]
fn the_committed_snapshot_matches_itself_as_base() {
    let snapshot: Value = serde_json::from_str(SNAPSHOT).unwrap();
    assert_eq!(compare_snapshots(&snapshot, &snapshot), Ok(vec![]));
}

/// The conditional shove bound accepts the whole older reshape contract, whose closed schema
/// excluded mode. A schema that already accepts mode cannot receive the same proof.
#[test]
fn reshape_shove_limit_is_compatible_with_plain_reshape() {
    let current = contract_of(&crate::command(crate::RESHAPE_TRACKS).unwrap().to_json());
    let params = &current["params"];
    assert_eq!(params["properties"]["tracks"]["maxItems"], json!(crate::board::track::MAX_TRACKS));
    assert_eq!(params["if"], json!({"required":["mode"]}));
    assert_eq!(params["then"]["properties"]["tracks"]["maxItems"], json!(crate::board::track::MAX_PATH_POINTS));

    let mut old = current.clone();
    let params = old["params"].as_object_mut().unwrap();
    params.remove("if");
    params.remove("then");
    params.get_mut("properties").unwrap().as_object_mut().unwrap().remove("mode");
    let changes = compare(crate::RESHAPE_TRACKS, &old, &current);
    assert!(!changes.is_empty() && changes.iter().all(|change| !change.breaking), "{changes:?}");

    let mut open = old.clone();
    open["params"]["additionalProperties"] = json!(true);
    old["params"]["patternProperties"] = json!({"^mode$":true});
    for allows_mode in [open, old] {
        let changes = compare(crate::RESHAPE_TRACKS, &allows_mode, &current);
        assert!(changes.iter().any(|change| change.breaking && change.what.contains("params.if")), "{changes:?}");
    }
}

/// Adding an `if`/`then` (without `else`) whose condition matches only values absent from the
/// old schema's required property enum is compatible, whereas matching an old value, adding an
/// `else`, or lacking an object/required enum guard remains breaking.
#[test]
fn if_then_for_newly_added_enum_variant_is_compatible() {
    let old = json!({
        "type": "object",
        "properties": {"operation": {"type": "string", "enum": ["undo", "redo"]}},
        "required": ["operation"]
    });
    let new_variant = json!({
        "type": "object",
        "properties": {"operation": {"type": "string", "enum": ["undo", "redo", "generate_courtyard"]}},
        "required": ["operation"],
        "if": {"properties": {"operation": {"const": "generate_courtyard"}}, "required": ["operation"]},
        "then": {"properties": {"clearance_nm": {"type": "integer", "minimum": 0}}, "required": ["clearance_nm"]}
    });
    let changes = compare("app.library_edit", &contract_of(&command(old.clone())), &contract_of(&command(new_variant.clone())));
    assert!(!changes.is_empty());
    assert!(changes.iter().all(|c| !c.breaking), "{changes:?}");

    // Matching an existing enum member is breaking.
    let existing_variant = json!({
        "type": "object",
        "properties": {"operation": {"type": "string", "enum": ["undo", "redo"]}},
        "required": ["operation"],
        "if": {"properties": {"operation": {"const": "undo"}}, "required": ["operation"]},
        "then": {"required": ["extra"]}
    });
    let changes = compare("app.library_edit", &contract_of(&command(old.clone())), &contract_of(&command(existing_variant)));
    assert!(changes.iter().any(|c| c.breaking), "{changes:?}");

    // Adding an `else` branch applies to old values and is breaking.
    let mut with_else = new_variant.clone();
    with_else["else"] = json!({"required": ["extra"]});
    let changes = compare("app.library_edit", &contract_of(&command(old.clone())), &contract_of(&command(with_else)));
    assert!(changes.iter().any(|c| c.breaking), "{changes:?}");

    // Without `type: "object"` on `old`, non-object values could satisfy `if` vacuously, so breaking.
    let mut untyped_old = old;
    untyped_old.as_object_mut().unwrap().remove("type");
    let changes = compare("app.library_edit", &contract_of(&command(untyped_old)), &contract_of(&command(new_variant)));
    assert!(changes.iter().any(|c| c.breaking), "{changes:?}");
}
