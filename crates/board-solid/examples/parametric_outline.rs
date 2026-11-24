//! Build and resize a rectangular test board through the shared drawing commands.
//! The exchange adapter consumes a saved board's STEP and solved outline.
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
    process::ExitCode,
};

use pcbcraft_board_solid::{BoardSolid, Thickness};
use pcbcraft_engine::{
    INITIALIZE_BOARD_SETUP, Project, SET_BOARD_SETUP, Session,
    board::{
        drawing::{ADD_DRAWING, Drawing, SET_DRAWING},
        setup::LayerPurpose,
        view::BoardView,
    },
    persistence::{decode_packed, encode_packed, read_bytes},
    schematic::Point,
};
use pcbcraft_units::{Unit, parse_length};
use serde_json::{Value, json};

const NM_PER_MM: i64 = 1_000_000;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Route the prototype's three operations and print errors without modifying the input.
fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [command, path] if command == "init" => init(Path::new(path)),
        [command, source, width, height] if command == "resize" => resize(Path::new(source), width, height),
        [command, source, step] if command == "snapshot" => snapshot(Path::new(source), Path::new(step)),
        _ => Err("usage: parametric_outline init BOARD.pc | resize BOARD.pc WIDTH HEIGHT | snapshot BOARD.pc BODY.step".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Build the fixture with a fixed board origin, a cutout, and a declared finished thickness.
fn init(path: &Path) -> Result<()> {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Parametric outline prototype","create_board":true}))?;
    edit(&mut session, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2}))?;
    let board = view(session.project().ok_or("missing project")?)?;
    let layer = board.layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).ok_or("missing Board Outline layer")?.id;
    let mut setup = serde_json::to_value(board.setup().ok_or("missing setup")?)?;
    setup["stackup"]["finished_thickness_nm"] = json!(1_600_000);
    edit(&mut session, SET_BOARD_SETUP, json!({"setup":setup}))?;
    for points in [rectangle(0, 0, 80, 50), rectangle(19, 15, 31, 23)] {
        edit(&mut session, ADD_DRAWING, json!({"layer":layer,"points":points,"closed":true}))?;
    }
    let project = session.snapshot().ok_or("missing project")?;
    BoardSolid::build(view(&project)?)?;
    save(path, &encode_packed(project)?, None)
}

/// Evaluate width and height in canonical units and replace the rectangle as one ordinary edit.
fn resize(path: &Path, width: &str, height: &str) -> Result<()> {
    let source = read_bytes(File::open(path)?)?;
    let project = decode_packed(&source)?.project;
    let board = view(&project)?;
    if board.footprints().next().is_some() || !board.tracks().is_empty() || !board.vias().is_empty() || !board.zones().is_empty() {
        return Err("Prototype resize supports a bare substrate. Use the generated fixture.".into());
    }
    let drawings = outlines(board);
    let outline = anchored_rectangle(&drawings)?;
    let width_nm = parse_length(width, Unit::Mm).map_err(|error| format!("Invalid width: {error}"))?;
    let height_nm = parse_length(height, Unit::Mm).map_err(|error| format!("Invalid height: {error}"))?;
    if width_nm <= 0 || height_nm <= 0 {
        return Err("Width and height must be positive.".into());
    }
    let points = corners(0, 0, width_nm, height_nm);
    if outline.points == points {
        return Ok(());
    }
    for cutout in drawings.iter().filter(|drawing| drawing.id != outline.id) {
        if !cutout.closed || cutout.points.iter().any(|p| p.x_nm <= 0 || p.x_nm >= width_nm || p.y_nm <= 0 || p.y_nm >= height_nm) {
            return Err(format!("The resized outline would intersect or exclude cutout {}.", cutout.id).into());
        }
    }
    let mut session = Session::default();
    session.open_snapshot(project);
    edit(&mut session, SET_DRAWING, json!({"drawing":outline.id,"points":points,"closed":true}))?;
    let candidate = session.snapshot().ok_or("missing resized project")?;
    BoardSolid::build(view(&candidate)?)?;
    save(path, &encode_packed(candidate)?, Some(&source))
}

