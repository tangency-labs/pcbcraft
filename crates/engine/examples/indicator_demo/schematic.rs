//! Original cross-coupled transistor flasher, assembled with public commands.
use super::demo_support::{Demo, point};
use std::error::Error;

/// Build the complete circuit with explicit rail junctions, cross-coupled timing and probe labels.
/// The 3.3 V supply and ground symbols are ordinary excluded-from-board power components.
pub(super) fn build() -> Result<Demo, Box<dyn Error>> {
    let mut demo = Demo::new("Dual LED · transistor flasher")?;
    demo.place("J1", "basic-components", "connector", "3.3 V input", (-48, -15, 0, true))?;
    demo.place("J2", "basic-components", "connector", "Collector probes", (38, -15, 0, false))?;
    for (r, d, q, x, mirrored, color) in [("R1", "D1", "Q1", -20, true, "Green"), ("R2", "D2", "Q2", 20, false, "Amber")] {
        demo.place(r, "basic-components", "resistor", "470", (x, -17, 270, false))?;
        demo.place(d, "basic-components", "led", color, (x, -7, 270, false))?;
        demo.place(q, "basic-components", "npn", "PMBT3904", (if x < 0 { -16 } else { 16 }, 6, 0, mirrored))?;
    }
    for (reference, x) in [("R3", -7), ("R4", 7)] {
        demo.place(reference, "basic-components", "resistor-iec", "100k", (x, -9, 270, false))?;
    }
    demo.place("C1", "basic-components", "capacitor", "4.7uF", (-12, 0, 0, false))?;
    demo.place("C2", "basic-components", "capacitor", "4.7uF", (12, 0, 180, false))?;
    demo.place("C3", "basic-components", "capacitor", "100nF", (-40, 1, 270, false))?;
    demo.place("C4", "basic-components", "capacitor", "10uF", (-32, 1, 270, false))?;
    demo.place("PWR1", "power", "3v3", "+3V3", (0, -25, 0, false))?;
    demo.place("PWR2", "power", "gnd", "GND", (0, 25, 0, false))?;

    // Junction identities make the rail branches real connections, rather than coincident strokes.
    let supply_bias_l = demo.wire(demo.pin("PWR1", "1")?, point(-7, -25), &[])?;
    let supply = demo.wire(supply_bias_l.clone(), point(-20, -25), &[])?;
    let supply_c4 = demo.wire(supply.clone(), point(-32, -25), &[])?;
    let supply_c3 = demo.wire(supply_c4.clone(), point(-40, -25), &[])?;
    let supply_input = demo.wire(supply_c3.clone(), point(-45, -25), &[])?;
    let supply_bias_r = demo.wire(demo.pin("PWR1", "1")?, point(7, -25), &[])?;
    let supply_right = demo.wire(supply_bias_r.clone(), point(20, -25), &[])?;
    demo.wire(demo.pin("J1", "1")?, supply_input, &[])?;
    demo.wire(supply, demo.pin("R1", "1")?, &[])?;
    demo.wire(supply_right, demo.pin("R2", "1")?, &[])?;
    demo.wire(supply_bias_l, demo.pin("R3", "1")?, &[])?;
    demo.wire(supply_bias_r, demo.pin("R4", "1")?, &[])?;
    demo.wire(supply_c3, demo.pin("C3", "1")?, &[])?;
    demo.wire(supply_c4, demo.pin("C4", "1")?, &[])?;

    let ground_l = demo.wire(demo.pin("PWR2", "1")?, point(-20, 25), &[])?;
    let ground_r = demo.wire(demo.pin("PWR2", "1")?, point(20, 25), &[])?;
    let ground_c4 = demo.wire(ground_l.clone(), point(-32, 25), &[])?;
    let ground_c3 = demo.wire(ground_c4.clone(), point(-40, 25), &[])?;
    let ground_input = demo.wire(ground_c3.clone(), point(-43, 25), &[])?;
    demo.wire(demo.pin("J1", "2")?, ground_input, &[(-43, -14)])?;
    demo.wire(demo.pin("C3", "2")?, ground_c3, &[])?;
    demo.wire(demo.pin("C4", "2")?, ground_c4, &[])?;
    demo.wire(demo.pin("Q1", "2")?, ground_l, &[])?;
    demo.wire(demo.pin("Q2", "2")?, ground_r, &[])?;

    for (r, d, q, c, x) in [("R1", "D1", "Q1", "C1", -20), ("R2", "D2", "Q2", "C2", 20)] {
        demo.wire(demo.pin(r, "2")?, demo.pin(d, "1")?, &[])?;
        let collector = demo.wire(demo.pin(d, "2")?, point(x, 0), &[])?;
        demo.wire(collector.clone(), demo.pin(q, "3")?, &[])?;
        demo.wire(collector, demo.pin(c, "1")?, &[])?;
    }
    let base_a = demo.wire(demo.pin("R3", "2")?, point(-7, 6), &[])?;
    let base_b = demo.wire(demo.pin("R4", "2")?, point(7, 6), &[])?;
    demo.wire(base_a.clone(), demo.pin("Q1", "1")?, &[])?;
    demo.wire(base_b.clone(), demo.pin("Q2", "1")?, &[])?;
    demo.wire(demo.pin("C1", "2")?, base_b, &[(-8, 0), (-8, 14), (7, 14)])?;
    demo.wire(demo.pin("C2", "2")?, base_a, &[(8, 0), (8, 18), (-7, 18)])?;
    demo.label("LED_A", -20, -12)?;
    demo.label("LED_B", 20, -12)?;
    demo.label("COL_A", -20, 0)?;
    demo.label("COL_B", 20, 0)?;
    demo.label("BASE_A", -7, 6)?;
    demo.label("BASE_B", 7, 6)?;
    demo.wire(demo.pin("J2", "1")?, point(30, -16), &[])?;
    demo.wire(demo.pin("J2", "2")?, point(30, -14), &[])?;
    demo.label("COL_A", 30, -16)?;
    demo.label("COL_B", 30, -14)?;
    Ok(demo)
}
