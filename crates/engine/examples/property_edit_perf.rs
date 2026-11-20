//! Bounded property-command elapsed service samples; not isolated CPU or presentation timing.
use pcbcraft_engine::{PLACE_SYMBOL, SET_SYMBOL_FIELDS, Session, command, persistence};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    sync::Arc,
    time::Instant,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Refuse replacement of an existing artifact or report. A failed write may leave a partial file.
fn write_new(path: &str, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

/// Generate a small or populated original schematic through production commands and codecs.
fn generate(count: usize, path: &str) -> Result<()> {
    if ![8, 128].contains(&count) {
        return Err("count must be 8 or 128".into());
    }
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Property edit workload"}))?;
    for index in 0..count {
        session.apply(
            PLACE_SYMBOL,
            &json!({"definition":"resistor","x_nm":(index % 16) as i64 * 25_400_000,
            "y_nm":(index / 16) as i64 * 25_400_000,"expected_revision":session.revision()}),
        )?;
    }
    write_new(path, &persistence::encode_packed(session.snapshot().ok_or("missing project")?)?)
}

/// Collect raw warmed-model service costs, checking exact rollback bytes outside the boundary.
/// Reuse the same archive and this instrumentation at both revisions for a matched comparison.
fn run(path: &str, output: &str, iterations: usize) -> Result<()> {
    if !(1..=1_000).contains(&iterations) {
        return Err("iterations must be 1 through 1000".into());
    }
    let bytes = persistence::read_bytes(File::open(path)?)?;
    let project = persistence::decode_packed(&bytes)?.project;
    if ![8, 128].contains(&project.components().count()) || project.sheets().count() != 1 {
        return Err("use an 8- or 128-part generated artifact".into());
    }
    let placement = project.symbols().first().ok_or("missing symbol")?;
    let symbol = placement.id();
    let part = project.component(placement).ok_or("missing symbol owner")?;
    let part_id = part.id();
    let mut initial = Session::default();
    initial.open_snapshot(Arc::clone(&project));
    let inspected = initial.inspect();
    let already_locked = inspected["project"]["components"]
        .as_array()
        .is_some_and(|parts| parts.iter().any(|owner| owner["id"] == part_id && owner["designator_locked"] == true));
    if already_locked || part.reference() == "R1000" {
        return Err("use an unlocked generated artifact with its original references".into());
    }
    let reference = part.reference().to_owned();
    let value = part.value().to_owned();
    let supports_locks = command(SET_SYMBOL_FIELDS).is_some_and(|spec| spec.params.iter().any(|param| param.name == "designator_locked"));
    let mut metrics = serde_json::Map::new();
    let mut modes = vec!["fields"];
    if supports_locks {
        modes.extend(["lock", "preview"]);
    }
    for mode in modes {
        let mut session = Session::default();
        session.open_snapshot(Arc::clone(&project));
        let original = persistence::encode_documents(Arc::clone(&project))?;
        let mut oracle = Session::default();
        oracle.open_snapshot(Arc::clone(&project));
        let mut proposed = json!({"symbol":symbol,"reference":reference,"value":value,"expected_revision":oracle.revision()});
        if mode == "fields" {
            proposed["reference"] = json!("R1000");
        } else {
            proposed["designator_locked"] = json!(true);
        }
        oracle.apply(SET_SYMBOL_FIELDS, &proposed)?;
        let expected = persistence::encode_documents(oracle.snapshot().ok_or("missing proposed project")?)?;
        let paths: std::collections::BTreeSet<_> = original.keys().chain(expected.keys()).collect();
        let mut expected_changes = Vec::new();
        for path in paths {
            if original.get(path) != expected.get(path) {
                let before = original.get(path).map(|bytes| std::str::from_utf8(bytes)).transpose()?;
                let after = expected.get(path).map(|bytes| std::str::from_utf8(bytes)).transpose()?;
                expected_changes.push(json!({"path":path,"before":before,"after":after}));
            }
        }
        let mut samples = Vec::with_capacity(iterations);
        for index in 0..iterations + 8 {
            let mut params = json!({"symbol":symbol,"reference":reference,"value":value,"expected_revision":session.revision()});
            if mode == "fields" {
                params["reference"] = json!("R1000");
            } else {
                params["designator_locked"] = json!(true);
            }
            let revision = session.revision();
            let snapshot = session.snapshot().ok_or("missing project")?;
            if mode == "preview" {
                params["dry_run"] = json!(true);
            }
            let start = Instant::now();
            let response = if mode == "preview" {
                session.execute(SET_SYMBOL_FIELDS, &params)?
            } else {
                session.apply(SET_SYMBOL_FIELDS, &params)?;
                Value::Null
            };
            let elapsed = start.elapsed().as_secs_f64() * 1_000.0;
            if mode == "preview" {
                if response["changes"].as_array().is_none_or(Vec::is_empty)
                    || response["changes"] != json!(expected_changes)
                    || response["revision"] != revision
                    || response["provisional"] != true
                    || session.revision() != revision
                    || !Arc::ptr_eq(&snapshot, &session.snapshot().ok_or("missing project")?)
                {
                    return Err("preview changed authority or omitted its document changes".into());
                }
            } else {
                if session.revision() != revision + 1 {
                    return Err("edit did not publish exactly one revision".into());
                }
                let inspected = session.inspect();
                let owner = inspected["project"]["components"]
                    .as_array()
                    .and_then(|parts| parts.iter().find(|part| part["id"] == part_id))
                    .ok_or("missing edited owner")?;
                if persistence::encode_documents(session.snapshot().ok_or("missing edited project")?)? != expected {
                    return Err("edit differed from its authoritative document oracle".into());
                }
                if mode == "lock" && owner["designator_locked"] != true {
                    return Err("lock was not applied".into());
                }
                if mode == "fields" && owner["reference"] != "R1000" {
                    return Err("reference was not applied".into());
                }
                session.apply("edit.undo", &Value::Null)?;
            }
            if persistence::encode_documents(session.snapshot().ok_or("missing project")?)? != original {
                return Err("sample did not retain or restore exact authored bytes".into());
            }
            if index >= 8 {
                samples.push(elapsed);
            }
        }
        let mut ordered = samples.clone();
        ordered.sort_by(f64::total_cmp);
        let percentile = |percent: usize| ordered[(ordered.len() * percent).div_ceil(100) - 1];
        metrics.insert(
            mode.into(),
            json!({"samples_ms":samples,"p50_ms":percentile(50),"p95_ms":percentile(95),
            "p99_ms":percentile(99),"max_ms":ordered.last()}),
        );
    }
    let report = json!({"schema":1,"boundary":"elapsed synchronous shared engine command service time; eight warmup calls per mode",
        "artifact_sha256":format!("{:x}",Sha256::digest(&bytes)),"parts":project.components().count(),"iterations":iterations,
        "metrics":metrics,"live_mcp":"NOT MEASURED","rendering":"NOT MEASURED",
        "notes":"Environment, compiler, profile and executable/revision hashes must accompany this raw diagnostic report. No CI wall-clock threshold."});
    write_new(output, &serde_json::to_vec_pretty(&report)?)
}

/// Keep generation and measurement finite and refuse ambiguous arguments before touching outputs.
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [action, count, path] if action == "generate" => generate(count.parse()?, path),
        [action, artifact, iterations, output] if action == "run" && Path::new(artifact).is_file() => run(artifact, output, iterations.parse()?),
        _ => Err("usage: property_edit_perf generate 8|128 OUTPUT.pc | run ARTIFACT.pc ITERATIONS OUTPUT.json".into()),
    }
}

/// The reused input reader accepts the limit and refuses cap+1 before decoding.
/// These non-archive bytes test bounded input length, not archive validity or allocation capacity.
#[test]
fn diagnostic_input_reader_has_a_cause_specific_cap() {
    use std::io::Read;
    for (prefix, encoding) in [(b"    ", persistence::Encoding::Json), (b"PK\x03\x04", persistence::Encoding::Zip)] {
        let cap = encoding.input_limit() as u64;
        let input = std::io::Cursor::new(prefix).chain(std::io::repeat(0).take(cap - 4));
        assert_eq!(persistence::read_bytes(input).unwrap().len() as u64, cap);
        let input = std::io::Cursor::new(prefix).chain(std::io::repeat(0).take(cap + 1 - 4));
        let error = persistence::read_bytes(input).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("native file size limit"), "{error}");
    }
}
