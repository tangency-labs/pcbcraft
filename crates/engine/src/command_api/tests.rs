use serde_json::{Value, json};

use super::*;

const SNAPSHOT: &str = include_str!("../../../../docs/schemas/command-contracts.json");

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

/// The committed snapshot matches what discovery describes: a breaking change fails here until
/// `COMMAND_API_VERSION` is raised, and any other change until the snapshot is refreshed.
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
    assert!(section.contains("\"edit.undo\": {\"params\":"));

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
        (p(json!({"maxItems": 8})), p(json!({})), "params.properties.p.maxItems: changed from 8 to absent"),
        (p(json!({"pattern": "^a$"})), p(json!({})), "params.properties.p.pattern: changed from \"^a$\" to absent"),
        (p(json!({"multipleOf": 2})), p(json!({})), "params.properties.p.multipleOf: changed from 2 to absent"),
        (p(json!({"default": 1})), p(json!({})), "params.properties.p.default: changed from 1 to absent"),
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
    ] {
        assert_eq!(differences(old, new), [format!("compatible: `schematic.synthetic` {expected}")]);
    }
}

/// Each change that may refuse a call the old schema accepted, or change its meaning, is labelled
/// breaking with its path, at the top of the parameters, nested, and inside alternatives.
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
        (p(json!({})), p(json!({"maxItems": 3})), "params.properties.p.maxItems: changed from absent to 3"),
        (p(json!({})), p(json!({"pattern": "^a$"})), "params.properties.p.pattern: changed from absent to \"^a$\""),
        (p(json!({"format": "uuid"})), p(json!({"format": "date"})), "params.properties.p.format: changed from \"uuid\" to \"date\""),
        (p(json!({"const": 1})), p(json!({"const": 2})), "params.properties.p.const: changed from 1 to 2"),
        (p(json!({"multipleOf": 2})), p(json!({"multipleOf": 4})), "params.properties.p.multipleOf: changed from 2 to 4"),
        (p(json!({"default": 1})), p(json!({"default": 2})), "params.properties.p.default: changed from 1 to 2"),
        (p(json!({})), p(json!({"default": 1})), "params.properties.p.default: changed from absent to 1"),
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
        "examples",
        "items",
        "additionalProperties",
        "anyOf",
        "oneOf",
        "prefixItems",
    ]
    .into_iter()
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
/// changed constraint there is breaking (the classifier has no rule for these keywords).
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
