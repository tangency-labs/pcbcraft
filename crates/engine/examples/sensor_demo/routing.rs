//! Original two-layer routes, submitted through the editor's ordinary copper commands.
use super::demo_support::{
    Demo,
    copper::{fanout, mm, route},
};
use pcbcraft_engine::{
    board::setup::{LayerPurpose, Side},
    schematic::Point,
};
use std::{collections::BTreeMap, error::Error};

/// Resolve pad positions from placed packages, then connect supply, ground and the ten named nets.
/// Narrow sensor fanouts use 0.20 mm tracks; vias are 0.60/0.30 mm and remain outside SMD lands.
pub(super) fn build(d: &mut Demo) -> Result<(), Box<dyn Error>> {
    let project = d.session.project().ok_or("missing project")?;
    let target = project.board_target().ok_or("missing board target")?;
    let layers = project.boards().find(|b| b.target == target).ok_or("missing board")?.layers();
    let layer = |side| layers.iter().find(|l| l.purpose == LayerPurpose::Copper && l.side == Some(side)).map(|l| l.id).ok_or("missing copper");
    let f = layer(Side::Front)?;
    let b = layer(Side::Back)?;
    let pads: BTreeMap<_, _> = project
        .board_view(target)
        .ok_or("missing board view")?
        .footprints()
        .flat_map(|f| f.pads.into_iter().map(move |p| ((f.reference.to_owned(), p.number), p.center)))
        .collect();
    let p = |r: &str, n: &str| pads.get(&(r.to_owned(), n.to_owned())).copied().ok_or_else(|| format!("missing pad {r}.{n}"));
    let s = &mut d.session;
    // Front perimeter supply and local decoupling.
    route(s, f, 350_000, p("J1", "1")?, &[(-20., 7.27), (-20., -9.54)], p("J3", "1")?)?;
    route(s, f, 350_000, p("J3", "1")?, &[(-17., -11.), (-15.5, -12.5)], mm(18., -12.5))?;
    for r in ["D1", "D2"] {
        let q = p(r, "2")?;
        route(s, f, 350_000, q, &[], Point { x_nm: q.x_nm, y_nm: -12_500_000 })?;
    }
    for r in ["R3", "R4"] {
        let q = p(r, "1")?;
        route(s, f, 350_000, q, &[], Point { x_nm: q.x_nm, y_nm: -12_500_000 })?;
    }
    route(s, f, 350_000, mm(18., -12.5), &[(18., -3.81)], p("J2", "1")?)?;
    route(s, f, 350_000, p("C3", "1")?, &[], mm(-20., 5.))?;
    route(s, f, 250_000, p("U1", "1")?, &[(-11.845, -1.905)], p("C1", "1")?)?;
    route(s, f, 350_000, p("C1", "1")?, &[(-10.8, -1.75), (-10.8, -10.5)], mm(-12.8, -12.5))?;
    route(s, f, 350_000, p("J1", "1")?, &[(-19.5, 10.23), (-19.5, 11.), (0., 11.), (2.25, 8.75)], p("R5", "1")?)?;
    fanout(s, f, 200_000, p("R5", "1")?, &[], mm(1., 8.))?;
    route(s, f, 200_000, p("U2", "5")?, &[(9.25, 0.)], p("C2", "1")?)?;
    fanout(s, f, 200_000, p("C2", "1")?, &[], mm(9.25, 3.4))?;
    route(s, b, 200_000, mm(9.25, 3.4), &[(10., 4.15), (10., 9.5), (8., 11.5), (2., 11.5), (1., 10.5)], mm(1., 8.))?;
    // Back ground tree, with separate fanouts for ADD0 (address 0x48) and the bypass capacitor.
    route(s, f, 250_000, p("J1", "2")?, &[(-15.73, 5.52)], p("C3", "2")?)?;
    fanout(s, f, 200_000, p("C1", "2")?, &[], mm(-12., -4.5))?;
    fanout(s, f, 200_000, p("U1", "8")?, &[(-3., -1.905)], mm(-3., -3.5))?;
    route(s, b, 250_000, p("J1", "2")?, &[(-17.5, 7.23), (-19., 5.73), (-19., -2.46)], p("J3", "3")?)?;
    route(s, b, 200_000, p("J3", "3")?, &[(-12.04, -4.46)], mm(-12., -4.5))?;
    route(s, b, 200_000, mm(-12., -4.5), &[(-8., -8.5), (-2., -8.5), (-3., -7.5)], mm(-3., -3.5))?;
    route(s, b, 200_000, mm(-3., -3.5), &[(-2., -2.5), (1.4, -2.5)], mm(4.5, 0.6))?;
    fanout(s, f, 200_000, p("U2", "2")?, &[(5.1, 0.)], mm(4.5, 0.6))?;
    fanout(s, f, 200_000, p("U2", "4")?, &[(8.3, 1.06)], mm(8.3, 1.4))?;
    fanout(s, f, 200_000, p("C2", "2")?, &[], mm(12., 2.))?;
    route(s, b, 200_000, mm(4.5, 0.6), &[(5.3, 1.4), (11.4, 1.4)], mm(12., 2.))?;
    route(s, b, 200_000, mm(12., 2.), &[(14.27, -0.27)], p("J2", "2")?)?;
    // UPDI remains between the two SOIC pad rows, preserving its dedicated programming pin.
    route(s, f, 200_000, p("U1", "6")?, &[(-6., 0.635), (-6., 0.), (-14., 0.), (-14.5, -0.5), (-14.5, -4.5)], p("J3", "2")?)?;
    for (r, led) in [("R1", "D1"), ("R2", "D2")] {
        let q = p(r, "2")?;
        route(s, f, 200_000, q, &[(q.x_nm as f64 / 1e6, -9.475)], p(led, "1")?)?;
    }
    fanout(s, f, 200_000, p("U1", "2")?, &[], mm(-13., -0.635))?;
    fanout(s, f, 200_000, p("U1", "3")?, &[(-12., 0.635)], mm(-13., 1.635))?;
    fanout(s, f, 200_000, p("R1", "1")?, &[], mm(-8., -4.))?;
    fanout(s, f, 200_000, p("R2", "1")?, &[(-1., -4.25)], mm(-1., -1.5))?;
    route(s, b, 200_000, mm(-8., -4.), &[(-10., -2.), (-11.635, -2.)], mm(-13., -0.635))?;
    route(s, b, 200_000, mm(-1., -1.5), &[(-7., -1.5), (-10., 1.5), (-13., 1.5)], mm(-13., 1.635))?;
    // SDA crosses the board on front, while SCL reaches the expansion header on back.
    route(s, f, 200_000, p("U1", "4")?, &[(-11., 1.905), (-11., 5.), (14., 5.), (14., -0.5)], p("U2", "6")?)?;
    route(s, f, 200_000, mm(14., 5.), &[(16.27, 5.)], p("J2", "3")?)?;
    fanout(s, f, 200_000, p("R3", "2")?, &[(1., -4.25)], mm(1., -4.))?;
    fanout(s, f, 200_000, p("U2", "6")?, &[], mm(12., -0.5))?;
    route(s, b, 200_000, mm(1., -4.), &[(3., -2.), (10.5, -2.)], mm(12., -0.5))?;
    route(s, f, 200_000, p("U1", "5")?, &[(-3., 1.905), (0., -1.095), (4., -1.095), (5.5, -2.595), (6.26, -1.835)], p("U2", "1")?)?;
    route(s, f, 200_000, p("R4", "2")?, &[], mm(4., -1.095))?;
    fanout(s, f, 200_000, p("R4", "2")?, &[], mm(4., -3.))?;
    route(s, b, 200_000, mm(4., -3.), &[(5., -4.), (5., -8.), (7., -10.), (15., -10.), (21.5, -3.5), (21.5, 2.31)], p("J2", "4")?)?;
    // ALERT is an open-drain signal with its own pullup and a GPIO input at the MCU.
    fanout(s, f, 200_000, p("U1", "7")?, &[], mm(-3., -0.635))?;
    fanout(s, f, 200_000, p("U2", "3")?, &[(5.6, 0.5), (4.5, 1.6)], mm(4.5, 2.))?;
    fanout(s, f, 200_000, p("R5", "2")?, &[], mm(5., 8.))?;
    route(s, b, 200_000, mm(-3., -0.635), &[(-2., 0.365), (0., 0.365), (1.635, 2.)], mm(4.5, 2.))?;
    route(s, b, 200_000, mm(4.5, 2.), &[(5., 2.5)], mm(5., 8.))?;
    Ok(())
}
