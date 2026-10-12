# Initial implementation validation

Validated on 11 October 2026. This records evidence for the initial subset, not
full KiCad workflow equivalence or manufacturing readiness for arbitrary boards.

## Automated Rust checks

`cargo xtask ci` passes: formatting, dependency layers, clippy with warnings denied,
17 tests, and WebAssembly compilation of the shared UI / browser entry point.

Tests cover failed-batch rollback, stale revisions, undo branch ID allocation,
automatic net creation in one undo step, preserved PCB placements, sample wire
paths avoiding foreign pins, counterclockwise pad coordinates, crossing shorts,
same-layer connectivity, atomic save replacement, invalid native files, MCP
negotiation/notifications/argument schemas, recoverable tool errors, actual tab
clicks and rotate/undo key events, and display refresh after an MCP edit.

## MCP client interoperability

The official Python MCP SDK 2.3.0 initialized the Rust server with protocol
2025-11-25, discovered all 30 tools and completed the same exercise in both
headless mode and via the loopback bridge into the running egui app:

- load the example and inspect stable IDs/revision;
- reject a failing batch without committing its preceding command;
- add eight routed track segments as one transaction;
- return zero findings in the implemented checks and zero airwires;
- return a PNG preview, save native data, export KiCad PCB;
- undo and redo the entire routing batch;
- change a component value through its direct command tool.

The SDK was used only as an external test client; the application/server is Rust.

## Independent KiCad validation

KiCad CLI 10.0.3 loaded the generated routed board and reported **zero violations
and zero unconnected items**. Reproduce:

```sh
pcbcraft-cli run examples/led-indicator.json --save led.pcbcraft
pcbcraft-cli run examples/route-led.json --project led.pcbcraft --save routed.pcbcraft --export routed.kicad_pcb
kicad-cli pcb drc routed.kicad_pcb --output drc.json --format json
```

The unrouted sample correctly reports three disconnected connections in both
PCBCraft and KiCad. Footprints are embedded original generic geometry, not parts
selected from a real component manufacturer's footprint specification.

## Builds and visual inspection

- Linux/WSL desktop and CLI build and run.
- Windows GNU desktop and CLI cross-build; CLI runs natively on Windows.
- Shared UI, engine and browser entry compile for wasm32-unknown-unknown.
- Actual desktop screenshots were captured under Xvfb and visually inspected:
  dark schematic, light schematic and routed PCB with circular cutouts. All use
  a single root window. This is not a browser runtime or native Windows GUI test.

The initial routing tool has no obstacle avoidance/shove; initial checks are not
comprehensive ERC/DRC. Full KiCad import, production libraries, arbitrary pad
shapes, zones, vias, simulation and 3D are tracked in ROADMAP.md.
