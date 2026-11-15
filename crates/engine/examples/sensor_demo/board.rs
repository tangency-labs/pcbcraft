//! Explicit package choices and a compact two-layer physical arrangement.
use super::demo_support::{Demo, copper::mm};
use pcbcraft_engine::{
    ADD_DRAWING, ASSIGN_FOOTPRINT, INITIALIZE_BOARD_SETUP, INITIALIZE_UPDATE_BASELINE, PLACE_ASSIGNED_FOOTPRINT, SET_FOOTPRINT_TRANSFORM,
    board::setup::LayerPurpose,
};
use serde_json::{Map, Value, json};
use std::error::Error;

/// Pin every package through assignments before placing it and accepting board connectivity.
/// MCU and sensor pad numbers follow their manufacturer pinouts; LED numbering is reversed.
pub(super) fn build(d: &mut Demo) -> Result<(), Box<dyn Error>> {
    d.session.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":d.session.revision()}))?;
    for (r, lib, id, n, x, y, rotation) in [
        ("U1", "sensor-demo", "soic8", 8, -7., 0., 0),
        ("U2", "sensor-demo", "sot563", 6, 7., 0., 0),
        ("J1", "basic-components", "header-1x02-254", 2, -17., 9., 0),
        ("J2", "sensor-demo", "header-1x04-254", 4, 19., 1., 0),
        ("J3", "sensor-demo", "header-1x03-254", 3, -17., -7., 0),
        ("D1", "basic-components", "led-0402", 2, -8., -10., 0),
        ("D2", "basic-components", "led-0402", 2, 0., -10., 0),
        ("R1", "basic-components", "resistor-0603", 2, -8., -6., 90),
        ("R2", "basic-components", "resistor-0603", 2, 0., -6., 90),
        ("R3", "basic-components", "resistor-0603", 2, 2., -6., 270),
        ("R4", "basic-components", "resistor-0603", 2, 4., -6., 270),
        ("R5", "basic-components", "resistor-0603", 2, 3., 8., 0),
        ("C1", "basic-components", "capacitor-0603", 2, -12., -2.5, 90),
        ("C2", "basic-components", "capacitor-0603", 2, 10., 2., 0),
        ("C3", "basic-components", "capacitor-0603", 2, -17., 5., 0),
    ] {
        let project = d.session.project().ok_or("missing project")?;
        let component = project.symbols().iter().find(|s| s.id() == d.symbols[r]).ok_or("missing symbol")?.component_id();
        let revision = d.session.libraries().highest_revision(lib, pcbcraft_engine::library::sources::Kind::Footprint, id);
        let definition = d.session.libraries().footprint(lib, id, revision).ok_or("missing package")?;
        let mapping: Map<String, Value> = (1..=n).map(|i| (i.to_string(), json!(if r.starts_with('D') { 3 - i } else { i }.to_string()))).collect();
        let at = mm(x, y);
        d.session.apply(ASSIGN_FOOTPRINT,&json!({"component":component,"library":lib,"definition":id,"definition_revision":definition.revision,"mapping":mapping,"expected_revision":d.session.revision()}))?;
        let target = d.session.project().and_then(|p| p.board_target()).ok_or("missing board")?;
        d.session.apply(
            PLACE_ASSIGNED_FOOTPRINT,
            &json!({"board":target.board,"component":component,"x_nm":at.x_nm,"y_nm":at.y_nm,"expected_revision":d.session.revision()}),
        )?;
        let footprint = d
            .session
            .project()
            .and_then(|p| p.board_view(target))
            .ok_or("missing board view")?
            .footprints()
            .find(|f| f.reference == r)
            .ok_or("missing footprint")?
            .id;
        d.session
            .apply(
                SET_FOOTPRINT_TRANSFORM,
                &json!({"footprint":footprint,"x_nm":at.x_nm,"y_nm":at.y_nm,"rotation":rotation,"expected_revision":d.session.revision()}),
            )
            .map_err(|e| format!("placement {r}: {e}"))?;
    }
    for (reference, x, y) in [("R2", 0., -1.7), ("R3", 2.3, 0.), ("R4", 0., -1.7)] {
        let project = d.session.project().ok_or("missing project")?;
        let target = project.board_target().ok_or("missing board")?;
        let footprint =
            project.board_view(target).ok_or("missing board view")?.footprints().find(|f| f.reference == reference).ok_or("missing footprint")?.id;
        d.session.apply(
            pcbcraft_engine::SET_FOOTPRINT_REFERENCE_OFFSET,
            &json!({"footprint":footprint,"offset":mm(x,y),"expected_revision":d.session.revision()}),
        )?;
    }
    let project = d.session.project().ok_or("missing project")?;
    let target = project.board_target().ok_or("missing board")?;
    let edge = project
        .boards()
        .find(|b| b.target == target)
        .ok_or("missing board")?
        .layers()
        .into_iter()
        .find(|l| l.purpose == LayerPurpose::Outline)
        .ok_or("missing outline layer")?
        .id;
    d.session.apply(ADD_DRAWING,&json!({"board":target.board,"layer":edge,"points":[mm(-22.5,-15.),mm(22.5,-15.),mm(22.5,15.),mm(-22.5,15.)],"closed":true,"expected_revision":d.session.revision()}))?;
    d.session.apply(INITIALIZE_UPDATE_BASELINE, &json!({"board":target.board,"expected_revision":d.session.revision()}))?;
    Ok(())
}
