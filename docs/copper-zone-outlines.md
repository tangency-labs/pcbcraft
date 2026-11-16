# Copper-zone outlines

The outline-authoring slice of [#231](https://github.com/tangency-labs/pcbcraft/issues/231)
creates editable, net-addressed zones on one copper layer. Zones appear as outlines
with light hatching in the layer colour. They do not contain conductive fill yet,
so they do not join pads, change the ratsnest, satisfy DRC connections or appear
as poured copper in fabrication output. Fill and thermal generation belong to
#232; zone fabrication output belongs to #233.

## Drawing and editing

In PCB Layout, activate a visible copper layer and choose **Layout → Route → Zone**.
Choose a current net in **New Zone**. Leave **Rectangle** unchecked for a polygon,
or check it to draw a rectangle with two opposite corners, then choose **Draw**.
A polygon finishes by clicking its first vertex, double-clicking after at least
three vertices, or pressing Enter. Backspace removes its last draft vertex, and
Escape cancels unfinished geometry before leaving the tool. The chooser and
preview do not edit the project. Completing a valid outline creates one undo step,
including promotion of a previously derived net to a persistent identity.

Click an edge to select a zone. Drag a vertex to reshape it, drag an edge to
translate it, or press M and click a destination. Delete removes the selected
zone; Undo and Redo restore its authored records. Hiding its layer or turning off
**Selection filters → Zones** prevents picking it and cancels pending edits.
Zone outlines use the Board Outline gesture implementation, while their shared
commands store zone intent rather than graphics or tracks.

With a zone selected, **Properties** edits its net, copper layer, priority,
clearance override, thermal gap, spoke width, connection style and minimum island
area. Lengths use the selected display units and accept explicit unit suffixes.
Island area is shown in mm². Each committed change is one undoable transaction;
these fill settings are saved intent for the later fill implementation.

The stored net UUID follows the existing registry's anchored continuation after
a split. If that identity becomes ambiguous or retired, Properties and inspection
report that state. A zone never silently switches to another branch or merged
identity. Choose another current net explicitly when that is the desired edit.

## Shared commands

`pcb.add_zone` takes `layer`, `points`, and either a registered `net` UUID or a
structured `target`. A target uses the same scoped persistent/derived address as
net-class assignments: a derived target is `{"kind":"derived","anchor":...}`,
using an exact anchor from net-registry inspection. Promotion and zone creation
publish together. Names shown in the picker do not identify a net.

`pcb.set_zone` takes `zone` and any changed `layer`, `points`, `net`/`target`, or
`options`. `options` replaces the complete settings object; omitting it retains
the current settings. `pcb.delete_zones` takes a nonempty list of distinct existing
UUIDs in `zones`. All three take `expected_revision` and an optional explicit
`board` target, and run through the same GUI, live and headless MCP transaction
path. Unknown/foreign targets, malformed geometry and stale requests leave the
snapshot and history unchanged.

Options contain `priority`, optional `clearance_nm`, `thermal_gap_nm`,
`spoke_width_nm`, `connection` (`thermal`, `solid`, `none`) and
`min_island_area_nm2`. Omitted options on creation use priority 0, inherited
clearance, 0.3 mm gap/spoke width, thermal connections and zero minimum area.
An omitted `clearance_nm` within a replacement options object restores inherited
clearance. Explicit stored null is not an alternate encoding.

## Storage and limits

[Board/20](native-project-storage.md#zone-outline-records) is a superset of board/19
with an optional board-rules object and a required nonempty `zones` array. A board
that also has a placement origin uses board/21 and preserves both records. Each
zone is one JSON line in UUID order and contains only authored intent. A board
without zones keeps the format its other content requires; no fill or preview is
saved. Existing folder/archive codecs, recovery and Undo/Redo use those records.

A board allows 256 zones and 8,192 total vertices, with 3–256 distinct bounded
vertices per zone. Closure is implicit, area must be nonzero and edges must not
self-intersect or touch. Priority is 0–1000. Clearance overrides, thermal gaps
and spoke widths are 10 µm–10 mm, and minimum island area is 0–1 m². Layer UUIDs
must name copper on the owning board, and net UUIDs must exist in its registry.
Zone clipboard duplication and mixed-category marquee selection remain outside
this outline-authoring slice.
