//! Bounded, engine-only membership/Undo measurement. Build this same source as a temporary
//! engine example on both revisions; the fixture is authored only through shared commands.
use pcbcraft_engine::{Session, part_groups, persistence, PLACE_RESISTOR};
use serde_json::{Value, json};
use std::{error::Error, fs, io::Write, path::Path, time::Instant};
type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Apply an authored edit against the current revision, retaining the ordinary Undo path.
fn apply(session: &mut Session, command: &str, mut params: Value) -> Result<()> {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params)?;
    Ok(())
}

/// Author one reusable 256-part/64-group snapshot without rewriting native documents.
fn generate(path: &Path) -> Result<()> {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Membership history fixture","create_board":false}))?;
    for index in 0..256 {
        apply(&mut session, PLACE_RESISTOR, json!({"x_nm":(index % 16)*25_400_000_i64,"y_nm":(index / 16)*25_400_000_i64}))?;
    }
    for index in 0..64 {
        apply(&mut session, part_groups::CREATE, json!({"name":format!("Group {index:02}")}))?;
    }
    let project = session.snapshot().ok_or("missing generated project")?;
    let parts: Vec<_> = project.part_instance_addresses().collect();
    let groups: Vec<_> = project.part_groups().groups.iter().map(|group| group.id).collect();
    for part in parts {
        apply(&mut session, part_groups::SET_GROUPS, json!({"part_instance":part,"groups":groups}))?;
    }
    let bytes = persistence::encode_packed(session.snapshot().ok_or("missing generated snapshot")?)?;
    fs::OpenOptions::new().write(true).create_new(true).open(path)?.write_all(&bytes)?;
    Ok(())
}

/// Retain a fixed number of real membership edits, then verify exact Undo/Redo and codec state.
/// Timing samples cover each edit; process-resource collection covers the whole child process.
fn run(artifact: &Path, environment: &Path, edits: usize, output: &Path) -> Result<()> {
    if edits == 0 || edits > 1000 {
        return Err("edits must be 1..1000".into());
    }
    let bytes = fs::read(artifact)?;
    let project = persistence::decode_packed(&bytes)?.project;
    let parts: Vec<_> = project.part_instance_addresses().collect();
    let groups: Vec<_> = project.part_groups().groups.iter().map(|group| group.id).collect();
    if parts.len() != 256 || groups.len() != 64 {
        return Err("expected the fixed 256-part/64-group fixture".into());
    }
    if parts.iter().any(|part| {
        part_groups::part_instance_key(part).is_none_or(|key|
            project.groups_of_part_instance(key).count() != groups.len())
    }) {
        return Err("fixture must assign every part to every group".into());
    }
    let seed = persistence::encode_documents(project.clone())?;
    let mut session = Session::default();
    session.open_snapshot(project);
    let mut samples = Vec::with_capacity(edits);
    for index in 0..edits {
        let selected = if index % 2 == 0 { &groups[..63] } else { &groups[..] };
        let revision = session.revision();
        let started = Instant::now();
        apply(&mut session, part_groups::SET_GROUPS,
            json!({"part_instance":parts[0],"groups":selected}))?;
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
        if session.revision() <= revision {
            return Err("membership edit did not advance revision".into());
        }
    }
    let final_documents = persistence::encode_documents(session.snapshot().ok_or("missing edited snapshot")?)?;
    let reopened = persistence::decode_documents(&final_documents)?;
    if persistence::encode_documents(reopened)? != final_documents {
        return Err("edited documents failed canonical roundtrip".into());
    }
    for _ in 0..edits {
        session.apply("edit.undo", &json!({}))?;
    }
    if persistence::encode_documents(session.snapshot().ok_or("missing undone snapshot")?)? != seed {
        return Err("Undo did not restore exact seed documents".into());
    }
    for _ in 0..edits {
        session.apply("edit.redo", &json!({}))?;
    }
    if persistence::encode_documents(session.snapshot().ok_or("missing redone snapshot")?)? != final_documents {
        return Err("Redo did not restore exact edited documents".into());
    }
    let environment: Value = serde_json::from_slice(&fs::read(environment)?)?;
    let report = json!({
        "schema":"pcbcraft-membership-history/1",
        "environment":environment,
        "workload":{"parts":256,"groups":64,"edits":edits,"edited_parts":1,
            "initial_memberships":16384,"undo_redo_steps_each":edits},
        "edit_ms":samples,
        "correctness":{"all_edits_changed_revision":true,"canonical_roundtrip":true,
            "exact_undo":true,"exact_redo":true},
        "limits":["One process per revision; edit samples are not independent process replays.",
            "Whole-process RSS includes startup, decoding, retained history and codec/Undo/Redo oracles.",
            "No allocator profiling, maximum-capacity, UI, Windows, browser or endurance qualification."]
    });
    fs::OpenOptions::new().write(true).create_new(true).open(output)?
        .write_all(serde_json::to_string_pretty(&report)?.as_bytes())?;
    Ok(())
}

/// Match the existing resource collector's run/artifact/environment/iterations/output interface.
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("generate") if args.len() == 3 => generate(Path::new(&args[2])),
        Some("run") if args.len() == 6 => run(Path::new(&args[2]), Path::new(&args[3]),
            args[4].parse()?, Path::new(&args[5])),
        _ => Err("usage: runner generate FIXTURE | runner run FIXTURE ENVIRONMENT EDITS OUTPUT".into()),
    }
}
