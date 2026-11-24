# PCB layout design plan

## Gaps, in recommended order

Each slice below can land on its own. See the [PCB layout workflow](../pcb-layout-workflow.md)
for the current editing steps and the [capability inventory](../capabilities.md)
for implementation status. Behavior references are the published KiCad
[PCB Editor manual](https://docs.kicad.org/9.0/en/pcbnew/pcbnew.html) and Ucamco's
public [Gerber format specification](https://www.ucamco.com/en/gerber). No KiCad
implementation source or other GPL EDA source is an input.

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

The board-default half is implemented by #242; see [board-rules.md](../pcb/board-rules.md)
for the fields, precedence, provenance and format. **Analyze → Rules** edits hard
clearance, track/via/drill, annular-ring and board-edge limits separately from
preferred routing dimensions. DRC and captured clearance queries use the shared
resolver. Omitted command dimensions and GUI routing sizes use those preferences;
explicit local tool sizes remain a per-view choice. Changing rules invalidates
previous checks and unfinished route authority without resizing existing copper.

`pcb.set_rules` requires the board ID and revision and creates one undo step.
Authored defaults have a stable rule UUID and require at least board format 19.
Zone outlines require at least 20, a placement origin 21, assembly profile adoptions
22, and the board setup's copper or board thickness fields 23. The format required
by the remaining data takes precedence, including explicit via assignments (24),
thermal or insulation material reports (25), circular copper arcs (26) and footprint
locks (34); see the [native storage rules](../formats/native-project-storage.md#assembly-profiles-and-stack-fields).
Removing the record restores built-in resolution and the format required by the
remaining board data. Omitting rules does not itself require a new board format.
Unchanged canonical codec bytes are stable; host Open and Save may apply the
existing [storage migrations](../formats/native-project-storage.md). Request-local DRC
limits may tighten authored constraints without modifying the saved record.

[Class definitions and dimensional resolution](../formats/net-classes.md),
[persistent assignments](../formats/net-class-assignments.md), and
[class-aware routing, clearance queries and DRC](../formats/net-class-consumers.md) are
implemented through shared commands and live/headless MCP (#300–#302). Classes
bind to persistent net IDs rather than names and extend the same effective-rule
resolver. Preferred values remain distinct from minimum/maximum constraints;
clearance between classes takes the larger supported requirement. Assignments
retain their origins through supported split/merge reconciliation, and conflicting
limits remain explicit rather than being resolved by name or ordering.

Routing uses effective class preferences when dimensions are omitted. Changed
classified tracks must meet minimum width; changed classified vias must meet
diameter, drill and annular-ring limits. Routing refuses shorts but does not yet
enforce class clearance; that enforcement remains future work in section 3.
Queries and DRC use per-object rules with provenance, including class clearance;
existing copper is not resized. Editor forms remain under
[#303](https://github.com/tangency-labs/pcbcraft/issues/303), while advanced
electrical/manufacturing analyses and the other rule extensions remain under #77.

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

[Exact circular copper records](../pcb/circular-copper-records.md) are a storage-only
extension. Boards containing arcs refuse these geometry operations rather than
substituting straight chords; curved routing and consumers remain separate work.

**Model and commands.** The clearance-query slice itself makes no format change.
The engine exposes one clearance query
for DRC, the router and later zone fill. The
[snapshot query](../pcb/copper-clearance-query.md) now shares the supported contours,
separates conservative candidates from final predicates and retains source
snapshot/revision/rule authority. Its simple sweep/linear scan and exhaustive
reference precede the separately benchmarked spatial index in #244. Routing
clearance enforcement and gesture integration remain later work.

**Acceptance.**
- A route toward another net's pad stops at exactly the clearance.
- The same route in "allow" mode commits and DRC reports it.
- The router and DRC agree on every finding.

### 4. Copper zones

This is #57.

**Implemented authoring and fill.** [Copper zones](../pcb/copper-zone-outlines.md)
(#231/#232) draws and edits single-layer, net-addressed outlines and derives their
conductive material. Automatic and explicit **Pour copper**, thermal connections,
priority, island filtering, connectivity/ratsnest and zone DRC share that result.
Fabrication output remains #233; advanced policy remains #304.

**Behavior.**
- Drawing a zone works like a Board Outline rectangle or polygon on a copper
  layer, with a net chosen from the board's nets.
- The fill clears other-net copper by the resolved clearance and stays inside the
  Board Outline by the edge clearance.
- The fill connects same-net pads with thermal spokes or a solid connection.
  Islands smaller than a minimum area are dropped.
- Overlapping zones fill by priority.
- A refill runs after edits, or on demand with **Pour copper**. After **Remove
  pours**, edits leave automatic refill suppressed until **Pour copper** resumes
  it. A changed fill updates connectivity and the ratsnest.

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

The current [route and connected-drag guide](../pcb/route-planning.md#through-vias-and-connected-segment-drags) describes the delivered stages and their limits. The stages below record the original plan.

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
- **Circular tracks and teardrops:** native [circular track records](../pcb/circular-copper-records.md)
  and [straight via teardrop intents](../pcb/teardrop-records.md) are available through
  shared commands. Rendering and manufacturing support, DRC and automatic
  teardrop regeneration remain separate work.
- **Blind, buried and micro vias** need layer spans checked against the stackup
  and drill pairs in the Excellon output.
- **Board text** on copper and silkscreen needs independently
  authored text records with physical font metrics from the silkscreen settings.
- **Locking** for tracks and vias is implemented under #249 through per-object
  flags and the shared snapshot publication guard. Existing [footprint locks](../pcb/locks.md#placed-footprint-locks)
  protect direct placement edits. To skip locked footprints, tracks
  and vias when selecting, clear **Locked objects** in the view's **Selection
  filters** card. It is checked by default. Clearing it also drops those objects
  from the current selection, but leaves them visible and listed in project
  inspection. Footprint lock authoring and locks for zones and board graphics
  are not available yet.
- **Assembly outputs (rest of #59):** placement (centroid) files and a bill of materials,
  added as more files in **Manufacture › Fabrication › Outputs** from the board's footprints and
  sides.
- **Pad-level selection:** selecting and cross-probing one pad and its net
  between the schematic and the board.

Copy and paste of board objects is in progress separately in #197 through #202.
