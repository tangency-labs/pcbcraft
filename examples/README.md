# Dual LED editing demo

This original example contains the wired input connector, two resistor/LED
branches and a capacitor, plus an unconnected NPN transistor for trying a
three-pad package. Symbol and footprint definitions travel with the project,
so it opens without a library service. The example, generator and original
starter definitions use the repository's MIT OR Apache-2.0 license.

| Reference | Footprint |
|---|---|
| R1 | 0402 resistor |
| R2 | 0603 resistor |
| C1 | 0603 capacitor |
| D1, D2 | 0402 LED, with a cathode mark |
| Q1 | SOT-23 NPN, pins 1 Base / 2 Emitter / 3 Collector |
| J1 | 2.54 mm two-pin header with plated drills |

The PCB view draws physical rounded rectangular copper pads, body outlines, courtyards
and silkscreen for the SMD packages. Front copper is red and J1 has gold
plated-hole lands with square pad 1. Fabrication and courtyard graphics start
hidden in new views and can be shown in Layers. Reference labels and silk lines
use physical dimensions. **Properties → Silkscreen** edits their text height,
stroke width and pad clearance; Q1 uses an external triangular pad-1 mark. See [footprint sources and dimensions](../docs/footprint-library.md).
Q1's three terminals are deliberately unwired; the original four connected groups
remain below. This editing example has no board outline or routing.
The full schematic/board walkthrough and preview release remain part of
[#91](https://github.com/tangency-labs/pcbcraft/issues/91).

## Open your own copy

Start the desktop app with `cargo run --locked -p pcbcraft` from the repository
root. On Home, choose **Open Project → Project file or archive…**, then select
[`dual-led/project.pc`](dual-led/project.pc). Before editing, use
**File → Save As… → Project folder…** to save into a new or empty folder outside
the checkout. **View → Fit** brings the circuit into view; collapse Parts if you
want more space. Reopening the distributed example resets the exercise without
changing your saved copy.

In the [browser build](../README.md#run-it), upload
[`dual-led.pc.zip`](dual-led.pc.zip) instead. It contains the same project as the
folder, including its symbols. **Download** produces an edited archive; reopen
that download to check it. Uploading again from the original archive resets the
exercise. The browser keeps its unsaved marker after requesting a download,
because it cannot confirm that you kept the file.

## Try the schematic

1. Expand **Browser → Sheets → Main → Symbols** and select **R1**. Move it a
   little on the canvas: its attached wires follow. Use **Undo**, then **Redo**.
   Press **E** with R1 selected and change its value to `2.2k`, then undo that edit.
2. Clear selection with **Esc**, then drag an empty area from left to right
   around R2 and D2, including their connecting wire. Move the selected branch
   and undo it. A right-to-left box also selects objects it touches; either
   mouse button can draw the box.
3. Click the wire between R1 and D1, press **Delete**, then undo. To try drawing,
   delete it again, press **W**, and connect R1's right pin to D1's left pin.
   Press **Esc** when finished. Existing shared rail junctions are explicit
   connections; crossing a wire elsewhere does not join it.
4. Choose **Capture → Nets**. The unchanged example has four connected groups
   and Q1's three unconnected pins, as listed below. Save your copy and reopen it to
   verify the result. Share the complete folder or archive, not `project.pc`
   alone.

| Connection | Pins in the unchanged demo |
|---|---|
| Input rail | J1.1, R1.1, R2.1, C1.1 |
| Return rail | J1.2, D1.2, D2.2, C1.2 |
| First indicator branch | R1.2, D1.1 |
| Second indicator branch | R2.2, D2.1 |

The connector's left pin is the input rail and its right pin the return rail.
The rail names in this table describe the circuit; they are not net labels in
the saved project. LED values distinguish the two branches without changing
their generic symbol drawing.

## Try PCB transforms

Choose **PCB layout** in the workspace picker, or its split icon to keep the
schematic visible. Select a footprint in Browser and use **Fit** if needed.
Drag a footprint, or press **M** and click its new position. **R**
rotates 90° counterclockwise and **Shift+R** clockwise, including while moving;
**Esc** or a right-click cancels an unfinished move. The origin snaps to the
view's grid even when the grid is hidden. A completed move and its preview
rotations share one undo step, available from either editor.

Save your copy and reopen it to check the stored position and orientation.
Footprint identities and pin mappings stay unchanged. Slots, board outlines, routing and general footprint assignment UI remain
separate work. Existing saved copies keep their original definitions; reopen the
updated distributed folder or archive to try these packages.

## Inspect the mappings through MCP

Build `pcbcraft-cli` with `cargo build --locked -p pcbcraft-cli`. Configure an
MCP client to launch `target/debug/pcbcraft-cli` with arguments
`mcp --project examples/dual-led/project.pc` and the repository root as its
working directory. Add `.exe` on Windows. This opens a separate in-memory
session; headless edits do not change the GUI or write back to the example.

Call `inspect_project` and find R1 in `project.schematic.symbols`. Call
`cross_probe` with the terminal object below, replacing the placeholder with
R1's inspected ID:

```json
{"terminal":{"symbol":"<R1 symbol ID>","terminal":"1"}}
```

Pass the returned `pad` object back as `{"pad": ...}` to find the same terminal.
Every terminal of every component has a mapping, including all three on Q1. This checks correspondence in the
engine; selecting a symbol or footprint in the GUI also highlights its
counterpart in the other workspace.

For a live edit in the desktop window, follow the
[live MCP setup](../README.md#live-mcp-in-the-desktop-editor) with your demo copy
open. Inspect R1 and the current revision, then execute
`schematic.move_symbol` with its ID, its current `x_nm`, `y_nm` reduced by
`2540000`, and the inspected numeric `expected_revision`. The wire follows in
the editor. GUI Undo and Redo share that history; inspect again after each step.
The same live connection can move and rotate footprints through
`pcb.set_footprint_transform`, using the inspected footprint and board IDs,
`x_nm`, `y_nm`, whole-degree `rotation` and `expected_revision`.

## Generate another copy

The checked-in folder and archive were produced by
[`indicator_demo.rs`](../crates/engine/examples/indicator_demo.rs), using shared
engine commands and the production serializer. To build a fresh copy, supply
a destination whose parent exists and which does not already exist:

```sh
cargo run --locked -p pcbcraft-engine --example indicator_demo -- /tmp/pcbcraft-demo
```

That creates `/tmp/pcbcraft-demo/dual-led/project.pc` and
`/tmp/pcbcraft-demo/dual-led.pc.zip`. Choose another path on Windows. Every run
creates a new project and sheet identity; ordinary saves of an existing copy
preserve its IDs and ordering. Use Save As or copy the entire existing folder
to retain those identities. The generator refuses an existing destination and
finishes encoding before writing. If writing fails, its error identifies the
partial output; choose a new destination for the retry.

`cargo test --locked -p pcbcraft-engine --test indicator_demo` checks the
distributed folder/archive agreement, byte-stable saves, all four nets, both
directions of every pin/pad mapping, and move/undo/redo on the demo itself.
