//! Measure the supported single-sheet editing and file paths on an original synthetic fixture.

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use pcbcraft_engine::{
    CONNECT_PINS, MOVE_SYMBOL, PLACE_FOOTPRINT, PLACE_SYMBOL, Session,
    persistence::{decode, encode},
};
use serde_json::json;

const SYMBOLS: usize = 2_048;
const SAMPLES: usize = 40;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = fixture()?;
    let source = session.snapshot().ok_or("no project open")?;
    let bytes = encode(source.clone())?;
    if let Some(path) = std::env::args_os().nth(1) {
        std::fs::write(path, &bytes)?;
    }
    let pin = source.symbols()[0].terminals()[0];
    println!("{SYMBOLS} symbols, {} wires, {} bytes", source.wires().len(), bytes.len());
    measure("cold connectivity index", |_| {
        let loaded = decode(&bytes)?;
        let start = Instant::now();
        black_box(loaded.connectivity().net_for(pin));
        Ok(start.elapsed())
    })?;
    black_box(source.connectivity().net_for(pin));
    measure("indexed net lookup", |_| {
        let start = Instant::now();
        for _ in 0..1000 {
            black_box(source.connectivity().net_for(black_box(pin)));
        }
        Ok(start.elapsed() / 1000)
    })?;
    measure("pin/pad cross-probe round trip", |_| {
        let start = Instant::now();
        for _ in 0..1000 {
            let pad = source.pad_for_terminal(black_box(pin)).ok_or("no footprint assigned")?;
            black_box(source.terminal_for_pad(black_box(pad)));
        }
        Ok(start.elapsed() / 1000)
    })?;
    measure("move with cached connectivity", |index| {
        let args = json!({"symbol":"symbol-2", "x_nm":0, "y_nm": (index + 1) * 1_270_000, "expected_revision":session.revision()});
        let start = Instant::now();
        session.apply(MOVE_SYMBOL, &args)?;
        black_box(session.project().ok_or("no project open")?.connectivity().net_for(pin));
        Ok(start.elapsed())
    })?;
    measure("full MCP inspection", |_| timed(|| Ok(black_box(session.inspect()))))?;
    measure("encode snapshot", |_| timed(|| Ok(black_box(encode(source.clone())?))))?;
    measure("decode and validate", |_| timed(|| Ok(black_box(decode(&bytes)?))))?;
    Ok(())
}

/// Generate a repeatable arrangement of pins and wires without importing a schematic.
fn fixture() -> Result<Session, Box<dyn std::error::Error>> {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name": "Performance fixture"}))?;
    for index in 0..SYMBOLS {
        session.apply(
            PLACE_SYMBOL,
            &json!({
                "definition": if index % 2 == 0 { "resistor" } else { "led" },
                "x_nm": (index % 64) as i64 * 25_400_000 - 812_800_000,
                "y_nm": (index / 64) * 12_700_000,
                "expected_revision": session.revision()
            }),
        )?;
    }
    for index in 0..SYMBOLS - 1 {
        session.apply(
            CONNECT_PINS,
            &json!({
                "from": {"symbol": format!("symbol-{}", index + 2), "terminal": "1"},
                "to": {"symbol": format!("symbol-{}", index + 3), "terminal": "1"},
                "expected_revision": session.revision()
            }),
        )?;
    }
    for index in 0..SYMBOLS {
        session.apply(
            PLACE_FOOTPRINT,
            &json!({
                "symbol": format!("symbol-{}", index + 2),
                "definition": "generic-through-hole-2",
                "pin_1_pad": if index % 2 == 0 { 1 } else { 2 },
                "x_nm": (index % 64) as i64 * 2_540_000,
                "y_nm": (index / 64) * 2_540_000,
                "expected_revision": session.revision()
            }),
        )?;
    }
    Ok(session)
}

fn timed<T>(action: impl FnOnce() -> Result<T, Box<dyn std::error::Error>>) -> Result<Duration, Box<dyn std::error::Error>> {
    let start = Instant::now();
    action()?;
    Ok(start.elapsed())
}

/// Report elapsed time without imposing machine-dependent timing assertions on CI.
fn measure(name: &str, mut sample: impl FnMut(usize) -> Result<Duration, Box<dyn std::error::Error>>) -> Result<(), Box<dyn std::error::Error>> {
    let mut results: Vec<_> = (0..SAMPLES).map(&mut sample).collect::<Result<_, _>>()?;
    results.sort_unstable();
    println!(
        "{name:30} median {:>8.4} ms  p95 {:>8.4} ms",
        results[SAMPLES / 2].as_secs_f64() * 1000.0,
        results[(SAMPLES * 95 / 100) - 1].as_secs_f64() * 1000.0
    );
    Ok(())
}
