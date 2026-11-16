use serde_json::{Value, json};

use super::*;

const LOCK: &str = include_str!("../../../../docs/schemas/commands-v1.json");

fn described() -> Vec<Value> {
    crate::catalog().iter().map(crate::Command::to_json).collect()
}

fn lock_of(described: &[Value], catalog_version: u32) -> Value {
    serde_json::from_str(&format!("{{\"catalog_version\":{catalog_version},\"commands\":{}}}", lock_section(described, &Map::new()))).unwrap()
}

fn lock_from(section: &str, catalog_version: u32) -> Value {
    serde_json::from_str(&format!("{{\"catalog_version\":{catalog_version},\"commands\":{section}}}")).unwrap()
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

/// A lowered catalog version is refused, not offered as a lock refresh.
#[test]
fn the_catalog_version_never_falls() {
    let commands = described();
    let lock = lock_of(&commands, 3);
    let Err(LockMismatch::Version(problems)) = check_lock(&lock, "commands", &commands, 2) else { panic!("a falling catalog version passed") };
    assert_eq!(problems, ["CATALOG_VERSION is 2, below the lock's 3; the catalog version never falls"]);
}

/// A removed command stays in the lock with its last version, so its id cannot come back lower.
#[test]
fn a_removed_command_keeps_its_version() {
    let mut commands = described();
    commands[0]["version"] = json!(3);
    let id = commands[0]["id"].as_str().unwrap().to_owned();
    let lock = lock_of(&commands, 3);
    let removed = commands.remove(0);

    let Err(LockMismatch::Stale { problems, section }) = check_lock(&lock, "commands", &commands, 3) else { panic!("a removal passed") };
    assert_eq!(problems, [format!("`{id}` is no longer described; keep it in the lock as removed")]);
    assert!(section.contains(&format!("\"{id}\": {{\"contract\":\"{}\",\"removed\":true,\"version\":3}}", contract_digest(&removed))));
    let refreshed = lock_from(&section, 3);
    assert_eq!(check_lock(&refreshed, "commands", &commands, 3), Ok(()));

    let mut lower = removed.clone();
    lower["version"] = json!(1);
    commands.push(lower);
    let Err(LockMismatch::Version(problems)) = check_lock(&refreshed, "commands", &commands, 3) else {
        panic!("a re-added id went below its version")
    };
    assert_eq!(problems, [format!("`{id}` is version 1, below its version when it was removed 3; versions never fall")]);

    commands.pop();
    commands.push(removed);
    let Err(LockMismatch::Stale { problems, section }) = check_lock(&refreshed, "commands", &commands, 3) else {
        panic!("a re-added id kept its removed mark")
    };
    assert_eq!(problems, [format!("`{id}` was removed and is described again")]);
    assert_eq!(check_lock(&lock_from(&section, 3), "commands", &commands, 3), Ok(()));
}

/// Clients see one list of ids, so a command removed from one section cannot come back lower in
/// another.
#[test]
fn a_command_moved_to_another_section_keeps_its_version() {
    let mut commands = described();
    commands[0]["version"] = json!(3);
    let id = commands[0]["id"].as_str().unwrap().to_owned();
    let mut moved = commands.remove(0);
    let Err(LockMismatch::Stale { section, .. }) = check_lock(&lock_of(&[moved.clone()], 3), "commands", &[], 3) else { panic!("a removal passed") };
    let mut lock = lock_from(&section, 3);
    lock["window_commands"] = json!({});

    moved["version"] = json!(1);
    let Err(LockMismatch::Version(problems)) = check_lock(&lock, "window_commands", &[moved.clone()], 3) else {
        panic!("a moved id went below its version")
    };
    assert_eq!(problems, [format!("`{id}` is version 1, below its version 3 in `commands`; versions never fall")]);

    moved["version"] = json!(3);
    let Err(LockMismatch::Stale { problems, .. }) = check_lock(&lock, "window_commands", &[moved.clone()], 3) else {
        panic!("a moved id was not added to its new section")
    };
    assert_eq!(problems, [format!("`{id}` is not in the lock")]);

    // Moving up is a raise like any other and needs the catalog bump.
    moved["version"] = json!(4);
    let Err(LockMismatch::Version(problems)) = check_lock(&lock, "window_commands", &[moved.clone()], 3) else {
        panic!("a moved id rose without the catalog version")
    };
    assert_eq!(problems, [format!("{id} raised its version; raise CATALOG_VERSION above 3")]);

    // The highest version in any section counts, even when the own section has a lower tombstone.
    let removed = |version: u64| json!({"version": version, "contract": contract_digest(&moved), "removed": true});
    lock["commands"] = json!({id.clone(): removed(5)});
    lock["window_commands"] = json!({id.clone(): removed(2)});
    moved["version"] = json!(3);
    let Err(LockMismatch::Version(problems)) = check_lock(&lock, "window_commands", &[moved], 3) else {
        panic!("an id fell below its version in another section")
    };
    assert_eq!(problems, [format!("`{id}` is version 3, below its version 5 in `commands`; versions never fall")]);
}

/// A lock version that is not a whole number of at least 1 is refused wherever it stands, never
/// read as 0: as the command's own entry, as a removed entry of another section, or as the catalog
/// version.
#[test]
fn a_lock_version_must_be_a_whole_number() {
    let command = json!({"id": "schematic.synthetic", "version": 3, "params": {"type": "object"}});
    let contract = json!(contract_digest(&command));
    // The field as it is written in the lock file; `None` leaves it out.
    let field = |name: &str, version: Option<&str>| version.map_or(String::new(), |version| format!("\"{name}\":{version},"));
    let entry = |version: Option<&str>| format!("{{{}\"contract\":{contract},\"removed\":false}}", field("version", version));
    let lock = |catalog: Option<&str>, own: Option<&str>, other: Option<&str>| {
        format!(
            "{{{}\"commands\":{{\"schematic.synthetic\":{}}},\"window_commands\":{{\"window.synthetic\":{}}}}}",
            field("catalog_version", catalog),
            entry(own),
            entry(other).replace("false", "true"),
        )
    };
    let good = Some("3");
    for spelling in [Some("3.0"), Some("3e0"), Some("18446744073709551616"), Some("\"3\""), Some("-1"), Some("0"), None, good] {
        for (place, text) in [
            ("own section", lock(good, spelling, good)),
            ("other section", lock(good, good, spelling)),
            ("catalog_version", lock(spelling, good, good)),
        ] {
            let lock: Value = serde_json::from_str(&text).unwrap();
            let result = check_lock(&lock, "commands", std::slice::from_ref(&command), 3);
            if spelling == good {
                assert_eq!(result, Ok(()), "{place} at 3");
            } else {
                let Err(LockMismatch::Version(problems)) = result else { panic!("{place} {spelling:?} was read as a version: {result:?}") };
                assert!(problems.len() == 1 && problems[0].contains("not a whole number of at least 1"), "{place} {spelling:?}: {problems:?}");
            }
        }
    }
}

/// One command's state in code: the lock section that describes it (if any), its version and a
/// number standing for its contract, and the catalog version.
#[derive(Clone, Copy, Debug)]
struct Code {
    at: Option<usize>,
    version: u64,
    contract: u64,
    catalog: u64,
}

const SECTIONS: [&str; 2] = ["commands", "window_commands"];

fn synthetic(code: Code) -> Value {
    json!({"id": "schematic.synthetic", "version": code.version, "params": {"type": "object", "properties": {"p": {"type": "integer", "maximum": code.contract}}}})
}

/// Every change one edit can make to `code`: add or re-add in either section (at version 1, the
/// last version or one above, with the last contract or a new one), remove, move to the other
/// section (one version below, the same or one above), change the contract, raise, lower, or raise
/// with a new contract; each with the catalog version kept or raised.
fn edits(code: Code) -> Vec<(String, Code)> {
    let mut changed = Vec::new();
    match code.at {
        None => {
            for (at, name) in SECTIONS.iter().enumerate() {
                for version in [1, code.version, code.version + 1] {
                    for contract in [code.contract, code.contract + 1] {
                        changed.push((format!("add in {name} at v{version} c{contract}"), Code { at: Some(at), version, contract, ..code }));
                    }
                }
            }
        }
        Some(at) => {
            changed.push(("remove".to_owned(), Code { at: None, ..code }));
            for version in [code.version - 1, code.version, code.version + 1].into_iter().filter(|version| *version >= 1) {
                changed.push((format!("move to {} at v{version}", SECTIONS[1 - at]), Code { at: Some(1 - at), version, ..code }));
            }
            changed.push(("change contract".to_owned(), Code { contract: code.contract + 1, ..code }));
            changed.push(("raise".to_owned(), Code { version: code.version + 1, ..code }));
            changed.push(("raise with new contract".to_owned(), Code { version: code.version + 1, contract: code.contract + 1, ..code }));
            if code.version > 1 {
                changed.push(("lower".to_owned(), Code { version: code.version - 1, ..code }));
            }
        }
    }
    changed
        .into_iter()
        .flat_map(|(edit, next)| [(format!("{edit}, raise catalog"), Code { catalog: next.catalog + 1, ..next }), (edit, next)])
        .collect()
}

/// Run every section's lock check for `code`: `Err` with the problems when any section refuses a
/// version; otherwise the lock after pasting every stale section, which must then pass.
fn apply(lock: &Value, code: Code) -> Result<Value, Vec<String>> {
    let described = |at: usize| if code.at == Some(at) { vec![synthetic(code)] } else { Vec::new() };
    let catalog = u32::try_from(code.catalog).unwrap();
    let mut refreshed = lock.clone();
    for (at, name) in SECTIONS.iter().enumerate() {
        match check_lock(lock, name, &described(at), catalog) {
            Ok(()) => {}
            Err(LockMismatch::Version(problems)) => return Err(problems),
            Err(LockMismatch::Stale { section, .. }) => refreshed[*name] = serde_json::from_str(&section).unwrap(),
        }
    }
    refreshed["catalog_version"] = json!(code.catalog);
    for (at, name) in SECTIONS.iter().enumerate() {
        assert_eq!(check_lock(&refreshed, name, &described(at), catalog), Ok(()), "the refreshed {name} section");
    }
    Ok(refreshed)
}

/// Which rule refuses `code` after the accepted history: the highest version accepted with its
/// contract, and the catalog version of the lock.
fn refused_by(published: Option<(u64, u64)>, catalog: u64, code: Code) -> Option<&'static str> {
    if code.catalog < catalog {
        return Some("the catalog version fell");
    }
    let (Some(_), Some((version, contract))) = (code.at, published) else { return None };
    if code.version < version {
        Some("a version fell")
    } else if code.version == version && code.contract != contract {
        Some("a contract changed at the same version")
    } else if code.version > version && code.catalog == catalog {
        Some("a version rose without the catalog version")
    } else {
        None
    }
}

