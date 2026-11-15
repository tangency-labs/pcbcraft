//! Board composition for the original demo; placements and copper use public commands.
use super::schematic::Demo;
use pcbcraft_engine::{INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT};
use serde_json::json;
use std::error::Error;

/// Assign every physical part explicitly and arrange the two channels symmetrically.
/// Power components retain their native board exclusion and receive no fabricated package.
pub(super) fn build(demo: &mut Demo) -> Result<(), Box<dyn Error>> {
    let session = &mut demo.session;
    session.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":session.revision()}))?;
    for (reference, definition, x, y, rotation) in [
        ("J1", "header-1x02-254", -15_000_000, 9_000_000, 0),
        ("J2", "header-1x02-254", 15_000_000, 9_000_000, 0),
        ("D1", "led-0402", -9_000_000, -10_000_000, 270),
        ("D2", "led-0402", 9_000_000, -10_000_000, 270),
        ("R1", "resistor-0603", -9_000_000, -6_000_000, 90),
        ("R2", "resistor-0603", 9_000_000, -6_000_000, 90),
        ("Q1", "sot23", -6_000_000, 1_000_000, 0),
        ("Q2", "sot23", 6_000_000, 1_000_000, 0),
        ("R3", "resistor-0603", -3_000_000, -5_000_000, 90),
        ("R4", "resistor-0603", 3_000_000, -5_000_000, 90),
        ("C1", "capacitor-0603", -3_000_000, 6_000_000, 0),
        ("C2", "capacitor-0603", 3_000_000, 6_000_000, 0),
        ("C3", "capacitor-0603", -15_000_000, 4_000_000, 0),
        ("C4", "capacitor-0603", -15_000_000, 0, 0),
    ] {
        let mapping = if reference.starts_with('Q') {
            json!({"1":"1","2":"2","3":"3"})
        } else if reference.starts_with('D') {
            json!({"1":"2","2":"1"})
        } else {
            json!({"1":"1","2":"2"})
        };
        session.apply(PLACE_FOOTPRINT,&json!({"symbol":demo.symbols[reference],"definition":definition,"mapping":mapping,"x_nm":x,"y_nm":y,"expected_revision":session.revision()}))?;
        let project = session.project().ok_or("missing project")?;
        let footprint = project
            .board_view(project.board_target().ok_or("missing board target")?)
            .ok_or("missing board view")?
            .footprints()
            .find(|f| f.reference == reference)
            .ok_or("placed footprint missing")?
            .id;
        session.apply(
            pcbcraft_engine::SET_FOOTPRINT_TRANSFORM,
            &json!({"footprint":footprint,"x_nm":x,"y_nm":y,"rotation":rotation,"expected_revision":session.revision()}),
        )?;
        if reference.starts_with('Q') {
            let dx = if reference == "Q1" { -2_400_000 } else { 2_400_000 };
            session.apply(
                pcbcraft_engine::SET_FOOTPRINT_REFERENCE_OFFSET,
                &json!({"footprint":footprint,"offset":{"x_nm":dx,"y_nm":0},"expected_revision":session.revision()}),
            )?;
        }
    }
    let project = session.project().ok_or("missing project")?;
    let target = project.board_target().ok_or("missing board target")?;
    let board = project.boards().find(|board| board.target == target).ok_or("missing board")?;
    let edge =
        board.layers().into_iter().find(|l| l.purpose == pcbcraft_engine::board::setup::LayerPurpose::Outline).ok_or("missing outline layer")?.id;
    session.apply(pcbcraft_engine::ADD_DRAWING,&json!({"layer":edge,"points":[{"x_nm":-20_000_000,"y_nm":-15_000_000},{"x_nm":20_000_000,"y_nm":-15_000_000},{"x_nm":20_000_000,"y_nm":15_000_000},{"x_nm":-20_000_000,"y_nm":15_000_000}],"closed":true,"expected_revision":session.revision()}))?;
    session.apply(pcbcraft_engine::INITIALIZE_UPDATE_BASELINE, &json!({"board":target.board,"expected_revision":session.revision()}))?;
    Ok(())
}
