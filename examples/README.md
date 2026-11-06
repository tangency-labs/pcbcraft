# Dual LED editing demo

This original example gives the current schematic editor a small, wired project
to explore. It has an input connector, two resistor/LED branches and a capacitor,
with enough space to select and move each branch. Its symbol definitions travel
with the project, so opening it needs no external library service. The example
and generator use the repository's MIT OR Apache-2.0 license; the four symbols
come from the existing original [starter library](../libraries/starter).

The six generic two-pad footprints exercise component identity and MCP
cross-probing. They are placeholders, not selected physical packages. This is
an editing demonstration, not an electrically or fabrication-validated design;
there is no board outline, routing or graphical PCB editor in this version.
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
   and no unconnected pins, as listed below. Save your copy and reopen it to
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
Both pins of every component have a mapping. This checks correspondence in the
engine; graphical selection between schematic and board is still upcoming.

For a live edit in the desktop window, follow the
[live MCP setup](../README.md#live-mcp-in-the-desktop-editor) with your demo copy
open. Inspect R1 and the current revision, then execute
`schematic.move_symbol` with its ID, its current `x_nm`, `y_nm` reduced by
`2540000`, and the inspected numeric `expected_revision`. The wire follows in
the editor. GUI Undo and Redo share that history; inspect again after each step.
The live adapter currently exposes only symbol movement as an edit.

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