fn walk(lock: &Value, published: Option<(u64, u64)>, code: Code, history: &mut Vec<String>, refusals: &mut BTreeMap<&'static str, usize>) {
    if history.len() == 4 {
        return;
    }
    let catalog = lock["catalog_version"].as_u64().unwrap();
    for (edit, next) in edits(code) {
        history.push(edit);
        let expected = refused_by(published, catalog, next);
        match (apply(lock, next), expected) {
            (Ok(refreshed), None) => {
                let published = if next.at.is_some() { Some((next.version, next.contract)) } else { published };
                walk(&refreshed, published, next, history, refusals);
            }
            (Err(_), Some(rule)) => *refusals.entry(rule).or_default() += 1,
            (Ok(_), Some(rule)) => panic!("{rule}, but the lock accepted: {}", history.join(" > ")),
            (Err(problems), None) => panic!("refused {problems:?}: {}", history.join(" > ")),
        }
        history.pop();
    }
}

/// Every history of up to four edits of one command over two sections keeps the three rules: a
/// version never falls, a changed contract needs a new version, and a raised version needs a raised
/// catalog version. The lock refuses exactly the edits that break one of them.
#[test]
fn every_short_history_keeps_the_version_rules() {
    let lock = json!({"catalog_version": 1, "commands": {}, "window_commands": {}});
    let mut refusals = BTreeMap::new();
    walk(&lock, None, Code { at: None, version: 1, contract: 0, catalog: 1 }, &mut Vec::new(), &mut refusals);
    for rule in ["a version fell", "a contract changed at the same version", "a version rose without the catalog version"] {
        assert!(refusals.get(rule).is_some_and(|count| *count > 0), "no history tried: {rule}");
    }
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
            format!("`{}` is no longer described; keep it in the lock as removed", removed["id"].as_str().unwrap())
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

/// The keys of `dependentRequired`, `dependentSchemas` and `dependencies` name parameters, so a
/// parameter called `title`, `summary` or `description` there is part of the contract. There is no
/// prose in those maps to leave out: each key is a parameter name.
#[test]
fn dependency_maps_keep_parameter_names() {
    let original = crate::command(crate::SET_SYMBOL_FIELDS).unwrap().to_json();
    for (keyword, first, second) in [
        ("dependentRequired", json!(["metadata"]), json!(["value"])),
        ("dependentSchemas", json!({"required": ["metadata"]}), json!({"required": ["value"]})),
        ("dependencies", json!(["metadata"]), json!({"required": ["value"]})),
    ] {
        for name in ["title", "summary", "description"] {
            let mut with = original.clone();
            with["params"][keyword] = json!({name: first.clone()});
            assert_ne!(contract_digest(&with), contract_digest(&original), "{keyword}.{name} was dropped as prose");
            let mut changed = with.clone();
            changed["params"][keyword][name] = second.clone();
            assert_ne!(contract_digest(&changed), contract_digest(&with), "a changed {keyword}.{name} kept the contract");
            let mut renamed = original.clone();
            renamed["params"][keyword] = json!({"other": first.clone()});
            assert_ne!(contract_digest(&renamed), contract_digest(&with), "{keyword} lost the name {name}");
        }
    }
}
