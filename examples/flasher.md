# Dual LED flasher (second demo)

This example is a complete two-transistor astable circuit with 14 physical
components, eight named nets and a routed 40 × 30 mm, two-layer board. Both
transistors participate in the cross-coupled timing network. Supply decoupling,
a power header and a collector probe header give the circuit useful connection
points, with no deliberately floating terminals.

The folder and archive carry their exact symbol definitions, physical footprints,
terminal mappings and board geometry, so they open offline. The project,
generator and original library records use MIT OR Apache-2.0.

| References | Role | Package |
| --- | --- | --- |
| J1 | 3.3 V input: pin 1 supply, pin 2 ground | 2.54 mm, 1×2 plated header |
| J2 | Collector probes: pin 1 COL_A, pin 2 COL_B | 2.54 mm, 1×2 plated header |
| Q1, Q2 | PMBT3904 NPN timing stages | SOT-23; 1 Base, 2 Emitter, 3 Collector |
| D1, D2 | Green and amber indicators | 0402 LED; physical pad 1 is cathode |
| R1, R2 | 470 Ω LED resistors | 0603 |
| R3, R4 | 100 kΩ base-bias resistors | 0603 |
| C1, C2 | 4.7 µF, non-polar timing capacitors | 0603 |
| C3, C4 | 100 nF and 10 µF supply decoupling | 0603 |

The board contains 82 track segments and 11 through vias, using front copper for
local connections and back copper for the timing and probe links. References
and original package silkscreen remain visible alongside the copper. The current
board checker reports no clearance, short, unrouted, outline or courtyard findings
with its default rules. [Footprint sources and dimensions](../docs/libraries/footprint-library.md)
record the package definitions and their qualification scope.

## Open a copy

On desktop, choose **Open Project → Project file or archive…** and open
[`dual-led/project.json`](dual-led/project.json). Use **File → Save As… → Project
folder…** before editing the checkout's copy. In the browser, upload
[`dual-led.pc`](dual-led.pc), which contains the same project and dependencies. The [sensor node](README.md) is the primary demo.
Share the complete folder or archive rather than `project.json` alone.

**Fit schematic** frames the circuit. **G** or held dragging moves a symbol with
its wires attached; **M** disconnects and moves it. R1 and D1 form a simple branch
for practicing selection, wiring and shared Undo/Redo. The rectangular R3/R4
symbols and revised connectors also exercise the expanded Basic components
library, while supply and ground use ordinary Power definitions.

Switch to **PCB Layout**, or open it beside the schematic, to inspect pad
correspondence and the two copper layers. Select a component or net in either
view to inspect its counterpart. **Analyze → DRC** checks the routed
board; moving a footprint or removing a track gives that check a concrete edit
to report, and Undo restores the original routing.

## Circuit connections

Local labels join J2 to the collectors without stretching probe wires across
the timing network. Power symbols supply the explicit global names `+3V3` and
`GND`. Crossings without a junction dot do not join the two base nets.

| Net | Physical terminals |
| --- | --- |
| +3V3 | J1.1, R1.1, R2.1, R3.1, R4.1, C3.1, C4.1 |
| GND | J1.2, Q1.2, Q2.2, C3.2, C4.2 |
| LED_A | R1.2, D1.1 |
| LED_B | R2.2, D2.1 |
| COL_A | D1.2, Q1.3, C1.1, J2.1 |
| COL_B | D2.2, Q2.3, C2.1, J2.2 |
| BASE_A | Q1.1, R3.2, C2.2 |
| BASE_B | Q2.1, R4.2, C1.2 |

Every physical terminal maps to one pad. Power symbols are explicitly excluded
from the board and do not introduce dummy footprints. For inspection through
MCP, launch `pcbcraft-cli mcp --project examples/dual-led/project.json`; the
[AI and MCP guide](../docs/mcp.md) covers both headless and live editor sessions.

## Regenerate and verify

The [generator](../crates/engine/examples/indicator_demo.rs) composes separate
schematic, placement and routing modules through revision-checked native commands.
It checks connectivity and the board before encoding either output, and refuses
an existing destination. From the repository root:

```sh
cargo run --locked -p pcbcraft-engine --example indicator_demo -- /tmp/pcbcraft-demo
cargo test --locked -p pcbcraft-engine --test indicator_demo
```

The generator creates `/tmp/pcbcraft-demo/dual-led/project.json` and
`/tmp/pcbcraft-demo/dual-led.pc`. A fresh run has new project identities; ordinary
saves retain those identities. The tests check folder/archive agreement,
byte-stable reopening, the eight intended nets, every terminal/pad mapping,
move/Undo/Redo and the saved board's design-rule result.
