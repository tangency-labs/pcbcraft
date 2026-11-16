//! A board-like STEP assembly, read without error, with its read time and peak memory printed.
//!
//! Every component is its own product holding a package body and its own pins, nothing shared:
//! the worst case for an import. Adapted from SolidCraft's `crates/kernel/examples/
//! step_board_fixture.rs` at revision `73bdb6cdd39efc3899fb54e7217289a630a89efe` (MIT OR
//! Apache-2.0, copyright (c) 2026 SolidCraft contributors); the packages here are plain blocks
//! with sharp edges.
//!
//! `PCBCRAFT_STEP_BOARD_COMPONENTS` and `PCBCRAFT_STEP_BOARD_PINS` set the size. The default, 2
//! components of 16 pins, runs in a few seconds in a debug build. SolidCraft's size, 500
//! components of 16 pins, runs on request with `PCBCRAFT_STEP_BOARD_COMPONENTS=500`; it takes
//! minutes in a debug build, almost all of it the kernel building bodies, and
//! docs/geometry-kernel.md records its figures. A child process builds the file and hands it over
//! on its standard output, so the reader's peak memory is measured in a process that held no
//! kernel bodies before; nothing is written to disk.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let result = if std::env::args().nth(1).as_deref() == Some(board::GENERATE) { board::generate() } else { board::run() };
    if let Err(e) = result {
        eprintln!("step_board: {e}");
        std::process::exit(1);
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod board {
    use std::io::Write;
    use std::process::Command;
    use std::time::Instant;

    use solidcore_ops::{Body, make_box, make_cylinder};
    use solidcore_step::write::product::{Colour, ExportBody, ExportProduct, Header, products};

    /// A count from `PCBCRAFT_STEP_BOARD_<name>`, or `default` when it is unset.
    fn count(name: &str, default: usize) -> Result<usize, String> {
        let key = format!("PCBCRAFT_STEP_BOARD_{name}");
        std::env::var(&key).map_or(Ok(default), |s| s.parse().map_err(|e| format!("{key}: {e}")))
    }

    /// Peak resident memory of this process in KiB (Linux only).
    fn peak_kib() -> Option<u64> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
        line.split_whitespace().nth(1)?.parse().ok()
    }

    /// The argument that makes this program the child that builds the file.
    pub const GENERATE: &str = "generate";

    /// Components when `PCBCRAFT_STEP_BOARD_COMPONENTS` is unset.
    const DEFAULT_COMPONENTS: usize = 2;

    /// One component: a package block and `pins` pins below its two long sides.
    fn component(c: usize, pins: usize) -> Result<Vec<Body>, String> {
        let w = 4.0 + (c % 7) as f64 * 0.37;
        let d = 3.0 + (c % 5) as f64 * 0.29;
        let h = 1.2 + (c % 3) as f64 * 0.1;
        let mut bodies = vec![make_box([0.0, 0.0, 0.0], [w, d, h]).map_err(|e| format!("package {c}: {e}"))?];
        for p in 0..pins {
            let side = if p % 2 == 0 { -0.6 } else { d + 0.6 };
            let x = 0.3 + (p / 2) as f64 * (w - 0.6) / (pins / 2).max(1) as f64 + c as f64 * 1e-4;
            let pin = make_cylinder([x, side, -1.5], 0.25, 2.0 + (p % 3) as f64 * 0.01).map_err(|e| format!("pin {c}.{p}: {e}"))?;
            bodies.push(pin);
        }
        Ok(bodies)
    }

    /// The child: builds the board and writes its STEP text to standard output, its build and
    /// write seconds to standard error.
    pub fn generate() -> Result<(), String> {
        let components = count("COMPONENTS", DEFAULT_COMPONENTS)?;
        let pins = count("PINS", 16)?;
        let start = Instant::now();
        let owned = (0..components).map(|c| component(c, pins)).collect::<Result<Vec<_>, _>>()?;
        let build_s = start.elapsed().as_secs_f64();

        let start = Instant::now();
        let records = owned
            .iter()
            .map(|bodies| bodies.iter().map(|b| b.to_record().map(|(record, _)| record).map_err(|e| e.to_string())).collect::<Result<Vec<_>, _>>())
            .collect::<Result<Vec<_>, _>>()?;
        let grey = Colour { rgb: [0.2, 0.2, 0.2], opacity: 1.0, name: String::new() };
        let mut list: Vec<ExportProduct> = records
            .iter()
            .enumerate()
            .map(|(c, bodies)| ExportProduct {
                name: format!("U{c}"),
                bodies: bodies
                    .iter()
                    .enumerate()
                    .map(|(k, record)| ExportBody { name: format!("U{c}_{k}"), record, color: Some(grey.clone()), faces: Vec::new() })
                    .collect(),
                children: Vec::new(),
            })
            .collect();
        let side = (components as f64).sqrt().ceil().max(1.0) as usize;
        let children = (0..components)
            .map(|c| {
                let (x, y) = ((c % side) as f64 * 12.0, (c / side) as f64 * 10.0);
                // Column-major: the fourth column is the translation, onto the 1.6 mm board's top.
                (c, [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [x, y, 1.6, 1.0]], format!("U{c}"))
            })
            .collect();
        list.push(ExportProduct { name: "Board".into(), bodies: Vec::new(), children });
        let header = Header { file_name: "board.step".into(), ..Header::default() };
        let text = products(&list, list.len() - 1, &header).map_err(|e| e.to_string())?;
        let write_s = start.elapsed().as_secs_f64();
        std::io::stdout().write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        eprintln!("{build_s} {write_s}");
        Ok(())
    }

    /// Seconds of the reader's phases, timed apart: parsing the exchange structure, reading its
    /// solids into records, and reading and walking its product structure.
    fn phases(text: &[u8]) -> Result<String, String> {
        let text = std::str::from_utf8(text).map_err(|e| e.to_string())?;
        let start = Instant::now();
        let exchange = solidcore_step::p21::parse(text).map_err(String::from)?;
        let parse_s = start.elapsed().as_secs_f64();
        let start = Instant::now();
        let solids = solidcore_step::topology::read_solids(&exchange);
        let solids_s = start.elapsed().as_secs_f64();
        let start = Instant::now();
        let structure = solidcore_record::step::product::read(&exchange)?;
        let placed = solidcore_ops::placed_bodies(&structure)?;
        let assembly_s = start.elapsed().as_secs_f64();
        Ok(format!("parse {parse_s:.2} s, {} solid records {solids_s:.2} s, {} placements {assembly_s:.2} s", solids.solids.len(), placed.len()))
    }

    /// The parent: has the child build the board, then reads it.
    pub fn run() -> Result<(), String> {
        let components = count("COMPONENTS", DEFAULT_COMPONENTS)?;
        let pins = count("PINS", 16)?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let child = Command::new(exe).arg(GENERATE).output().map_err(|e| e.to_string())?;
        let note = String::from_utf8_lossy(&child.stderr).trim().to_string();
        if !child.status.success() {
            return Err(format!("building the board failed: {note}"));
        }
        let times: Vec<f64> = note.split_whitespace().filter_map(|s| s.parse().ok()).collect();
        let [build_s, write_s] = times[..] else { return Err(format!("no times from the child: {note}")) };
        let text = child.stdout;

        let phases = phases(&text)?;
        let before = peak_kib();
        let start = Instant::now();
        let read = pcbcraft_geometry::read_step(&text).map_err(|e| e.to_string())?;
        let read_s = start.elapsed().as_secs_f64();
        let peak = peak_kib();

        let bodies = components * (pins + 1);
        if !read.refused.is_empty() {
            return Err(format!("{} items refused, the first {:?}", read.refused.len(), read.refused.first()));
        }
        if read.bodies.len() != bodies || read.placements.len() != bodies {
            return Err(format!("read {} bodies in {} placements, expected {bodies} of each", read.bodies.len(), read.placements.len()));
        }
        let mib = |kib: Option<u64>| kib.map_or_else(|| "unknown".to_string(), |k| format!("{:.0} MiB", k as f64 / 1024.0));
        println!("step_board: reader phases alone: {phases}");
        println!(
            "step_board: {components} components of {pins} pins, {bodies} bodies, {:.1} MB of STEP text; built in {build_s:.1} s, written in {write_s:.1} s, read in {read_s:.1} s; peak resident memory {} holding the text, {} after the read",
            text.len() as f64 / 1e6,
            mib(before),
            mib(peak),
        );
        Ok(())
    }
}
