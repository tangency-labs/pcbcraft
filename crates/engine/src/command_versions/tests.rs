use serde_json::{Value, json};

use super::*;

const LOCK: &str = include_str!("../../../../docs/schemas/commands-v1.json");

fn described() -> Vec<Value> {
    crate::catalog().iter().map(crate::Command::to_json).collect()
}

fn lock_of(described: &[Value], catalog_version: u32) -> Value {
    serde_json::from_str(&format!("{{\"catalog_version\":{catalog_version},\"commands\":{}}}", lock_section(described))).unwrap()
}

/// Discovery publishes an integer version for every command, taken from the version table.
#[test]
fn every_described_command_has_its_version() {
    for command in described() {
        let id = command["id"].as_str().unwrap();
        assert_eq!(command["version"], json!(version(id)), "{id}");
        assert!(command["version"].as_u64().is_some_and(|version| version >= 1), "{id}");
    }
    assert_eq!(version("schematic.not_a_command"), 1);
    assert!(CAPABILITIES.contains(&"command_versions"));
}

/// The committed lock matches what discovery describes, so a contract change that kept its
/// version fails here.
#[test]
fn every_command_matches_the_committed_lock() {
    let lock: Value = serde_json::from_str(LOCK).unwrap();
    match check_lock(&lock, "commands", &described(), CATALOG_VERSION) {
        Ok(()) => {}
        Err(LockMismatch::Version(problems)) => panic!("{}", problems.join("\n")),
        Err(LockMismatch::Stale { problems, section }) => {
            panic!("{}\nthe \"commands\" section of docs/schemas/commands-v1.json should read:\n  \"commands\": {section}", problems.join("\n"))
        }
    }
}

/// A parameter added to a real command without raising its version is refused, the raised
/// version needs a raised catalog version, and only then may the lock be rewritten.
#[test]
fn a_changed_contract_needs_a_version_bump() {
    let original = described();
    let lock = lock_of(&original, 1);
    assert_eq!(check_lock(&lock, "commands", &original, 1), Ok(()));

    let mut changed = original.clone();
    let undo = changed.iter_mut().find(|command| command["id"] == "edit.undo").unwrap();
    undo["params"]["properties"]["steps"] = json!({"type": "integer", "minimum": 1});
    let Err(LockMismatch::Version(problems)) = check_lock(&lock, "commands", &changed, 1) else { panic!("an unversioned change passed") };
    assert_eq!(problems, ["the contract of `edit.undo` changed; bump the version of `edit.undo` above 1"]);

    let undo = changed.iter_mut().find(|command| command["id"] == "edit.undo").unwrap();
    undo["version"] = json!(2);
    let Err(LockMismatch::Version(problems)) = check_lock(&lock, "commands", &changed, 1) else { panic!("the catalog version stayed") };
    assert_eq!(problems, ["edit.undo raised its version; raise CATALOG_VERSION above 1"]);

    let Err(LockMismatch::Stale { problems, section }) = check_lock(&lock, "commands", &changed, 2) else { panic!("a stale lock passed") };
    assert_eq!(problems, ["`edit.undo` has a new version", "the lock's catalog version is 1, not 2"]);
    assert!(section.contains("\"edit.undo\": {\"contract\":"));
    assert_eq!(check_lock(&lock_of(&changed, 2), "commands", &changed, 2), Ok(()));
}

/// A version below the locked one is refused even when the contract is unchanged.
#[test]
fn versions_never_fall() {
    let mut commands = described();
    commands[0]["version"] = json!(2);
    let lock = lock_of(&commands, 2);
    commands[0]["version"] = json!(1);
    let id = commands[0]["id"].as_str().unwrap().to_owned();
    let Err(LockMismatch::Version(problems)) = check_lock(&lock, "commands", &commands, 2) else { panic!("a falling version passed") };
    assert_eq!(problems, [format!("`{id}` is version 1, below its locked version 2; versions never fall")]);
}

/// Added and removed commands and description key order leave versions alone but keep the lock
/// current.
#[test]
fn added_and_removed_commands_only_refresh_the_lock() {
    let mut commands = described();
    let lock = lock_of(&commands, 1);
    let removed = commands.pop().unwrap();
    let mut added = removed.clone();
    added["id"] = json!("schematic.synthetic_example");
    commands.push(added);
    let Err(LockMismatch::Stale { problems, .. }) = check_lock(&lock, "commands", &commands, 1) else { panic!("a stale lock passed") };
    assert_eq!(
        problems,
        [
            "`schematic.synthetic_example` is not in the lock".to_owned(),
            format!("`{}` is in the lock but no longer described", removed["id"].as_str().unwrap())
        ]
    );

    let reordered: Value = serde_json::from_str(&removed.to_string()).unwrap();
    assert_eq!(contract_digest(&reordered), contract_digest(&removed));
    let mut without_version = removed.clone();
    without_version.as_object_mut().unwrap().remove("version");
    assert_eq!(contract_digest(&without_version), contract_digest(&removed));
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
    assert_eq!(contract_digest(&reworded), contract_digest(&original));

    let mut named = original.clone();
    named["params"]["properties"]["title"] = json!({"type": "string"});
    assert_ne!(contract_digest(&named), contract_digest(&original));
    let mut retyped = named.clone();
    retyped["params"]["properties"]["title"] = json!({"type": "integer"});
    assert_ne!(contract_digest(&retyped), contract_digest(&named));
    let mut required = original.clone();
    required["params"]["required"].as_array_mut().unwrap().push(json!("metadata"));
    assert_ne!(contract_digest(&required), contract_digest(&original));

    for keyword in ["default", "const"] {
        let mut first = named.clone();
        first["params"]["properties"]["title"] = json!({"type": "object", keyword: {"title": "a"}});
        let mut second = first.clone();
        second["params"]["properties"]["title"][keyword] = json!({"title": "b"});
        assert_ne!(contract_digest(&second), contract_digest(&first), "a changed {keyword} value kept the contract");
    }
    for keyword in ["enum", "examples"] {
        let mut first = named.clone();
        first["params"]["properties"]["title"] = json!({"type": "object", keyword: [{"description": "a"}]});
        let mut second = first.clone();
        second["params"]["properties"]["title"][keyword] = json!([{"description": "b"}]);
        assert_ne!(contract_digest(&second), contract_digest(&first), "a changed {keyword} member kept the contract");
    }
}
