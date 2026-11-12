# PCB layout editing workflow

Status and plan recorded November 9, 2026. Related work:
[#54](https://github.com/tangency-labs/pcbcraft/issues/54) (board workspace), [#56](https://github.com/tangency-labs/pcbcraft/issues/56) (tracks, vias and interactive routing),
[#57](https://github.com/tangency-labs/pcbcraft/issues/57) (copper zones), [#58](https://github.com/tangency-labs/pcbcraft/issues/58) (DRC), [#59](https://github.com/tangency-labs/pcbcraft/issues/59) (fabrication outputs),
[#77](https://github.com/tangency-labs/pcbcraft/issues/77) (net classes and rules) and [#95](https://github.com/tangency-labs/pcbcraft/issues/95) (two-layer track and via editing).

This page follows one board from placed footprints to fabrication files. It
records what each step does today and what is still missing. For each gap it gives
the behavior, model, commands and acceptance cases a bounded slice needs. It
builds on the [PCB representation](pcb-representation.md),
[net identity](net-identity.md) and [board workspace](board-workspace-plan.md)
designs and does not restate them. Behavior references are the published KiCad
[PCB Editor manual](https://docs.kicad.org/9.0/en/pcbnew/pcbnew.html) and Ucamco's
public [Gerber format specification](https://www.ucamco.com/en/gerber). No KiCad
implementation source or other GPL EDA source is an input.

## The workflow today

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
| Route | Tracks with an explicit width and 45° posture. Snapping to pads, track ends and vias. Through vias with V or PgUp/PgDn mid-route, plus standalone vias | Layout › Route |
| Modify | Click, box and U (connected) selection. Selection filters for tracks and vias. M/drag moves, D segment drags, width and via size in Properties, per-track/via locks, Delete | Layout › Modify, Properties |
| Inspect | Net highlight (`), ruler (Ctrl+Shift+M), DRC with a results card and canvas markers, board summary when nothing is selected | Analyze, Properties |
| Output | Gerber X2 copper, mask, paste, silkscreen and outline layers, plus an Excellon drill file in one archive | Manufacture › Fabrication › Outputs |
| View | Mirrored bottom view, layer visibility and colors | View › Navigate › Flip view, Layers |

Copper connectivity is derived from overlap on shared layers. Track edits and
footprint transforms refuse new shorts in the copper they touch. DRC separately
reports existing islands that contain conflicting intended nets, including direct
pad contact and via-mediated connections. It uses accepted board connectivity when
available and otherwise the project-wide logical graph; names are display text,
not the identity used to compare memberships. DRC checks the board without editing
it. A `pcb.check_design` request can override the fixed defaults for that run. Short
findings include sorted copper references and the involved component-terminal keys,
grouped by snapshot-local net index. Those indices must not be used as saved bindings.

## Gaps, in recommended order

Each slice below can land on its own. The order puts first what unblocks the
most everyday work and needs the fewest format decisions.

### 1. Footprint moves carry their copper

**Status.** `pcb.set_footprint_transform` and `pcb.align_footprints` already carry
attached track ends and run a scoped short check. The broader interior-contact,
branch and fixed-via preservation cases below remain requirements to verify as
routing develops; this section is not a claim that every case is implemented.

**Behavior.** While a footprint moves, rotates or flips, each track end on one of
its pads follows the pad, and the rest of that track stretches. Vias stay put.
Preserve contacts where a pad touches the interior of a track, where a branch
joins the interior of a stretched track, or where a pad overlaps a fixed via.
Refuse the edit if it would break any of those contacts.
The edit is refused, like a track edit, if the moved pads or stretched tracks
would join two nets. One undo step covers the placement and the stretched
tracks. If a flip changes a pad's copper layer, the attached track end is
released rather than moved onto the other layer, and its airwire returns.

**Model and commands.** No format change. The transform commands gain the
stretch and the scoped short check, applied to the moved footprint's pads and
the affected tracks. The placement preview shows the stretched tracks, the same
way D segment drags show them now.

**Acceptance.**
- Move and rotate a footprint with one routed pad: the track stays attached.
- Move a pad that touches the middle of a track: the contact is preserved, or the
  edit is refused with the board unchanged.
- Stretch a track with a branch attached between its endpoints: the branch
  stays connected, or the edit is refused with the board unchanged.
- Move a pad with a via in it and no track end on the pad: the fixed via stays
  connected, or the edit is refused with the board unchanged.
- Move a footprint onto another net's track: refused, with the board unchanged.
- Flip a routed SMD footprint: the end detaches and the airwire returns.
- Align three routed footprints: one undo step.
- Undo and reopen leave identical documents.

### 2. Board design rules and net classes

Part of #77 and #58.

**Problem.** DRC defaults are not stored, and the route and via tools keep their
own sizes per view. The [rules section](pcb-representation.md#rules-and-net-classes)
already describes the full resolver. This slice is the smallest part of it that
the layout tools can use.

**Behavior.** **Inspect › Rules** (now planned) opens board rules:
- Minimum clearance, track width, annular ring, via drill and edge clearance.
- Preferred track width and via size, which the route and via tools start from.

DRC reads these rules, and a `pcb.check_design` override still applies only to
that one request. Net classes come in the second half of the slice, after the
net registry. A class sets a preferred width and clearance for its nets. Routing
a classified net starts at that width, and DRC resolves clearance between two
classes by taking the larger.

**Model and commands.**
- **First half:** board-scoped rule records in `board.json` under the next
  board format. A board without rules keeps its current encoding. `pcb.set_rules`
  is revision-checked, and inspection reports the effective values with the
  contributing rule IDs.
- **Second half:** classes bind to persistent net IDs from the registry (see
  [net identity](net-identity.md)). They never bind to net names.

**Decision needed.** Should the first half also add "minimum" and "preferred"
fields for each class? That would let the class slice need no further format
change. The proposal is to wait: one board-default record per property is
enough for the layout tools today.

**Acceptance.**
- Rules round-trip through save and reopen.
- A rule change marks the DRC result out of date.
- New routes start at the preferred width.
- A track narrower than the minimum is a DRC finding.
- A class conflict reports the resolution it chose.

### 3. Clearance while routing

**Problem.** Routing refuses shorts but not near misses. Clearance problems show
up only when DRC runs.

**Behavior.**
- While routing, the active segment shows a clearance halo at the resolved
  clearance.
- A segment that would violate clearance against other-net copper stops at the
  last legal point. Clicking there commits the route up to that point.
- A setting allows clearance violations while copper on different nets stays
  physically separate. DRC reports those violations, and the router still
  refuses shorts.

**Model and commands.** No format change. The engine exposes one clearance query
that DRC, the router and later the zone fill all use. It reuses DRC's segment
distance and adds a spatial index, so the query stays fast on larger boards.

**Acceptance.**
- A route toward another net's pad stops at exactly the clearance.
- The same route in "allow" mode commits and DRC reports it.
- The router and DRC agree on every finding.

### 4. Copper zones

This is #57.

**Problem.** **Place › Zone** and **Design › Refill zones** are planned. Ground
planes and power pours are not possible.

**Behavior.**
- Drawing a zone works like a Board Outline rectangle or polygon on a copper
  layer, with a net chosen from the board's nets.
- The fill clears other-net copper by the resolved clearance and stays inside the
  Board Outline by the edge clearance.
- The fill connects same-net pads with thermal spokes or a solid connection.
  Islands smaller than a minimum area are dropped.
- Overlapping zones fill by priority.
- A refill runs after edits, or on demand with **Refill zones**. A changed fill
  updates connectivity and the ratsnest.

**Model and commands.**
- **Authored record:** the zone (outline, layer, net, priority, clearance
  override, thermal settings, minimum island area) goes in `board.json` under a
  new board format.
- **Derived fill:** the filled polygons are a revision-keyed result. They are not
  authored data, so the rules design's "store intent, cache results" contract
  applies. Outputs and DRC compute or reuse the fill for the current revision, so
  a saved board never carries a stale pour.
- **Net target:** a zone needs a persistent net target, because a pour has no
  pads of its own to derive one from. The zone slice therefore promotes the
  chosen net through the [net registry](net-identity.md) and stores the
  registry ID. The registry must land first.

**Decision needed: Gerber output.** There are two ways to write a pour:
- **Contours:** write each fill as dark regions with holes already cut, as exact
  polygon outlines. Every reader sees the same geometry, DRC checks the same
  polygons the file contains, and the Gerber stays simple to verify.
- **Clear polarity:** write the zone as one dark region followed by clear-polarity
  clearances (the specification's LPC command). The fill code is simpler, but
  every later dark object must be ordered after the clearances. The written
  image then depends on object order, which is harder to check against DRC.

The proposal is contours, computed by a qualified polygon-boolean library chosen
under the dependency rules in `deny.toml`. The candidate needs its own source and
license review before it is added.

**Acceptance.**
- A ground zone around two pads of another net leaves exact clearances.
- Thermal spokes reach same-net pads.
- An island below the minimum area is dropped.
- Two zones fill by priority.
- Refill after a track move changes the fill and the ratsnest.
- Export contains only dark contour regions, and DRC of the exported fill
  matches DRC of the board.
- Save, undo and reopen keep the authored zone without saving the fill.

### 5. Push-and-shove

The interactive-routing part of #56, after #95's basic editing. It comes after clearance-aware routing because it
needs the same clearance query and spatial index.

- **Stage 1, walk around:** the route bends around other-net copper at
  clearance, keeping 45° postures.
- **Stage 2, shove:** the route pushes other-net tracks aside, which keep their
  connections. If a pushed track can't move legally, the route stops.
- **Stage 3, vias:** vias are shoved too, and D segment drags shove their
  neighbors.

Every stage commits as one undo step through the existing track commands, so it
needs no format change. Acceptance for each stage compares the result against
DRC: no new violations, and connectivity is unchanged except for the routed net.

### 6. Later slices

These need their own design before implementation. They are listed so the
ribbon's planned entries and the editor guide have a stated destination.

- **Differential pairs and length tuning** (Layout › Route › Pair, Layout › Route › Tune) need
  net-registry pairs and length rules from slice 2. Pairs reference two net IDs,
  never name suffixes.
- **Arcs and teardrops** need a curved track segment in the board format. DRC,
  the clearance query and Gerber output (arcs are native in the specification)
  must support it in the same slice.
- **Blind, buried and micro vias** need layer spans checked against the stackup
  and drill pairs in the Excellon output.
- **Board text** on copper and silkscreen needs independently
  authored text records with physical font metrics from the silkscreen settings.
- **Locking** for tracks and vias is implemented under #249 through per-object
  flags and the shared snapshot publication guard. Footprint/zone locks and an
  optional locked-object selection filter remain broader #81 work.
- **Assembly outputs (rest of #59):** placement (centroid) files and a bill of materials,
  added as more files in **Manufacture › Fabrication › Outputs** from the board's footprints and
  sides.
- **Pad-level selection:** selecting and cross-probing one pad and its net
  between the schematic and the board.

Copy and paste of board objects is in progress separately in #197 through #202.

## Fixed tracks and vias

Select tracks and vias and set **Lock** in Properties to **Locked** or
**Unlocked**. A mixed selection displays **Mixed**. Locked copper remains
selectable and inspectable, while its size controls are read-only. The shared
`pcb.set_copper_locked` command requires an explicit board UUID, expected revision,
`locked` boolean and at least one track/via UUID. One atomic edit changes only
lock flags; no-op requests leave revision/history unchanged.

An edit fails as a whole if it would change or delete any existing locked track
or via. This includes positions, track ends/layers/widths, via diameters/drills,
segment drags that stretch a locked neighbor, footprint moves/rotations/alignment
that carry locked track ends, and mixed-selection Delete or Cut. An unchanged
locked object does not block independent unlocked edits or genuine no-ops.
Lock flags preserve authored copper records; they do not freeze the board's
setup, schematic net intent or footprint contacts when no copper record changes.
In particular, an existing flip that releases a stationary track end keeps that
track's fixed geometry. Locking/unlocking alone changes no copper connectivity.

Copy, duplicate and paste preserve lock flags while assigning new identities
and the requested new positions to copies. Existing source objects remain
protected. Cut refuses a selection containing locked copper. Native folder and
archive saves preserve locks; board format 18 and board-kind clipboard format 3
are used only while locks are present. Older readers refuse these extensions.
Unlocked boards/fragments keep their earlier encodings. Unlocking the last object
returns to the format needed by the remaining authored data. Inspection exposes
an explicit `locked` boolean for every track and via, and typed board snapshots
carry it for later walkaround/shove consumers. No routing solver is added here.

## Format changes at a glance

| Slice | Board format | Stored |
| --- | --- | --- |
| 1. Footprint moves carry copper | none | — |
| 2. Board rules | next | Board-scoped rule records |
| 2. Net classes | with the net registry | Class records bound to net IDs |
| 3. Clearance while routing | none | — |
| 4. Zones | next after rules | Zone records; fills are not stored |
| 5. Push-and-shove | none | — |

Each format step follows the existing ladder: a superset of the previous format,
empty collections refused, older readers rejecting the new format, and
byte-identical round trips.
