use serde_json::{Value, json};

use super::ErrorCode;
use crate::{CommandError, Session, board::identity::CREATE_BOARD, catalog, net_labels};

const LOCK: &str = include_str!("../../../../docs/schemas/error-codes-v1.json");
const REFERENCE: &str = include_str!("../../../../docs/automation/command-api.md");

fn published() -> Vec<&'static str> {
    ErrorCode::ALL.iter().map(|code| code.as_str()).collect()
}

/// Codes are distinct snake_case words that parse back to themselves.
#[test]
fn codes_are_unique_snake_case_and_round_trip() {
    let mut seen = std::collections::BTreeSet::new();
    for &code in ErrorCode::ALL {
        let text = code.as_str();
        assert!(seen.insert(text), "{text} is listed twice");
        assert!(!text.is_empty() && text.split('_').all(|word| !word.is_empty() && word.bytes().all(|b| b.is_ascii_lowercase())), "{text}");
        assert_eq!(ErrorCode::parse(text), Some(code));
        assert_eq!(code.to_string(), text);
    }
    assert_eq!(ErrorCode::parse("Not_Found"), None);
}

/// The enum and committed lock agree, including publication order. Appending a code requires
/// updating both lists; changing both lists together still needs append-only review.
#[test]
fn the_lock_keeps_every_published_code() {
    let lock: Value = serde_json::from_str(LOCK).unwrap();
    let locked: Vec<&str> = lock["codes"].as_array().unwrap().iter().map(|code| code.as_str().unwrap()).collect();
    let published = published();
    assert!(
        published.starts_with(&locked),
        "published codes are never removed, renamed or reordered: the lock lists {locked:?}, the code {published:?}"
    );
    assert_eq!(
        locked,
        published,
        "append the new codes to docs/schemas/error-codes-v1.json; \"codes\" should read:\n{}",
        serde_json::to_string_pretty(&published).unwrap()
    );
}

/// The protocol reference documents every code.
#[test]
fn the_reference_lists_every_code() {
    for code in published() {
        assert!(REFERENCE.contains(&format!("| `{code}` |")), "docs/automation/command-api.md has no row for `{code}`");
    }
}

/// An open project with a board, so most commands are available and refuse for the reason
/// under test rather than for want of a project.
fn fixture() -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name": "Codes"})).unwrap();
    session.apply(CREATE_BOARD, &json!({"expected_revision": session.revision()})).unwrap();
    session
}

