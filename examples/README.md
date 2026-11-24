# Sensor node

The primary demo is a small 3.3 V controller board with an ATtiny402, a TMP102
I²C temperature sensor, two status LEDs, an expansion header and a UPDI header.
It has 15 physical components and ten nets on a 45 × 30 mm, two-layer board.
The [two-transistor LED flasher](flasher.md) remains available as a second demo.

Open [`sensor-node/project.json`](sensor-node/project.json) on desktop, or upload
[`sensor-node.pc`](sensor-node.pc) in the browser. Both forms carry exact native
symbol and footprint copies, so they open offline. Save a copy before editing.

| References | Role | Package |
| --- | --- | --- |
| U1 | ATtiny402; PA1/SDA, PA2/SCL, PA6/PA7 LEDs, PA3/ALERT, PA0/UPDI | SOIC-8, 3.9 mm body |
| U2 | TMP102 temperature sensor; ADD0 grounded for address 0x48 | SOT-563 (DRL), 6 pins |
| J1 | 3.3 V input: 1 supply, 2 ground | 2.54 mm, 1×2 plated header |
| J2 | I²C expansion: 1 supply, 2 ground, 3 SDA, 4 SCL | 2.54 mm, 1×4 plated header |
| J3 | Programming: 1 supply, 2 UPDI, 3 ground | 2.54 mm, 1×3 plated header |
| D1, D2 | Status and activity LEDs, switched low by U1 | 0402; physical pad 1 cathode |
| R1, R2 | 1 kΩ LED current limiting | 0603 |
| R3, R4 | 4.7 kΩ SDA/SCL pullups | 0603 |
| R5 | 10 kΩ open-drain ALERT pullup | 0603 |
| C1, C2, C3 | 100 nF MCU/sensor bypass and 4.7 µF bulk capacitance | 0603 |

Supply the board with regulated 3.3 V. The MCU uses its internal oscillator;
firmware to read the sensor and drive the LEDs is not included. This is an
editing and layout example, with connectivity and the saved board checked by
PCBcraft; it has not been built or tested as hardware.

The schematic groups power, indicators, controller, sensor, pullups and headers.
Local labels connect the groups, while capacitor wires have straight runs before
any corner. **Fit schematic** frames the drawing. Try selecting U1 and its wires,
using **G** to keep connections while moving, or **M** to disconnect and move,
then Undo. In **PCB Layout**, inspect pad correspondence, toggle copper layers,
and run **Analyze → DRC**; removing a track gives the check an edit to report.

## Sources and native library

The [example library](sensor-node-library/) contains original MIT OR Apache-2.0
symbols, recommended land geometry and original silk/courtyard drawings. Pin
assignments and dimensions were checked against Microchip's
[ATtiny202/204/402/404/406 datasheet, DS40002318A](https://ww1.microchip.com/downloads/aemDocuments/documents/MCU08/ProductDocuments/DataSheets/ATtiny202-204-402-404-406-DataSheet-DS40002318A.pdf)
(2021, pp. 13, 17 and 496–498), and TI's
[TMP102 datasheet, SBOS397I](https://www.ti.com/lit/ds/symlink/tmp102.pdf)
(June 2024, p. 3, address table and DRL0006A package/land drawings 4223266/F,
November 2024). Manufacturer drawings and PDFs are referenced, not bundled.
The existing passive/LED/header package notices are described in the
[footprint library](../docs/libraries/footprint-library.md).

## Regenerate and verify

The [generator](../crates/engine/examples/sensor_demo.rs) composes schematic,
package assignment, placement and routing through ordinary revision-checked
commands. Both demos reuse the same command helpers and native folder/archive
encoder; the generator refuses existing output destinations and requires accepted
connectivity, complete routing and zero current design-rule findings before writing.

```sh
cargo run --locked -p pcbcraft-engine --example sensor_demo -- /tmp/pcbcraft-sensor-demo
cargo test --locked -p pcbcraft-engine --test sensor_demo
```

Open `/tmp/pcbcraft-sensor-demo/sensor-node/project.json` or its sibling archive.
The tests compare the distributed folder/archive, independently state all ten
nets and terminal mappings, and check native reopening, move/Undo/Redo and DRC.
