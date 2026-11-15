//! Original two-layer routing plan for the demonstration board, submitted as native edits.
use super::demo_support::{
    Demo,
    copper::{fanout, mm, route},
};
use pcbcraft_engine::{
    board::setup::{LayerPurpose, Side},
    schematic::Point,
};
use std::{collections::BTreeMap, error::Error};

/// Route the placed pads with front-side series/supply/ground paths and back-side timing links.
/// Endpoints come from resolved physical pads; via fanouts remain outside the SMD lands.
pub(super) fn build(demo: &mut Demo) -> Result<(), Box<dyn Error>> {
    let project = demo.session.project().ok_or("missing project")?;
    let target = project.board_target().ok_or("missing board target")?;
    let board = project.boards().find(|b| b.target == target).ok_or("missing board")?;
    let layers = board.layers();
    let layer = |side| layers.iter().find(|l| l.purpose == LayerPurpose::Copper && l.side == Some(side)).map(|l| l.id).ok_or("missing copper layer");
    let front = layer(Side::Front)?;
    let back = layer(Side::Back)?;
    let pads: BTreeMap<_, _> = project
        .board_view(target)
        .ok_or("missing board view")?
        .footprints()
        .flat_map(|f| f.pads.into_iter().map(move |p| ((f.reference.to_owned(), p.number), p.center)))
        .collect();
    let pad = |reference: &str, number: &str| {
        pads.get(&(reference.to_owned(), number.to_owned())).copied().ok_or_else(|| format!("missing pad {reference}.{number}"))
    };
    let s = &mut demo.session;
    for (r, d) in [("R1", "D1"), ("R2", "D2")] {
        route(s, front, 250_000, pad(r, "2")?, &[], pad(d, "2")?)?;
    }

    // The front supply rail feeds the LED resistors, base bias and both decouplers.
    route(s, front, 250_000, mm(-17., -3.), &[], mm(9., -3.))?;
    for r in ["R1", "R2", "R3", "R4"] {
        let p = pad(r, "1")?;
        route(s, front, 250_000, p, &[], Point { x_nm: p.x_nm, y_nm: -3_000_000 })?;
    }
    route(s, front, 250_000, pad("J1", "1")?, &[(-17., 8.27), (-17., -1.775)], mm(-15.775, -3.))?;
    for c in ["C3", "C4"] {
        let p = pad(c, "1")?;
        route(s, front, 250_000, p, &[], Point { x_nm: -17_000_000, y_nm: p.y_nm })?;
    }

    route(s, front, 250_000, pad("J1", "2")?, &[(-12.5, 10.23), (-12.5, 11.)], mm(8., 11.))?;
    route(s, front, 250_000, pad("C3", "2")?, &[(-13., 4.), (-12.5, 4.5)], mm(-12.5, 11.))?;
    route(s, front, 250_000, pad("C4", "2")?, &[(-13., 0.), (-12.5, 0.5)], mm(-12.5, 4.5))?;
    route(s, front, 250_000, pad("Q1", "2")?, &[(-5.05, 3.), (-6., 3.95)], mm(-6., 11.))?;
    route(s, front, 250_000, pad("Q2", "2")?, &[(8., 3.05)], mm(8., 11.))?;

    // Fan out the two collectors; C2's collector connection stays on front to cross the timing pair.
    for (d, x) in [("D1", -1.), ("D2", 1.)] {
        fanout(s, front, 250_000, pad(d, "1")?, &[(9. * x, -11.5)], mm(10. * x, -12.5))?;
    }
    fanout(s, front, 250_000, pad("Q1", "3")?, &[], mm(-6., -1.2))?;
    fanout(s, front, 250_000, pad("Q2", "3")?, &[], mm(6., -1.2))?;
    fanout(s, front, 250_000, pad("C1", "1")?, &[], mm(-5., 6.))?;
    route(s, front, 250_000, pad("Q2", "3")?, &[(3.5, 0.), (2.225, 1.275)], pad("C2", "1")?)?;
    route(s, back, 250_000, mm(-10., -12.5), &[(-12., -10.5), (-12., -7.2)], mm(-6., -1.2))?;
    route(s, back, 250_000, mm(10., -12.5), &[(12., -10.5), (12., -7.2)], mm(6., -1.2))?;
    route(s, back, 250_000, mm(-6., -1.2), &[(-9., 1.8), (-9., 6.)], mm(-5., 6.))?;
    route(s, back, 250_000, mm(-5., 6.), &[(-4., 7.), (-4., 11.), (-2., 13.), (12., 13.), (13.73, 11.27)], pad("J2", "1")?)?;
    route(s, back, 250_000, mm(6., -1.2), &[(10., 2.8), (12., 2.8), (16.27, 7.07)], pad("J2", "2")?)?;

    fanout(s, front, 250_000, pad("Q1", "1")?, &[(-8., 2.)], mm(-8., 4.))?;
    fanout(s, front, 250_000, pad("Q2", "1")?, &[(4., 2.)], mm(4., 3.))?;
    fanout(s, front, 250_000, pad("R3", "2")?, &[], mm(-3., -7.))?;
    fanout(s, front, 250_000, pad("R4", "2")?, &[], mm(3., -7.))?;
    fanout(s, front, 250_000, pad("C1", "2")?, &[], mm(-1., 6.))?;
    fanout(s, front, 250_000, pad("C2", "2")?, &[], mm(5., 6.))?;
    route(s, back, 250_000, mm(-3., -7.), &[(-1., -5.), (-1., 1.), (-4., 4.)], mm(-8., 4.))?;
    route(s, back, 250_000, mm(3., -7.), &[(1., -5.), (1., 0.)], mm(4., 3.))?;
    route(s, back, 250_000, mm(-8., 4.), &[(-7., 5.), (-3., 5.), (-2., 6.), (-2., 8.), (-1., 9.), (4., 9.), (5., 8.)], mm(5., 6.))?;
    route(s, back, 250_000, mm(4., 3.), &[(3., 4.), (0., 4.), (-1., 5.)], mm(-1., 6.))?;
    Ok(())
}