/// Every catalogue command, given each failure the reference documents for all commands,
/// refuses with the documented code and for the documented reason, changes nothing, and
/// reports a published code. A stale revision names the current one, except for route planning,
/// which tests its stale outcome separately. Parameters are checked in
/// their declared order, so a malformed revision is tried on the commands that declare no
/// required parameter before it, where its own check is the one that refuses.
#[test]
fn every_command_reports_its_documented_failures_with_published_codes() {
    let mut session = fixture();
    let revision = session.revision();
    let before = session.inspect();
    let mut wrong = Vec::new();
    let mut malformed_checked = 0;
    let mut expect = |session: &mut Session, id: &str, params: Value, case: &str, code: ErrorCode, reason: &str| -> Option<CommandError> {
        match session.execute(id, &params) {
            Ok(_) => wrong.push(format!("{id} {case}: succeeded, expected {code}")),
            Err(error) => {
                assert!(ErrorCode::ALL.contains(&error.code()), "{id} {case}: {error:?}");
                if error.code() == code && error.to_string().contains(reason) {
                    return Some(error);
                }
                wrong.push(format!("{id} {case}: {} ({error}), expected {code} with {reason:?}", error.code()));
            }
        }
        None
    };
    for command in catalog() {
        let id = command.id;
        let invalid = ErrorCode::InvalidParams;
        expect(&mut session, id, json!([1]), "with non-object parameters", invalid, "takes an object of parameters");
        let unknown_param = "has no parameter `no_such_parameter`";
        expect(&mut session, id, json!({"no_such_parameter": 1}), "with an unknown parameter", invalid, unknown_param);
        if let Some(first) = command.params.iter().find(|param| param.required) {
            let needs = format!("needs `{}`", first.name);
            expect(&mut session, id, json!({}), "without its required parameters", invalid, &needs);
        }
        if command.params.iter().any(|param| param.name == "expected_revision") {
            if id != crate::board::route::PLAN_ROUTE {
                for (expected, case) in [(revision - 1, "with an older revision"), (revision + 7, "with a newer revision")] {
                    let stale = expect(&mut session, id, json!({"expected_revision": expected}), case, ErrorCode::StaleRevision, "revision");
                    if let Some(stale) = stale {
                        assert_eq!(stale.details(), json!({"current_revision": revision}), "{id}");
                    }
                }
            }
            let first_required = command.params.iter().find(|param| param.required);
            if first_required.is_none_or(|param| param.name == "expected_revision") {
                malformed_checked += 1;
                for malformed in [json!("1"), json!(-1), json!(1.5), Value::Null] {
                    expect(&mut session, id, json!({"expected_revision": malformed}), "with a malformed revision", invalid, "`expected_revision`");
                }
            }
            if let Some(other) = command.params.iter().find(|param| param.required && param.name != "expected_revision") {
                let needs = format!("needs `{}`", other.name);
                expect(&mut session, id, json!({"expected_revision": revision}), "with the current revision and nothing else", invalid, &needs);
            }
        }
        let unknown = format!("{id}x");
        if crate::command(&unknown).is_none() {
            expect(&mut session, &unknown, json!({}), "as an unknown id", ErrorCode::UnknownCommand, &format!("unknown command `{unknown}`"));
        }
        assert_eq!(session.revision(), revision, "{id} changed the project while refusing");
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(malformed_checked > 0, "no command was given a malformed revision");
    assert_eq!(session.inspect(), before);
}

/// State, lookup and limit failures carry their own codes.
#[test]
fn state_lookup_and_limit_failures_have_their_codes() {
    let mut empty = Session::default();
    let unavailable = empty.execute("edit.undo", &Value::Null).unwrap_err();
    assert_eq!(unavailable.code(), ErrorCode::Unavailable);
    assert_eq!(unavailable.to_string(), "nothing to undo");

    let mut session = fixture();
    let missing = session.execute(net_labels::DELETE, &json!({"id": "net-label-99", "expected_revision": session.revision()})).unwrap_err();
    assert_eq!(missing.code(), ErrorCode::NotFound, "{missing}");
    assert_eq!(missing.to_string(), "unknown net label on this sheet");
    // An id not of the expected form reads as not found where the lookup parses it, and as
    // invalid parameters where the command checks its form first.
    let revision = session.revision();
    let unit = json!({"component": "garbage", "unit": "a", "x_nm": 0, "y_nm": 0, "expected_revision": revision});
    let part = session.execute(crate::PLACE_UNIT, &unit).unwrap_err();
    assert_eq!(part.code(), ErrorCode::NotFound, "{part}");
    assert_eq!(part.to_string(), "unknown part instance");
    let label = session.execute(net_labels::DELETE, &json!({"id": "garbage", "expected_revision": revision})).unwrap_err();
    assert_eq!(label.code(), ErrorCode::InvalidParams, "{label}");
    assert!(label.to_string().contains("must identify a named net label"), "{label}");

    let grid = crate::schematic::GRID_NM;
    for k in 0..net_labels::MAX_LABELS as i64 {
        let params = json!({"name": format!("N{k}"), "style": "local", "x_nm": k * grid, "y_nm": 0, "expected_revision": session.revision()});
        session.execute(net_labels::ADD, &params).unwrap();
    }
    let revision = session.revision();
    let full = session
        .execute(net_labels::ADD, &json!({"name": "MORE", "style": "local", "x_nm": 0, "y_nm": grid, "expected_revision": revision}))
        .unwrap_err();
    assert_eq!(session.project().unwrap().net_labels.len(), net_labels::MAX_LABELS);
    assert_eq!(full.code(), ErrorCode::CapExceeded, "{full}");
    assert_eq!(full.to_string(), "sheet already has 256 named labels");
    assert_eq!(full.details(), json!({}));
    assert_eq!(session.revision(), revision);
}

/// Callers that edit through `Session::apply` directly, as the GUI and the headless MCP backend
/// do, get the stale revision reported before any other fault and nothing changes; with the
/// current revision the same call reports that other fault.
#[test]
fn apply_reports_a_stale_revision_before_other_faults() {
    let mut session = fixture();
    let revision = session.revision();
    let before = session.inspect();
    let stale = session.apply(net_labels::DELETE, &json!({"id": "garbage", "expected_revision": revision + 1})).unwrap_err();
    assert_eq!(stale.code(), ErrorCode::StaleRevision, "{stale}");
    assert_eq!(stale.to_string(), format!("project changed, inspect the current revision ({revision}) and retry"));
    assert_eq!(stale.details(), json!({"current_revision": revision}));
    assert_eq!(session.revision(), revision);
    assert_eq!(session.inspect(), before);
    let current = session.apply(net_labels::DELETE, &json!({"id": "garbage", "expected_revision": revision})).unwrap_err();
    assert_eq!(current.code(), ErrorCode::InvalidParams, "{current}");
    assert!(current.to_string().contains("must identify a named net label"), "{current}");
    assert_eq!(session.revision(), revision);
}
