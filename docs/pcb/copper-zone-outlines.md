# Copper zones

[Zone authoring #231](https://github.com/tangency-labs/pcbcraft/issues/231) creates
editable, net-addressed outlines on one copper layer. [Fill #232](https://github.com/tangency-labs/pcbcraft/issues/232)
derives conductive material inside a closed Board Outline, with clearance holes,
thermal or solid pad connections, priority and minimum island area. Short diagonal
boundary hatching continues to identify the authored outline. [Fabrication output #233](https://github.com/tangency-labs/pcbcraft/issues/233)
writes the poured copper into each copper layer's Gerber file.

## Drawing and editing

In PCB Layout, activate a visible copper layer and choose **Layout → Place → Zone**.
Choose a current net in **New Zone**. Leave **Rectangle** unchecked for a polygon,
or check it to draw a rectangle with two opposite corners, then choose **Draw**.
A polygon finishes by clicking its first vertex, double-clicking after at least
three vertices, or pressing Enter. Backspace removes its last draft vertex, and
Escape cancels unfinished geometry before leaving the tool. The chooser and
preview do not edit the project. Completing a valid outline creates one undo step,
including promotion of a previously derived net to a persistent identity.
An unrelated project edit expires the chosen net even before the first point;
accepted repeated zone creations retain the same choice until another edit occurs.

Click an edge to select a zone. Drag a corner to reshape it or drag an edge
to shift that side parallel to its original direction. Drag inside the selected
zone or press M and click a destination to move the whole outline. Right-click
a selected edge and choose **Add vertex**, or right-click a corner and choose
**Remove vertex**. The outline must keep at least three vertices.

While drawing or moving a corner, a guide shows horizontal or vertical alignment
when the pointer is close to an adjacent corner's coordinate. Hold Alt to bypass
axis alignment. Delete removes the selected zone, and each completed edit is
one undo step. Hiding its layer or turning off
**Selection filters → Zones** prevents picking it and cancels pending edits.
Zone outlines use the Board Outline gesture implementation, while their shared
commands store zone intent rather than graphics or tracks.
Ctrl/Command-click toggles individual zones and Shift-click adds them to the
selection. A left-to-right box contains whole outlines; a right-to-left box
crosses their edges. Zones can join a mixed board selection, and Delete removes
the complete selection in one undo step. Hatch marks are short diagonal strokes
along the boundary, leaving the interior clear.
With multiple board objects selected, M picks up the complete group, including
zones, graphics, footprints, tracks and vias. Click places it in one undo step;
Escape or right-click cancels without changing the selection or document. Explicitly
selected tracks translate once; unselected track ends on moved pads follow the
existing footprint movement policy.

With a zone selected, **Properties** edits its net, copper layer, priority,
clearance override, thermal gap, spoke width, connection style and minimum island
area. Lengths use the selected display units and accept explicit unit suffixes.
Island area is shown in mm². Each committed change is one undoable transaction;
the resulting fill is recalculated from the new saved intent.

The stored net UUID follows the existing registry's anchored continuation after
a split. If that identity becomes ambiguous or retired, Properties and inspection
report that state. A zone never silently switches to another branch or merged
identity. Choose another current net explicitly when that is the desired edit.

With a zone, track, via or pad selected, choose **Net color…** in **Properties**
to set the color for its active persistent net. The color follows that net's outlines and pours across
copper layers, as well as surface-mount pads and tracks with an unambiguous
physical connection to the net. Via rings and plated through-hole pads retain
their usual display colors. Choose **Use layer color** to clear the override,
or turn off **Net colors** in **Layers** to see layer colors in the current view.
Undo and Redo restore net and layer colors in the same order as other editor
edits. The colors stay in local app settings, so changing them leaves the board
file and project revision unchanged.

## Filling and checking

Fill runs automatically after committed board or electrical edits. **Layout →
Board → Pour copper** explicitly recalculates it. The status bar shows active
work or a calculation failure, while zone Properties shows whether current fill
is ready. Pending or failed fill contributes no copper; a previous revision's
material is never displayed as current. A closed Board Outline is required. **Layout → Board → Remove pours** removes
the actual derived copper while retaining zone outlines, so affected airwires
return. It stays unfilled across edits and DRC until **Pour copper** is used again.
B pours; Ctrl+B (Command+B on macOS) removes pours. These actions apply to all
zones on the addressed board, each on its own assigned copper layer.
An explicit pour shows the completed-zone count and elapsed time. Zones can
take different amounts of time to fill. **Cancel** or Escape retires the job
and leaves zone outlines available for editing. Automatic
refills run in the background and offer **Cancel pour** in the board status bar.
After cancellation, use **Pour copper** to resume pouring.
Multiple closed outlines use the board checker's existing union interpretation.

Foreign and unknown copper is cleared by the larger applicable board, object
class or zone minimum, with a 2 nm numerical reserve. Higher-priority zones are
filled first; equal-priority foreign zones clear each other's authored outlines.
Thermal connections use up to four board-axis spokes, retaining only spokes that
reach the remaining material. Solid connects pads directly; None leaves gaps.
Minimum-area removal reports affected pad islands in DRC. **Remove unconnected
islands** in zone Properties also removes any island that touches no pad, via or
track of the zone's net, whatever its size; it is off by default. Fill inspection
lists every removed island with its reason (`minimum_area` or `unconnected`),
area and bounds. Retired or ambiguous
net identities produce no conductive fill and remain explicit findings.

Connectivity and airwires use actual retained islands, so disconnected pieces of
one zone do not become electrically joined merely by sharing its UUID. DRC uses
the same analytic material; stricter diagnostic thresholds check that material
without silently refilling it under different rules. Display triangles have no
electrical authority. An explicitly net-assigned via connects wherever its land
touches retained fill on a layer it spans. Advanced pour and thermal policies are
tracked in [#304](https://github.com/tangency-labs/pcbcraft/issues/304).

Walkaround and track shove check authored copper and mechanical openings. After
a route commits, fill is recomputed for the new tracks and any displaced copper.
Repeated pad clearance profiles and unchanged thermal neighborhoods can reuse
their analytic cuts. A neighborhood includes every interacting pad and the
foreign copper that can affect its spokes. The fill still checks the resulting
plane's islands and connectivity before publishing it for the new board.
During the preview, the existing pour still describes the board before the edit.
A non-plated hole gets clearance on every copper layer. Boards containing arcs or
teardrops report fill as unavailable until those shapes have clearance support.

Fill uses the layer's copper envelopes to select nearby clearance shapes. It
checks each thermal spoke against the local cutouts it can reach. After clipping
the zone boundary, enclosed clearance groups can be assembled as analytic holes
and retained copper islands. Groups with separated source contours stay independent
when their aggregate bounds overlap. The fill subtracts boundary cuts separately
and assigns the enclosed holes to the material faces left by that cut. It uses a
local SolidCore Boolean for nested shapes within one group. An expanded higher-priority
zone can absorb an enclosed clearance cut when it covers the cut's full envelope.
Any cut that may extend beyond the expanded material still goes through the
normal Boolean path. A fill that exceeds
the applicable geometry budget reports unavailable material. The pad count alone
does not determine that budget.

Native calculation uses the bounded snapshot worker pool. Browser fill and
analytic zone checking use disposable same-build Web Workers, with cancellation,
a 30-second timeout and no foreground geometry fallback. Worker failure leaves
fill unavailable until another edit or explicit refill. These bounds do not
promise a maximum duration for an individual native kernel operation.

## Fabrication output

**Manufacture › Fabrication › Outputs** and `pcb.export_fabrication` write the
current fill, the same material DRC checks, into the copper Gerber file of each
zone's layer. Export never pours copper itself. While the fill for the current
board is still being calculated, or after **Remove pours**, it is refused with a
message naming **Pour copper**; it is also refused when the fill failed or left a
zone without material.

Each filled island is one dark region (`G36`/`G37`) tagged `.AperFunction,Conductor`
and carrying its zone's net as `.N`, the same name its pads carry. Every outer
boundary and hole of the island is one contour; holes join their outer boundary by
a horizontal cut-in, a pair of coincident segments, as the Gerber specification
requires (§4.10), so no clear polarity is written. Circular boundary pieces stay
circular arcs (`G75` with `G02`/`G03`), split at quarter turns. Every vertex and arc
centre is rounded once to the file's 1 nm resolution, so the written boundary lies
within about 2 nm of the fill; arcs shorter than 1 µm are written as lines. The
drill files and boards without zones are unchanged. The Manufacturing review draws
pour regions; it flattens arcs to within 1 µm for display only.

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
`spoke_width_nm`, `connection` (`thermal`, `solid`, `none`),
`min_island_area_nm2` and optional `remove_unconnected_islands`. Omitted options
on creation use priority 0, inherited clearance, 0.3 mm gap/spoke width, thermal
connections, zero minimum area and no unconnected island removal.
An omitted `clearance_nm` within a replacement options object restores inherited
clearance. Explicit stored null is not an alternate encoding.

`pcb.unfill_zones` removes derived copper until the next explicit pour, without
deleting zone records. `pcb.refill_zones` accepts an optional explicit `board` and replaces only derived
fill. Headless execution completes synchronously; the live editor schedules work
and returns its pending state. `pcb.inspect_zone_fill` reports pending, ready, unfilled or
unavailable without starting work; it is a query, so a read-only
[headless session](../automation/headless-mcp.md) can run it. A ready result lists, for each zone, its
retained island count, `dropped` count and `removed` islands (`reason`,
`area_nm2`, `bounds_nm` as minimum x, minimum y, maximum x, maximum y). Neither command dirties native files or adds
a document undo entry.

## Storage and limits

Current board files use [format 59](../formats/native-project-storage.md#current-board-records).
Each zone is one JSON line in UUID order and contains only authored intent,
including whether to remove unconnected islands. Older boards load with that
option off.
Removing the final zone omits the array. Historical format 20 requires a
nonempty `zones` array. Folder and archive codecs, recovery, and Undo/Redo
use the authored records. Fill and preview are derived in memory.

A board allows 256 zones and 8,192 total vertices, with 3–256 distinct bounded
vertices per zone. Closure is implicit, area must be nonzero and edges must not
self-intersect or touch. Priority is 0–1000. Clearance overrides, thermal gaps
and spoke widths are 10 µm–10 mm, and minimum island area is 0–1 m². Layer UUIDs
must name copper on the owning board, and net UUIDs must exist in its registry.
Zone clipboard copying and duplication remain outside this slice; a selection
containing zones is refused as a whole rather than copying or cutting a subset.