/// Export a solved board in the shared frame and emit the prototype's metadata as JSON.
fn snapshot(source: &Path, step: &Path) -> Result<()> {
    let project = decode_packed(&read_bytes(File::open(source)?)?)?.project;
    let board = view(&project)?;
    let solid = BoardSolid::build(board)?;
    let Thickness::SpecifiedFinishedNm(thickness) = solid.thickness else {
        return Err("Declare the finished board thickness in Board Setup before publishing.".into());
    };
    let bounds = solid.body.bounding_box()?;
    let outline_paths = outlines(board)
        .into_iter()
        .map(|drawing| {
            let points = drawing.points.into_iter().map(|p| [mm(p.x_nm), mm(-p.y_nm)]).collect::<Vec<_>>();
            json!({"id":drawing.id,"points":points,"closed":drawing.closed})
        })
        .collect::<Vec<_>>();
    let data = json!({
        "format":"board/1",
        "project":board.target.project,
        "board":board.target.board,
        "thickness_mm":mm(thickness),
        "outline_paths":outline_paths,
        "bounds_mm":{"min":bounds.min_nm.map(mm),"max":bounds.max_nm.map(mm)},
        "width_mm":mm(bounds.max_nm[0] - bounds.min_nm[0]),
        "height_mm":mm(bounds.max_nm[1] - bounds.min_nm[1])
    });
    let step_bytes = pcbcraft_geometry::write_step("PCBcraft outline", &[solid.body])?;
    fs::write(step, step_bytes)?;
    println!("{data}");
    Ok(())
}

/// Resolve the project's explicit board through its immutable physical view.
fn view(project: &Project) -> Result<BoardView<'_>> {
    let target = project.board_target().ok_or("project has no board")?;
    project.board_view(target).ok_or_else(|| "missing board".into())
}

/// Read the Board Outline layer using the same view as the solid builder.
fn outlines(board: BoardView<'_>) -> Vec<Drawing> {
    let layers = board.layers();
    board
        .drawings()
        .into_iter()
        .filter(|drawing| layers.iter().any(|layer| layer.id == drawing.layer && layer.purpose == LayerPurpose::Outline))
        .collect()
}

/// Find the only closed rectangle anchored at board zero, in the prototype's point order.
fn anchored_rectangle(drawings: &[Drawing]) -> Result<&Drawing> {
    let mut matches = drawings.iter().filter(|drawing| {
        let [_, _, far, _] = drawing.points.as_slice() else { return false };
        drawing.closed && far.x_nm > 0 && far.y_nm > 0 && drawing.points == corners(0, 0, far.x_nm, far.y_nm)
    });
    let outline = matches.next().ok_or("use an axis-aligned rectangle anchored at board zero (try init)")?;
    if matches.next().is_some() {
        return Err("ambiguous anchored rectangle".into());
    }
    Ok(outline)
}

/// Call the same revision-checked mutation as the editor.
fn edit(session: &mut Session, command: &str, mut params: Value) -> Result<()> {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params)?;
    Ok(())
}

/// Stage a complete archive and hold the editor's sibling lock across the stale check and publish.
fn save(path: &Path, bytes: &[u8], prior: Option<&[u8]>) -> Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut staged = tempfile::NamedTempFile::new_in(dir)?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    let lock = pcbcraft_storage::open_archive_save_lock(path)?;
    lock.try_lock().map_err(|error| format!("Could not lock project for saving: {error}"))?;
    if let Some(prior) = prior {
        if prior != read_bytes(File::open(path)?)? {
            return Err("The saved board changed while preparing the resize.".into());
        }
        staged.persist(path)?;
    } else {
        staged.persist_noclobber(path)?;
    }
    Ok(())
}

/// Convert fixture millimetres into exact native point coordinates.
fn rectangle(left: i64, top: i64, right: i64, bottom: i64) -> [Point; 4] {
    corners(left * NM_PER_MM, top * NM_PER_MM, right * NM_PER_MM, bottom * NM_PER_MM)
}

fn corners(left: i64, top: i64, right: i64, bottom: i64) -> [Point; 4] {
    [(left, top), (right, top), (right, bottom), (left, bottom)].map(|(x_nm, y_nm)| Point { x_nm, y_nm })
}

fn mm(nm: i64) -> f64 {
    nm as f64 / NM_PER_MM as f64
}

#[test]
fn archive_save_rejects_a_held_lock_and_a_stale_baseline() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("board.pc");
    save(&path, b"original", None)?;

    let editor_lock = pcbcraft_storage::open_archive_save_lock(&path)?;
    editor_lock.try_lock()?;
    assert!(save(&path, b"contender", Some(b"original")).is_err());
    assert_eq!(fs::read(&path)?, b"original");
    drop(editor_lock);

    save(&path, b"winner", Some(b"original"))?;
    assert!(save(&path, b"contender", Some(b"original")).is_err());
    assert_eq!(fs::read(&path)?, b"winner");
    Ok(())
}
