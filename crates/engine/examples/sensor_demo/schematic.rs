//! An original 3.3 V controller with a temperature sensor, expansion and programming headers.
use super::{
    demo_support::{Demo, point},
    library,
};
use std::error::Error;

/// Compose the circuit in functional groups, leaving straight wire space at capacitor contacts.
/// Local labels connect separated functional groups without hiding terminal identities.
pub(super) fn build() -> Result<Demo, Box<dyn Error>> {
    let mut d = Demo::new("Sensor node · MCU and I²C")?;
    library::install(&mut d.session)?;
    d.place("U1", "sensor-demo", "attiny402", "ATtiny402", (-4, 4, 0, false))?;
    d.place("U2", "sensor-demo", "tmp102", "TMP102", (30, 2, 0, false))?;
    d.place("J1", "basic-components", "connector", "3.3 V input", (-52, -20, 0, true))?;
    d.place("J2", "basic-components", "header-4", "I²C expansion", (54, -15, 0, false))?;
    d.place("J3", "basic-components", "header-3", "UPDI", (54, 16, 0, false))?;
    for (c, x, value) in [("C1", -45, "100nF"), ("C2", -35, "100nF"), ("C3", -25, "4.7uF")] {
        d.place(c, "basic-components", "capacitor", value, (x, -12, 270, false))?;
    }
    d.place("PWR1", "power", "3v3", "+3V3", (-35, -20, 0, false))?;
    d.place("PWR2", "power", "gnd", "GND", (-35, -4, 0, false))?;
    for (c, x) in [("C1", -45), ("C2", -35), ("C3", -25)] {
        d.wire(d.pin(c, "1")?, point(x, -20), &[])?;
        d.wire(d.pin(c, "2")?, point(x, -4), &[])?;
    }
    d.wire(point(-45, -20), point(-25, -20), &[])?;
    d.wire(point(-45, -4), point(-25, -4), &[])?;
    d.wire(d.pin("J1", "1")?, point(-45, -20), &[(-45, -21)])?;
    d.wire(d.pin("J1", "2")?, point(-45, -4), &[(-48, -19), (-48, -4)])?;
    for (led, r, y, pin, color) in [("D1", "R1", 8, "2", "Status · green"), ("D2", "R2", 20, "3", "Activity · amber")] {
        d.place(led, "basic-components", "led", color, (-44, y, 0, false))?;
        d.place(r, "basic-components", "resistor-iec", "1k", (-32, y, 180, false))?;
        d.wire(d.pin(led, "1")?, point(-51, y), &[])?;
        d.label("+3V3", -51, y)?;
        d.wire(d.pin(led, "2")?, d.pin(r, "2")?, &[])?;
        let bends = if led == "D1" { vec![(-22, 8), (-22, 0)] } else { vec![(-18, 20), (-18, 8)] };
        d.wire(d.pin(r, "1")?, d.pin("U1", pin)?, &bends)?;
    }
    d.label("STATUS_N", -22, 0)?;
    d.label("ACTIVITY_N", -18, 20)?;
    d.label("LED_A", -40, 8)?;
    d.label("LED_B", -40, 20)?;
    for (pin, net, x, y) in
        [("1", "+3V3", -4, -13), ("8", "GND", -4, 20), ("4", "SDA", 10, -2), ("5", "SCL", 10, 2), ("6", "UPDI", 10, 6), ("7", "ALERT_N", 10, 10)]
    {
        d.wire(d.pin("U1", pin)?, point(x, y), &[])?;
        d.label(net, x, y)?;
    }
    for (pin, net, x, y) in [("1", "SCL", 16, -2), ("6", "SDA", 16, 2), ("3", "ALERT_N", 16, 6), ("5", "+3V3", 30, -15)] {
        d.wire(d.pin("U2", pin)?, point(x, y), &[])?;
        d.label(net, x, y)?;
    }
    d.wire(d.pin("U2", "2")?, point(30, 18), &[])?;
    d.wire(d.pin("U2", "4")?, point(30, 18), &[(42, 6), (42, 18)])?;
    d.label("GND", 34, 18)?;
    for (r, x, net) in [("R3", 6, "SDA"), ("R4", 16, "SCL"), ("R5", 26, "ALERT_N")] {
        d.place(r, "basic-components", "resistor-iec", if r == "R5" { "10k" } else { "4.7k" }, (x, -25, 270, false))?;
        d.wire(d.pin(r, "1")?, point(x, -32), &[])?;
        d.label("+3V3", x, -32)?;
        d.wire(d.pin(r, "2")?, point(x, -18), &[])?;
        d.label(net, x, -18)?;
    }
    for (pin, net, y) in [("1", "+3V3", -18), ("2", "GND", -16), ("3", "SDA", -14), ("4", "SCL", -12)] {
        d.wire(d.pin("J2", pin)?, point(44, y), &[])?;
        d.label(net, 44, y)?;
    }
    for (pin, net, y) in [("1", "+3V3", 14), ("2", "UPDI", 16), ("3", "GND", 18)] {
        d.wire(d.pin("J3", pin)?, point(44, y), &[])?;
        d.label(net, 44, y)?;
    }
    Ok(d)
}
