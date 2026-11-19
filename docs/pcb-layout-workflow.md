# PCB layout editing workflow

Workflow and plan first recorded November 9, 2026; board-rule and net-class
status updated November 14, 2026. Related work:
[#54](https://github.com/tangency-labs/pcbcraft/issues/54) (board workspace), [#56](https://github.com/tangency-labs/pcbcraft/issues/56) (tracks, vias and interactive routing),
[#57](https://github.com/tangency-labs/pcbcraft/issues/57) (copper zones), [#58](https://github.com/tangency-labs/pcbcraft/issues/58) (DRC), [#59](https://github.com/tangency-labs/pcbcraft/issues/59) (fabrication outputs),
[#77](https://github.com/tangency-labs/pcbcraft/issues/77) (net classes and rules) and [#95](https://github.com/tangency-labs/pcbcraft/issues/95) (two-layer track and via editing).

This guide follows a board from placed footprints to fabrication files.
See the [PCB layout design plan](design/pcb-layout-gaps.md) for staged work
and the [capability inventory](capabilities.md) for current limits.

## The workflow today

When a project has no board, PCB Layout shows **Create board** and **Open
schematic**, along with the sequence for board setup, adding schematic footprints
through Update PCB, and drawing the outline and routing. Opening the workspace
or returning to the schematic does not create a board. **Create board** opens
Board setup as a draft. Cancel returns to the empty state without changing the
project; **Apply board setup** creates the board and saves its settings together
as one undoable edit. Board defaults and net classes become available after creation.

The PCB ribbon separates Layout, Draw, Analyze, Manufacture, Libraries and
View. Layout groups placement, Route, clipboard, selection edits and board
setup/synchronization; alignment is available in Modify's menu. Draw operates
on the active layer, while layer, width and via settings stay visible across
tabs. Analyze holds checks and inspection; Manufacture holds outputs.
Planned tools remain disabled in their task's menu. Every edit below uses a
revision-checked engine command that the GUI, live MCP and headless MCP share.
Each edit is one undo step.

| Step | What works | Where |
| --- | --- | --- |
| Set up | Copper count, stackup and materials, silkscreen settings | Layout › Board › Board setup |
| Place | Assign packages, place, move (M or drag), rotate (R), flip to the bottom side (F), exact X/Y/rotation/side in Properties, align and distribute | Layout › Place / Modify, Properties |
| Draw | Active-layer lines and rectangle outlines: conductive copper segments or non-copper graphics with physical strokes; Board Outline vertex, segment and whole-shape edits | Draw |
| Route | Tracks with an explicit width and clearance-aware 45° walkaround. Stable snapping to pads, track ends and vias. Through vias with V or PgUp/PgDn mid-route, plus standalone vias | Layout › Route |
| Modify | Click, box and U (connected) selection. Selection filters for tracks and vias. M and selection moves, held track drags and D with the [active router policy](pcb/route-planning.md#track-shove), width and via size in Properties, per-track/via locks, Delete | Layout › Modify, Properties |
| Inspect | Net highlight (`), ruler (Ctrl+Shift+M), DRC with a results card and canvas markers, board summary when nothing is selected | Analyze, Properties |
| Output | Gerber X2 copper with net and pin attributes, mask, paste, silkscreen and outline layers, plus separate [Excellon drill files](manufacturing/drill-files.md#fabrication-drill-files) in one archive | Manufacture › Fabrication › Outputs |
| View | Mirrored bottom view, layer visibility and colors | View › Navigate › Flip view, Layers |

When items overlap, hover over them and press **Shift+Space** to cycle through
the selectable items at that point. A click on overlapping tracks starts with
the active copper layer. Layer visibility and selection filters also apply when
cycling. Tracks and vias in the gaps between footprint pads can be clicked
directly.

For closed Board Outline graphics, select an edge, then drag the edge to move
that side parallel to itself. Drag inside the selected outline to move the whole
shape or drag a corner to reshape it. Right-click a selected edge or corner to
add or remove a vertex. The [zone editing guide](pcb/copper-zone-outlines.md#drawing-and-editing)
describes the shared axis alignment and vertex controls.

Copper connectivity is derived from overlap on shared layers. Track edits and
footprint transforms refuse new shorts in the copper they touch. DRC separately
reports existing islands that contain conflicting intended nets, including direct
pad contact and via-mediated connections. It uses accepted board connectivity when
available and otherwise the project-wide logical graph; names are display text,
not the identity used to compare memberships. DRC checks the board without editing
it. A `pcb.check_design` request can override the fixed defaults for that run. Short
findings include sorted copper references and the involved component-terminal keys,
grouped by snapshot-local net index. Those indices must not be used as saved bindings.

## PCB operations

- <a name="unroute-one-connection"></a>[Unroute one connection](pcb/unroute.md)
- <a name="set-pitch-distribution"></a>[Set-pitch distribution](pcb/set-pitch-distribution.md)
- <a name="align-to-a-reference-footprint"></a>[Align to a reference footprint](pcb/align-to-reference-footprint.md)
- <a name="placed-footprint-locks"></a>[Placed-footprint locks](pcb/locks.md#placed-footprint-locks) and
  <a name="fixed-tracks-and-vias"></a>[fixed tracks and vias](pcb/locks.md#fixed-tracks-and-vias)
- <a name="fabrication-drill-files"></a>[Fabrication drill files](manufacturing/drill-files.md)

## Format changes at a glance

| Slice | Stored formats | Stored |
| --- | --- | --- |
| 1. Footprint moves carry copper | none | — |
| 2. Board rules (implemented) | board/19 | Board-scoped rule records |
| 2. Net classes (implemented) | project/20 for definitions; project/21 with bindings; instances/3 for board-owned bindings | Project class definitions and assignments to persistent net IDs; consumers add no saved vocabulary |
| 3. Clearance while routing | none | — |
| 4. Zones | board/20 or a higher compatible format | Zone outlines; fills are not stored |
| 5. Push-and-shove | none | — |
| Placed-footprint lock metadata | [board/34](formats/native-project-storage.md#placed-footprint-lock-metadata) or later | At least one active footprint lock; formats 27–33 remain unsupported |
| Named layer groups | [board/41](formats/native-project-storage.md#named-layer-groups) | Written only when named groups exist; derived groups are not stored |

These are selected layout formats, not a consecutive version list. Canonical
saves use the highest supported format required by the authored data; compatible
extensions keep their validation rules. Older readers refuse unsupported formats.
Canonical no-op saves preserve encoding; accepted noncanonical input can be
normalized, for example by omitting an explicit `locked: false`.

Standalone vias can retain an explicit net through placement and Properties; see
[via net assignments](pcb/via-net-assignments.md) for automatic inheritance, physical
connectivity and unresolved-target behavior.
