# RFC: Parametric board

| | |
|---|---|
| Status | Draft, open for comment. Unnumbered (see "Name" below) |
| Started | 2026-11-19 |
| Builds on | [SolidCraft RFC 0001: Craft bridge](https://github.com/tangency-labs/solidcraft/blob/a23b231a600eb606ddfa4be4deecb4f4f719196d/docs/rfcs/0001-craft-bridge.md) (the revision [#70](https://github.com/tangency-labs/pcbcraft/issues/70) adopted; unchanged on SolidCraft main at `7f34f1f`) and #70's PCBCraft-specific decisions |
| Issues | [#153](https://github.com/tangency-labs/pcbcraft/issues/153) (linked outline), [#70](https://github.com/tangency-labs/pcbcraft/issues/70) (Craft bridge integration), [#259](https://github.com/tangency-labs/pcbcraft/issues/259) and [#260](https://github.com/tangency-labs/pcbcraft/issues/260) (arcs and curved outlines), [#80](https://github.com/tangency-labs/pcbcraft/issues/80) (dimensions), [#86](https://github.com/tangency-labs/pcbcraft/issues/86) (3D, STEP and the model record), [#246](https://github.com/tangency-labs/pcbcraft/issues/246)/[#247](https://github.com/tangency-labs/pcbcraft/issues/247) (walkaround and shove) |
| Discussion | the pull request that adds this file, then one issue per slice |

Nothing here is built. This proposes the shape of the feature and the order of
work, so that the outline, placement, routing, zone, 3D and bridge work that
touches it fits from the start. The first two slices give SolidCraft a linked
board that stays up to date as the board changes in PCBCraft; the parametric
slices build on them. Every command, record and field name below is a proposal
for review.

**Name.** PCBCraft has no `docs/RFC/` index yet, and in Craft discussions
"RFC 0001" already means SolidCraft's Craft bridge, which #70 and this document
cite throughout. Calling this file "RFC 0001" as well would make every sentence
that mentions both ambiguous. It is therefore unnumbered; the maintainers can
assign a number when they start an index (question 15). In this document,
**"the bridge RFC"** always means SolidCraft RFC 0001.

## 1. What the user sees at the end

Two things, in this order.

**First, the board in the enclosure, kept up to date.** The designer sends a board
to SolidCraft and places it in the enclosure assembly. From then on, each saved
change in PCBCraft to the outline, cutouts, holes, thickness or part placement
shows up in the same open assembly: the same linked board, in the same place, with
its joints, and nothing duplicated. A reference the enclosure made to something
that is gone is named, never moved to something else. Publishing is a command,
so an agent can do it over MCP too. Section 6.7 describes it; slices 1 and 2
deliver it. It needs no parametric work.

**Then, a board that follows the enclosure:** a board whose shape can come from
the enclosure, whose key positions are named values, and whose parts stay where
the designer meant them to be when either changes.

- **The outline from the enclosure.** In SolidCraft, the designer sketches the
  board pocket of the enclosure. "Send to PCBCraft" writes a `.craftx` exchange
  file (the bridge RFC, section 2). PCBCraft links the board outline, cutouts and
  mounting points to it. When the enclosure changes, PCBCraft shows what would
  move and the designer accepts or rejects it. To change the shape, the designer
  uses **Edit in SolidCraft** (the bridge RFC, section 4): the shape is solved
  where it is owned.
- **Or drawn here, or imported once.** A board without SolidCraft keeps today's
  drawn Board Outline. A one-way DXF import (#153's suggested first step) gives
  drawn geometry too.
- **Board parameters.** Board Setup gains a Parameters table: `usb_y = 18 mm`,
  `edge_gap = 2 mm`, `led_pitch = 5 mm`, typed lengths and angles entered with the
  [shared unit parser](../quantity-units.md), optionally as expressions over each
  other. Parameters the enclosure exposes (the bridge RFC's `params.json`, such as
  `wall = 3 mm`) appear read-only next to them.
- **Parts held to the board.** A footprint can be held to an outline edge, a
  mounting point or another footprint: "J1's origin `edge_gap` from the left edge,
  at `usb_y` along it, parallel to it", "D2 `led_pitch` right of D1". A held part
  shows that it is held and by what.
- **One update.** Accepting a new enclosure revision, or changing `usb_y`,
  moves the held parts, carries their tracks and vias, re-routes copper the carry
  left too close to something, and re-runs the board-edge checks. It is one undo
  step. If a copper connection would be lost, the whole change is refused with the
  nets named, and the board is unchanged.
- **The same from MCP.** An agent uses the same commands and gets the same
  result, the same preview and the same refusals, with a reply listing what moved.
- **The enclosure follows the board, too.** Labelled board datums ("USB edge",
  "J1 origin") travel in `board/1`, so a SolidCraft sketch can follow them, for
  example an opening that follows a connector.

A board without links, parameters or holds works exactly as it does today.

## 2. How this sits on the Craft bridge

The bridge RFC is the cross-application contract, and #70 records PCBCraft's
decisions on it. This RFC does not propose another exchange format, transport or
link model. It uses theirs and adds only what is specific to a parametric board.

**Taken from the bridge RFC, unchanged:**

- The exchange file `*.craftx` with a `craft-exchange/1` manifest, `source_id`,
  `revision`, content hash, units, `parts` and `requires` (sections 2 and 6).
- Transport A + D: the exchange file is the contract; launch/open hands it over;
  file watching (and optionally a control-channel notice) detects a new revision
  (section 3). MCP stays agent-facing, not the app-to-app path.
- The link record `{source_id, path hint, revision, content hash}`; the path is
  only a hint (section 4).
- Persistent face and edge names from SolidCraft, carried as opaque strings, with
  `exact` flags and aliases (section 2).
- Re-resolution outcomes: exact, merged, split, by geometry, missing (section 4).
- One owner per item; the other side holds a read-only linked copy; Break Link
  turns it into its own editable copy; "Edit in" the owner app to change it
  (section 4).
- Millimetres in the source design's world frame; the receiver places what it
  receives (section 6).

**PCBCraft's decisions in #70 that this RFC follows:** an enclosure-driven
outline or mounting point remains SolidCraft-owned source geometry; accepting its
effect changes PCBCraft-owned board data through a revision-checked native
transaction, with a preview of affected outline, holes, placements, attached
routing and derived checks; stale or invalid proposals are rejected atomically;
refreshing a read-only linked representation is distinct from accepting changes
to authored PCB data, and the latter always gets explicit review; a split or
approximate match never silently retargets a hole, connector or constraint;
identity is never inferred from names, paths or vertex order.

**Ownership, extending the bridge RFC's table for this feature:**

| Item | Owner | The other side gets |
|---|---|---|
| Enclosure, board pocket sketch, mounting points, exposed parameters | SolidCraft | PCBCraft: a read-only linked copy, accepted into the board through a preview |
| Accepted board outline, cutouts, holes and finished thickness | PCBCraft | SolidCraft: the board body and `board/1`, refreshed in place (6.7) |
| Board parameters and holds | PCBCraft | SolidCraft: nothing directly; their effect arrives as placements in `board/1` |
| Placements, copper, zones, stackup | PCBCraft | SolidCraft: placements and component model bodies in `board/1`; copper and keep-outs later (6.7) |
| Board datums (board origin, holes, part origins; labelled edges and points later) | PCBCraft | SolidCraft: named entries in `board/1`, so its joints and sketches can follow them |
| The linked board's placement in the assembly, its joints, and enclosure features that reference it | SolidCraft | PCBCraft: nothing |

**What the bridge RFC does not define yet, and this feature needs.** The bridge
RFC's parts describe 3D bodies with named faces and edges, parameters, metadata,
and the outbound `board.json`. Nothing yet describes the inbound **2D profile**: a
sketch's solved loops in a stated plane, with a persistent name per curve and per
marked point. PCBCraft could derive it from a body (the loop of a planar pocket
face, its edges named `<face A>|<face B>`), but that needs a B-rep reader and
plane sectioning in PCBCraft and depends on the edge-naming details still open in
[SolidCraft #17233](https://github.com/tangency-labs/solidcraft/issues/17233)
(point 3). The smaller option is a `profile/1` capability written by SolidCraft:
lines and arcs with their sketch entity IDs, points marked as datums, the
sketch's plane and placement. This RFC proposes that, **as an amendment to be
agreed in #70 and the bridge RFC's Phase 0 / Phase 5 discussion, not as a
PCBCraft-only format** (question 1).

PCBCraft reads only the exchange file, not SolidCraft's `.sc` files.
[SolidCraft #18936](https://github.com/tangency-labs/solidcraft/issues/18936)
lets CamCraft's phase 1 read the `.sc` file directly, which suits a body; a
board outline needs exactly what the manifest and name tables add: identity,
revision and frame.

## 3. Non-goals

- A sketch solver or SolidCore inside PCBCraft's engine (section 6.1).
- Another exchange format, transport, link model or outcome vocabulary.
- Parametric shape editing inside PCBCraft. The shape is edited where it is
  owned: drawn here, or in SolidCraft for a linked outline.
- Driving SolidCraft parameters from PCBCraft. The bridge RFC mentions it as
  later work; it is out of scope here.
- Silent or automatic acceptance of a linked enclosure revision into PCBCraft's
  board data. Refreshing SolidCraft's read-only copy of the board is a different
  thing and may be automatic (section 6.7).
- Building SolidCraft's receiver. Section 6.7 proposes its behaviour for
  agreement in the bridge RFC's Phase 5; SolidCraft decides and builds it.
- Parametric copper. Tracks and vias are never hold participants; they follow the
  parts they connect.
- Holds to faces of 3D component models (depends on #86's model record).
- Porting another implementation (section 10), or anything from a SolidCraft fork.
  Every SolidCraft dependency named here is upstream.

## 4. Where PCBCraft is today

All on main:

| Area | Today | Relevance |
|---|---|---|
| Board outline | Board Outline drawings: open/closed point chains (`pcb.add_drawing`, `pcb.set_drawing`), one UUID per drawing, vertices without IDs ([identity inventory](../object-identity.md)) | An edge has no stable identity yet; this is the first gap |
| Curves | Circular copper arc records stored ([#259](../circular-copper-records.md)); consumers refuse them until supported. Curved outlines are #260 | Arcs in a linked or drawn outline wait for this contract |
| Footprints | Counter IDs (`footprint-N`), pads as owner tokens; X/Y in nm, **rotation in whole degrees (0–359)**, side | Holds must respect whole-degree rotation (question 6) |
| Copper carry | Moves carry attached track ends and refuse new shorts in one undo step ([layout workflow, slice 1](../pcb-layout-workflow.md#1-footprint-moves-carry-their-copper)); locked copper refuses | The base of the board update |
| Routing | Deterministic, bounded walkaround with `reached`/`blocked`/`incomplete`/`canceled`/`stale` outcomes ([route planning](../route-planning.md)); shove next (#247) | Re-routing inside an update needs these properties |
| Checks | DRC with outline containment and edge clearance; [board rules](../board-rules.md) with sources | The update's guard uses the same predicates |
| Zones | [Zone outlines](../copper-zone-outlines.md) stored; fill in progress (#232, draft PR #395) | A zone can later follow the outline |
| Origin | `pcb.set_placement_origin` with an `outline_lower_left` anchor that follows the outline | A hold to the outline, in miniature |
| Parameters | Text parameters (`@project.Name`) for sheets | Not numeric (question 5) |
| Edits | Revision-checked commands, `dry_run` on some, one undo step, preview/apply with a plan token ([single-board update](../single-board-update.md#preview-and-apply)) | The update and the linked preview follow these patterns |
| 3D and STEP | `crates/geometry` reads and writes STEP; the board solid and the model record are claimed or proposed in #86; no `.craftx` writer | What slice 1 builds on |
| Kernel | `crates/geometry` is the only crate linking SolidCore; the engine is kernel-free; the browser leaves the kernel off ([geometry kernel](../geometry-kernel.md#boundary)) | Why shape solving belongs in SolidCraft (section 6.1) |

## 5. Design constraints

Each comes with its reason; most were learned by building it another way first
(section 10).

**C1. Geometry is solved by its owner; PCBCraft evaluates holds.** A linked
outline arrives solved, with names. PCBCraft never re-solves another app's
sketch. Its own holds are one relation model with one evaluator, for every kind of
reference (drawn edge, linked edge, linked point, footprint feature). *Reason:*
two solvers for one shape disagree; and a special-purpose "offset from edge"
record next to a general relation grows its own anchors and alignment modes, then
has to be migrated once a part must be held to another part or to an arc. We did
that once.

**C2. References name stable identities, never names, positions or order.** A
drawn edge by its segment ID, a linked edge or point by its persistent name under
its link's `source_id`, a part by its footprint ID, a pad by its owner token.
Designer-given labels ("USB edge") are display text. *Reason:* PCBCraft's own
rule ([AGENTS.md](../../AGENTS.md#extension-and-cross-application-boundaries)) and
#70's; vertex order changes on every structural edit, and a hold that retargets
to a different edge moves a connector to the wrong side of the board.

**C3. Holds measure to an edge's line and its virtual corners.** A distance from
an edge is measured to the infinite line through it. A position along an edge is
measured from a **virtual corner**: the intersection of the edge's line with the
line of another straight edge, not the segment's end. The hold stores both edges
by identity (C2), so the corner is the same point however the outline between them
changes. When a designer picks an end of an edge, the other edge is found by
walking the outline from that end past arcs and past straight segments collinear
with the picked edge, to the first straight edge that is not parallel to it:

- a neighbour that is an arc (a rounded corner) is passed over, so the corner is
  where the two straight edges would meet without the rounding;
- a collinear neighbour (a straight edge split by an extra vertex) is passed
  over, so splitting an edge does not move the origin;
- if the walk reaches only parallel edges or arcs (a slot with semicircular ends,
  a circular board), there is no virtual corner at that end, and the hold must
  use a point datum instead.

If an edit later makes the two stored edges parallel, or removes one of them, the
corner no longer resolves and the hold stops driving (C4). *Reason:* otherwise
rounding a corner in the enclosure, or inserting a vertex, moves every part
dimensioned along that edge, which nobody expects.

**C4. A reference that no longer resolves stops driving, keeps its last
geometry, and says so.** Mapped to the bridge RFC's outcomes in section 6.6.
The finding names the edge or point in the user's words and blocks fabrication
output and publishing `board/1` until it is resolved. *Reason:* deleting the hold
or snapping to "the nearest" edge loses design intent without telling anyone.

**C5. Free degrees of freedom keep their values.** A part held only in X keeps its
Y and rotation. A distance never rotates a part; only an angle or parallel hold
does. *Reason:* evaluation must never move what nobody asked to move.

**C6. The update is one atomic transaction that carries copper.** Accepting a
linked revision, editing a parameter or a hold, or redrawing a drawn edge runs:
resolve references → evaluate parameters → evaluate holds → carry copper →
re-route offending carried copper → guard → commit. One revision, one undo step,
one reply. *Reason:* moving parts and leaving their tracks behind gives a board
full of clearance errors and dangling copper to clean up by hand.

**C7. Refuse rather than silently break copper.** After carry and re-route,
compare copper connectivity before and after: pads, tracks, vias and, once zone
fill exists (#232), the fill of every zone the update affects, recomputed in the
same snapshot (section 6.8). A lost connection, a new short, a dangling track or
via, or locked copper that would have to move refuses the whole change, naming
the nets and the stage. Today zones have no fill, so no connection can depend on
one; fill joins the comparison in the same change that makes fill part of
connectivity. *Reason:* an update that quietly turns routed nets back into
airwires would only show up later, in DRC's unrouted-connections check or at the
fab, which is worse than a refusal.

**C8. Routing inside an update is deterministic and bounded.** The same board and
the same change give the same copper, within a fixed work budget, with an explicit
outcome. *Reason:* undo/redo, MCP retries and a preview that must match its apply
all replay the update. Walkaround has these properties today; shove (#247) should
keep them.

**C9. One resolved board for every consumer.** DRC, Gerber, zone fill, canvas,
picking, the 3D view and STEP (#86) and `board/1` export read the same accepted
outline and copper through `BoardView`. Selection and camera never re-evaluate.
*Reason:* two derivations drift; a 3D board that disagrees with the Gerber is a
mechanical failure found late.

**C10. Store inputs and results.** The file keeps the accepted outline (with its
source names and the accepted revision), parameters, holds, and the placements
they produced. Opening, rendering, DRC and outputs never need the source design,
SolidCraft or a re-evaluation. *Reason:* PCBCraft must work without SolidCraft
(#70), no-op saves must stay byte-identical, and a moved part should show as one
changed line in Git. The bridge RFC's "keeps the old path for viewing" and
SolidCraft RFC 0002's recorded fallbacks follow the same principle.

**C11. No invented limits.** Every limit the update checks (edge clearance,
copper clearance, widths, the tolerance for matching linked geometry) comes from
the board's [effective rules](../board-rules.md) with sources, or a
[fabrication profile](../pcb-representation.md#project-settings-and-fabrication-profiles)
once those exist. *Reason:* a hidden constant in the carry that differs from DRC
gives updates that pass and then fail DRC.

**C12. Preview before accept for anything linked.** A local parameter or hold
edit applies directly, with `dry_run`, because undo covers it. A linked revision
always goes through a preview bound to the accepted revision and content hash on
the source side and the board's revision on PCBCraft's side, like
`pcb.preview_update`/`pcb.apply_update`. *Reason:* #70 and AGENTS.md: authored
updates are reviewed, never silent.

**C13. Measured budgets, labelled as targets until measured** with the
[#240 harness](../performance-testing.md) on original fixtures (section 8).

**C14. Agents see what the change did.** The reply lists references resolved and
their outcomes, parameters evaluated, parts moved (old and new pose), copper
carried and re-routed per net, and holds that stopped driving. *Reason:* without
it, an agent diffs the whole board to learn what its edit did.

## 6. Foundations

### 6.1 Where solving happens

**The shape.** A linked outline is solved in SolidCraft, by SolidCraft's own
sketch solver, and arrives as solved curves with persistent names. A drawn outline
is authored geometry and needs no solver. A DXF import is fixed geometry. In
none of these does PCBCraft solve a sketch.

**The holds.** The hold kinds in section 6.4 are chosen so that each part's pose
is computed directly from geometry that is already known: distance from a line,
position along a line from a point, parallel or at an angle to a line, offset
from another part's feature. Holds form a directed graph (a part held to a part
held to an edge); PCBCraft evaluates it in dependency order, refuses cycles and
over-determined degrees of freedom, and needs no iterative solver and no SolidCore.
Evaluation uses integer nanometres, converting to floating point only for
non-axis-aligned lines and rounding once.

**This removes the main obstacle of an in-app solver.** SolidCraft's
`solidcraft-sketch` depends, through `solidcraft-geom`, on `solidcore-ops` and
`solidcore-tess`, and reads system fonts. PCBCraft's engine is kernel-free and
the browser build leaves the kernel off because kernel panics abort on wasm32.
With shape solving in SolidCraft and closed-form holds in the engine, nothing in
the slices below needs either, and the browser can edit holds and parameters.

**When PCBCraft would need a solver.** Only for (a) holds that must be solved
simultaneously (a part held between two other parts that also depend on it), or
(b) parametric shape editing inside PCBCraft without SolidCraft. Neither is
proposed here. If maintainers later want either, the smallest option is the same
upstream crate behind a host-provided engine trait, implemented next to
`crates/geometry` (native first), with an upstream request to SolidCraft to make
the solver usable without the kernel and fonts. The hold kinds map one-to-one to
SolidCraft sketch constraints, so stored holds would not change (question 3).

### 6.2 Datums and stable edge identity

A **datum** is anything a hold or an external design may reference:

| Datum | Identity |
|---|---|
| Drawn outline edge or vertex | A new per-segment and per-vertex ID in the Board Outline drawing record (a format step, slice 3) |
| Linked outline edge or point | `(link id, persistent name)`, the name taken from the exchange file as an opaque string |
| Mounting hole center | Its footprint or hole record ID |
| Footprint feature | Footprint ID plus origin, X axis, a pad center (pad token) or a courtyard side |

Datums can be given labels for display ("USB edge"), and can be marked
**published** for `board/1` (section 6.7).

### 6.3 Board parameters

A board-local, ordered list `{ id, name, kind, expression, value }` with `kind`
`length` or `angle`. `name` is the display key used in expressions, unique per
board; `id` is a UUID so a rename keeps references. Expressions allow numbers with
units, `+ - * /`, parentheses and parameter names, with bounded depth and count;
cycles and unknown names are refused. The evaluated value is stored next to the
expression (C10). Linked parameters from `params.json` are read-only, addressed by
`(link id, parameter name)` as the exchange gives it, and usable in expressions.
They are never written back.

### 6.4 Holds

A hold is a board record `{ id, footprint, axis, kind, from, value }`:

- `axis`: which degree of freedom it fixes: `x`, `y` (in a frame given by the
  reference line or the board axes) or `rotation`.
- `kind`: `distance` (from a line, along its normal), `along` (from a virtual
  corner or point, along a line), `angle` (to a line; `parallel` is angle 0),
  `offset` (from another footprint's feature, in board or that part's axes).
- `from`: a datum (6.2); for `along`, a virtual corner `{edge, other}` (C3) or a
  point datum; for `offset`, a footprint feature.
- `value`: a literal or a parameter expression.

A footprint has at most one hold per degree of freedom; a second is refused,
naming the first. Unheld degrees of freedom keep their stored values (C5). The
same record with `driving: false` could serve as a measured dimension for #80
(question 7).

**Moving a held part** directly is limited to its free degrees of freedom: a part
held only in distance from an edge slides along it. A move that would change a
held value is refused with the way out ("J1 is held 2 mm from the USB edge; edit
or remove that hold to move it"). Locked footprints (#81) refuse holds that would
move them.

### 6.5 The board update

One engine operation behind every command that changes a hold, a parameter, a
drawn datum or an accepted linked revision:

1. Resolve references (drawn IDs; linked names with their outcomes).
2. Evaluate parameters.
3. Evaluate holds in dependency order, only for those whose inputs changed.
4. Carry copper for each footprint whose pose changed, with the existing carry,
   extended to carry a copper island rigidly when every pad it joins moves with
   the same motion.
5. Re-route carried copper that now violates clearance or edge clearance, one
   connection at a time in ID order, with the deterministic router (C8).
6. Guard (C7) and the scoped short check.
7. Commit as one revision and one undo step, or refuse with nothing changed.

Steps 1–6 are a pure function of the captured snapshot and the request, so
`dry_run` and a linked-revision preview return exactly what apply would commit.
Large updates can run as a bounded snapshot job (#243) with its stale and cancel
rules; apply rechecks the revision.

### 6.6 A linked outline over the bridge

**Link.** The board stores the bridge RFC's link `{source_id, path hint,
revision, content hash}` and which of the board's items it drives: the outer
outline, each cutout, each mounting point. It also stores the transform from the
source sketch's frame to board coordinates (#70: units and transforms) and the
inset (#153's "fit clearance" question 3, here a board-local value, question 8).

**New revision.** A changed content hash (by file watching, or a notice) marks the
link **changed**; nothing moves. **Refresh** updates only the read-only linked
representation shown as an overlay. **Preview** runs the update (6.5) against the
new geometry and lists outline, cutout and hole changes, moved parts, carried and
re-routed copper per net, re-resolved references and new DRC edge findings.
**Accept** commits the preview's result as one undo step; a stale preview (either
side's revision moved) is refused. Rejecting leaves the board on the accepted
revision and the link marked changed.

**Outcomes.** The bridge RFC's outcomes, with PCB meaning:

| Outcome | For a hold or outline item |
|---|---|
| exact | Follows |
| merged | Follows the face that answers to the name; the preview notes the merge |
| split | Never followed silently: the preview asks which piece, or the hold stops driving |
| by geometry | Shown as "check this" in the preview; accepted only by explicit confirmation |
| missing | Stops driving (C4), keeps its last geometry, named in the preview and as a finding blocking outputs |

A file whose `requires` lists a capability PCBCraft does not read is refused,
never read by geometry fallback (#17233, point 5). A missing source keeps the last
accepted geometry and reports the link as missing, with Relink. Break Link turns
the accepted outline into ordinary drawn geometry with new segment IDs; holds on
linked edges are remapped to them in the same step.

### 6.7 The board in a SolidCraft assembly

This is the outbound direction, PCBCraft → SolidCraft, and the first thing this
RFC proposes to build (slices 1 and 2). It needs none of the parametric work: an
ordinary drawn board is published and kept up to date. The parametric slices
later change only *why* a placement moved, not how the change travels.

**The workflow.**

1. In PCBCraft, the designer chooses **Send to SolidCraft** for a board.
   PCBCraft publishes revision 1 of an exchange file for that board and starts
   SolidCraft with its path (the bridge RFC, section 3, transport D). Or the
   designer inserts the file in SolidCraft with Insert Exchange
   (`file.insert_exchange`, the bridge RFC, section 5).
2. In the enclosure assembly, the board is one linked component, placed and
   jointed like any inserted design. Its placement and joints belong to the
   assembly.
3. Back in PCBCraft, the designer makes an edit the first milestone covers: moves
   an outline edge or a cutout, adds, moves or deletes a mounting hole, changes
   the finished thickness in Board Setup, or moves, rotates or flips a part.
   Saving publishes revision 2 to the same file.
4. SolidCraft sees the new file and refreshes the same linked component in place.
   The board and its parts change in the open assembly. The component, its entry
   in the Browser, its placement in the assembly and its joints stay. References
   into the board re-resolve by name; one that no longer resolves is named, never
   moved to something else.

**What the first milestone carries.**

| Item | In `board/1` and the exchange bodies |
|---|---|
| Board body | One solid: the closed outer outline extruded through the finished thickness, cutouts and holes as through-voids |
| Holes | Footprint pad holes and non-plated footprint holes, at the size and shape the footprint defines, each with its own name |
| Thickness | The declared finished board thickness from Board Setup ([board materials](../board-materials.md)). If none is declared, publishing is refused, naming the setting; no default stands in |
| Components | One entry per placed footprint: footprint ID, reference designator (display only), side, position, rotation, model reference and content hash, and the composed transform below. Its body is the model bound through #86's model record, written once per content hash |
| Datums | The board origin, the placement origin, the top and bottom faces, each hole's axis and each component's origin frame |

Not in the first milestone: copper, solder mask, silkscreen and paste; vias
(their drills matter electrically, not mechanically, and there can be thousands);
keep-outs and height zones (not stored yet); zone fill; outline arcs (#260). A
component without a usable model is listed with its state from #86 (no binding,
record missing, bytes unavailable, hash mismatch, unreadable) and gets no body.
No invented box or height stands in for a missing model (#70: missing data
cannot become a mechanical boundary).

**Names.** Every face and datum gets a persistent name in `faces.json` (the bridge
RFC, section 2), built from PCBCraft identities, never from order or position:

- `board:top`, `board:bottom`; `hole:<footprint>:<pad or hole>` for a hole's wall
  and axis; `component:<footprint>` for a component's occurrence and origin
  frame. These are `exact`.
- Outline and cutout side faces: until outline segments have IDs (slice 3), they
  are named from the drawing's UUID and the segment's position and published with
  `exact: false`, so SolidCraft treats a reference to them as found by geometry
  ("check this"). Slice 3 makes them exact.
- Faces inside a component model come from its STEP file, which has no
  persistent names. A reference to one is by geometry and is flagged whenever
  that model changes.

Footprint IDs are project counters today (question 11). The publication is per
board, where they are unique.

**Frames and units.** This is the shared contract #70 asks for ("Resolve units and
transforms") and #86's model record uses; it follows the model-frame proposal in
#86.

- Lengths are millimetres, converted once from PCBCraft's whole nanometres.
- **Board frame.** Origin at the board's coordinate zero. +X is board X; +Y is
  board −Y, because board Y points down in PCBCraft; +Z points away from the
  board on the top side. Z = 0 is the finished top outer surface, so the body
  spans −t to 0 for finished thickness t. The frame is fixed to board zero, not to
  the outline's lower-left corner or the placement origin, because those follow
  the outline: a frame that moved with the outline would make every refresh shift
  the whole board inside the enclosure. The placement origin is published as a
  datum instead.
- **Model frame** (#86). Origin at the footprint origin on the mounting surface;
  +X is footprint X; +Y is footprint −Y; +Z points away from the board on the
  component's side; Z = 0 is the model's seating plane.
- **Component pose** in the board frame: `T(x, −y, 0) · Rz(r) · S · A`, where
  `(x, y)` is the footprint position, `r` its rotation counterclockwise as seen
  from the top, `A` the binding's alignment (#86), and `S` is the identity on the
  top side and `T(0, 0, −t) · Ry(180°)` on the bottom. A bottom part is turned
  over, not mirrored; this matches PCBCraft's bottom placement, which mirrors
  local X before the rotation ([footprint schema](../footprint-schema.md)).
- `board/1` carries each composed transform as a 4×4 matrix in millimetres, and
  the inputs it came from (position, rotation, side, alignment, t). Receivers
  place with the matrix and need not repeat PCBCraft's rules; tests check the
  composition from the inputs.
- Where the board frame sits in the enclosure belongs to the receiving assembly
  (the bridge RFC, section 6).

**Publishing and revisions** (PCBCraft).

- Each board has its own export target: a path hint, auto-publish on or off, and
  the last published revision. It is stored with the board, in a
  board format step written only when a target exists. The default path is beside
  the project, named after the project and the board, and chosen once.
- `source_id` is `<project UUID>/<board UUID>` (question 18).
- Publishing captures one immutable snapshot of the **saved** board (question 16)
  and follows the [export snapshot rules](../multiboard-assembly-and-outputs.md).
  Publish with unsaved changes saves first. With auto-publish on, each save that
  changes the board publishes; a no-op save publishes nothing and changes no
  bytes.
- `revision` is the target's counter plus one, written in that same save. The
  manifest also carries the content hash and the snapshot's capture ID. Revisions
  only increase; a publish that fails leaves a gap, which is harmless.
- The file is written under a temporary name and renamed over the previous one, so
  a reader sees a whole revision or the previous one, never part of one.
  Publishing runs as a bounded job off the editor thread and never waits for
  SolidCraft.
- Publishing never changes authored board data. A refused publish (no thickness,
  a model whose bytes do not match their hash, a write error) leaves the previous
  file and says why; the save itself still succeeds.

**Refreshing in SolidCraft.** This is a proposal for SolidCraft's receiver, to be
agreed in the bridge RFC's Phase 5; SolidCraft decides and builds it. It reuses
Insert Design's link, Update Links and Break Link machinery, as the bridge RFC's
`file.insert_exchange` already says.

- Insert Exchange makes one linked component whose identity is the `source_id`.
  Each PCB component is a child occurrence keyed by `component:<footprint>`;
  occurrences with the same model hash share one body.
- A refresh updates that component in place. The board body is replaced.
  Occurrences are matched by key: kept and moved, added for new keys, removed for
  keys that are gone. Nothing is matched by order or reference designator, and a
  refresh never adds a second copy of the board or a part.
- The assembly's placement of the linked component and its joints are kept.
  SolidCraft-owned bodies, such as the enclosure, are never replaced or deleted by
  a refresh. They change only through SolidCraft's own features that reference
  the board, recomputed as for any linked design.
- References into the board (a joint to a hole, a sketch that projects a board
  edge, a measurement) re-resolve by name with the bridge RFC's outcomes. Exact and
  merged follow. Split and by-geometry matches are flagged "check this" and are not
  silently used for a joint or feature that would change meaning. Missing marks
  the joint or feature broken, naming the item in the user's words ("hole MH2 of
  J3 on board Main") and where it was; it keeps its last result. A broken
  reference is never moved to another hole or face, even one in the same place.
- A refresh is one undo step in SolidCraft, as Update Links is today, and its
  report lists what moved, what was added or removed, and what broke.

**When updates happen.**

- PCBCraft publishes on save when auto-publish is on, or on an explicit
  **Publish**.
- SolidCraft watches the linked file and, when the live session is used, also
  takes an `exchange.changed` notice (the bridge RFC, section 6). Either marks the
  link changed, as `component.links` reports it. A notice only says "read the
  file"; the file decides what the revision is.
- The linked board is PCBCraft-owned and read-only in the assembly, so refreshing
  it is not accepting a change to SolidCraft's authored design. The bridge RFC lets
  the receiver choose automatic or one-click updates. Proposed: SolidCraft's
  existing linked-design preference (**Update changed linked designs when a tab
  comes back**, `update_changed_links`) also covers exchanges, so the board
  refreshes when the assembly opens, comes back to the front, or gets a notice
  while it is in front; with the preference off, **Update Links** refreshes it.
- Accepting changes to authored data stays separate in both apps. In PCBCraft,
  enclosure changes come back only through the reviewed preview in 6.6. In
  SolidCraft, whether a refresh that would change an enclosure feature should
  preview first, instead of recomputing and reporting, is question 17.
- Continuous updates while a part is dragged are a follow-up (below). In the
  first milestone the assembly changes when PCBCraft saves.

**Failure outcomes.** Each leaves the assembly on its current revision unless
stated, with nothing removed quietly:

| Case | Outcome |
|---|---|
| The file holds a lower revision than the accepted one (a restored copy) | Refused as stale, naming both revisions; **Take This Revision** accepts it explicitly |
| The same revision with a different hash (two writers) | Refused as a conflict, naming both; nothing changes |
| A notice for a revision the file no longer holds | Ignored; the file is read instead |
| Several revisions published while SolidCraft was closed or in the background | Only the newest file is read |
| PCBCraft closed, or SolidCraft closed while PCBCraft publishes | Nothing waits on the other app; SolidCraft takes the newest file when it next opens the assembly |
| The file is missing or moved | The link is reported missing; the last bodies stay (SolidCraft keeps linked bodies in the assembly); **Relink** |
| A file with a different `source_id` at the path | Refused as another board; offered as Relink, never swapped in |
| The file requires a capability or major version SolidCraft does not read | Refused (the bridge RFC, section 6); the last bodies stay |
| An item a reference uses was deleted in PCBCraft (a hole, a part) | The reference is missing: its joint or feature is broken and named; no retargeting |
| The file cannot be read | The last bodies stay and SolidCraft tries again on the next change, as it does for linked designs |

**Follow-ups, not in the first milestone.** Continuous updates while dragging
(transient previews over the live session, with their own rate and cancel rules,
never durable revisions); copper, mask and silkscreen bodies, vias, keep-outs and
height zones; exact outline side-face names (slice 3); labelled datums and
enclosure features that follow them (below, slice 12); **Edit in PCBCraft** from
the linked board.

**Who specifies and builds what.**

| Piece | Built by | Specified in |
|---|---|---|
| Exchange file, manifest, link record, outcomes, capabilities, transports | Both | The bridge RFC, sections 2–4 and 6 |
| `board/1` payload, names, frames and units | PCBCraft | #70 ("Define `board/1`", "Resolve units and transforms"); this section |
| Exporter: snapshot, target, revision, atomic write | PCBCraft | #70 ("Export one snapshot", "Persist and inspect links"); this section |
| Board body | PCBCraft, `crates/geometry` | #86 (the board solid) |
| Component models, binding and model frame | PCBCraft | #86 (the model record proposal) |
| Insert Exchange, refresh in place, reference outcomes, update preference | SolidCraft | The bridge RFC, section 5 and Phase 5; this section as a proposal |
| Fixtures and acceptance | Both | Slice 2; #70's checklist |

**Labelled datums.** Later (slice 12), designers can label datums and mark them
published (6.2), so a SolidCraft sketch can follow "J1 origin" or "USB edge" by
name rather than by coordinates, for example an opening that follows a connector.
A published datum that no longer resolves blocks publishing and is named in the
user's words. Detecting a cross-application cycle (an enclosure edge that follows
a board datum which follows that edge) is question 10.

### 6.8 Zones and keepouts

A zone gets a `region`: `drawn` (today) or `board` (the accepted outline, inset
by edge clearance at fill time). Fill stays derived (#232). Because a pad can be
connected only through a zone, the guard (C7) cannot ignore fill once fill
exists: the update recomputes the fill of each zone whose region changed or whose
area the moved copper crosses, with #232's fill rules, inside the same pure
function (6.5), and compares connectivity including it. A connection that only
the old fill made is refused like any other, naming the net and the zone. Whether
that recomputed fill is then stored or recomputed again for display follows
#232. Keepouts, once stored, get the same `region` and can be held like
footprints.

## 7. Slices

Each slice lands on its own with its own acceptance tests. Format changes follow
[native storage](../native-project-storage.md): a new board format only when the
new record is present, older readers refuse it, no-op saves stay byte-identical.

The outbound link comes first (slices 1 and 2): it is the interop #86 puts first,
and it needs no parametric work. The parametric slices follow; slices 3–8 need no
bridge and no SolidCraft.

### Outbound: the board in a SolidCraft assembly

#### Slice 1: publish the board as `board/1`

The export target, **Send to SolidCraft**, **Publish** and auto-publish on save; a
`craft-exchange/1` writer for the manifest, the board body as STEP (from #86's
board solid), each bound component model once per content hash, `faces.json` and
`board.json`, all as section 6.7 describes; `pcb.publish_board` and inspection of
the target. Components are listed without bodies until #86's model record lands.

*Acceptance:* an original fixture board (an outline with a cutout, four mounting
holes, parts on both sides, a declared thickness) publishes a file whose board
body, hole positions, thickness and component transforms, read back with
PCBCraft's own reader and `read_step`, match the known dimensions in millimetres,
with bottom parts turned over, not mirrored; the same saved board publishes the
same bytes; a no-op save publishes nothing; an edit and save publishes the next
revision with a new hash; a board without a declared thickness and a model whose bytes
do not match their hash are refused, naming the cause, and the previous file
stays; a crash between writing and renaming leaves the previous file whole;
publishing changes no board data and no undo history.

#### Slice 2: the linked board updated in the open assembly

SolidCraft's Insert Exchange and refresh in place (6.7), which is SolidCraft's
work in the bridge RFC's Phase 5, and the joint acceptance run. On the PCBCraft
side this slice adds only the fixtures and the run.

*Acceptance scenario*, on the slice 1 fixture and an original enclosure:

1. Publish revision 1. In SolidCraft, insert it into the enclosure assembly, add
   a joint between mounting hole MH1 and a boss, and save the assembly.
2. In PCBCraft, move the right outline edge 5 mm, move connector J1 3 mm along
   that edge, change the finished thickness from 1.6 mm to 1.2 mm, delete mounting
   hole MH4, and save. Revision 2 is published.
3. Without reopening, the same SolidCraft assembly shows the link as changed and
   refreshes (automatically with the preference on, otherwise by Update Links).
   Checked: the linked component is the same one (same `source_id`, same Browser
   entry, same assembly placement, MH1's joint kept); the board body and J1 are at
   the new known dimensions; bottom-side parts sit 0.4 mm higher in the board
   frame; the linked component holds one board body and one occurrence per part
   with a model, with no duplicates; the enclosure bodies are unchanged apart
   from features that reference the board; a joint or feature that used MH4 is broken and names MH4,
   keeps its last result and is not moved to another hole; one Undo returns the
   assembly to revision 1's representation.
4. Rotate a part 90° and flip another to the bottom in PCBCraft, save: revision 3
   refreshes the same component again, with the same checks.
5. Save and close the assembly, close PCBCraft, and reopen the assembly: revision 3
   is shown with no change marker and no PCBCraft running.

Failure fixtures, each with the outcome in 6.7's table: revision 2's file restored
over revision 3 (stale, refused, explicit Take This Revision); two writers with
the same revision and different hashes (conflict); a late notice for revision 2
(ignored); revisions 4 and 5 published while SolidCraft is closed (5 is read);
the file deleted (missing, bodies kept, Relink); another board's file at the path
(refused, Relink offered); a file requiring an unknown capability (refused). No
case moves a reference to a different item.

### Parametric board

#### Slice 3: stable outline edge identity

Per-segment and per-vertex IDs in Board Outline drawings, kept through vertex
edits, moves, undo and copy (copies get new IDs). Published outline side faces
become `exact` (6.7).

*Acceptance:* IDs survive drag, insert/delete of other vertices, undo/redo and
reopen; boards without the IDs keep their format and bytes until edited; the
identity inventory classifies the new IDs; clipboard copies get fresh IDs.

#### Slice 4: board parameters

Records, evaluator, `pcb.set_board_parameters` (complete list,
`expected_revision`, `dry_run`), inspection, Board Setup table.

*Acceptance:* units and suffixes per #75; expressions; cycles, unknown names,
division by zero, overflow and over-length input refused with nothing changed;
rename keeps ID; one undo step; no-op edit keeps revision; reopen byte-stable.

#### Slice 5: parts held to outline edges, carrying copper

Holds `distance`, `along` and `angle` to drawn edges; evaluation; the update
(6.5) without step 5, so carried copper that ends up violating a rule refuses the
change, naming nets; held state in Properties with Remove; moves limited to free
degrees of freedom; `pcb.set_holds`.

*Acceptance:* moving the right outline edge moves a part held to it and not one
held to the left edge; a held part keeps its unheld Y and rotation; dragging a
part held in distance slides it along the edge; a held move is refused with the
way out; routed copper follows in one undo step; a change that would lose a
connection or short a net is refused with the net names and the board
byte-identical; locked copper refuses; deleting a referenced edge leaves the hold
non-driving with a blocking finding; inserting a vertex in an edge does not move
parts measured along it, and neither does a corner rounded later (slice 11); with
the board published (slice 1), the held parts arrive moved in the next revision.

#### Slice 6: parts held to parts

`offset` holds; dependency-ordered evaluation; cycles and double holds refused,
naming them; rigid carry of islands whose pads all move together.

*Acceptance:* a chain of three parts follows a parameter; a routed group keeps its
internal routing shape exactly; a cycle is refused naming the holds in it.

#### Slice 7: re-route inside the update

Step 5 of the update on #246 walkaround (and #247 shove when it lands): remove only
the offending carried segments and reconnect their ends deterministically within
the router's budget; refuse if any connection cannot be restored (C7).

*Acceptance:* slice 5 fixtures that were refused now succeed where a legal route
exists and still refuse, naming nets, where none does; the same change twice gives
byte-identical boards; undo restores exactly; results agree with full DRC; latency
recorded with the #240 harness against slice 5.

#### Slice 8: one-way DXF outline import

The adapter suggested in #153's triage: closed polylines and arcs (once #260
exists) from a chosen layer, units, gap tolerance from rules, into drawn outline
geometry with new segment IDs. No link; independent of every other slice.

*Acceptance:* original DXF fixtures written from the published DXF reference
import with exact dimensions; open loops, crossings and unsupported entities are
refused, naming them; one undo step.

#### Slice 9: reading `.craftx` and a linked outline (after `profile/1` is agreed)

A `craft-exchange/1` reader for the manifest, `requires` and the agreed profile
part; the link record; refresh, preview and accept (6.6); missing source and
Relink; Break Link.

*Acceptance:* per #70's checklist (transforms, simultaneous edits, removed
references, stale base, undo, reconnect, offline reopening), plus: preview and
accept give the same board; nothing moves until accept; each outcome in 6.6 has a
fixture; a file requiring an unknown capability is refused; the board opens and
exports with the source file absent.

#### Slice 10: holds on linked datums and linked parameters

Holds may reference linked edges and points; expressions may use linked
parameters. Mounting-hole footprints held to linked points (#153 step 4b:
position from the enclosure, hole type stays a PCBCraft decision).

*Acceptance:* an enclosure revision that moves a mounting point moves the held
hole footprint and its copper in the preview; a missing point stops driving and
blocks outputs.

#### Slice 11: curves (after #259/#260)

Arc edges in drawn, imported, linked and published outlines; holds measure to arc
edges (distance along the radius) and to virtual corners (C3).

*Acceptance:* rounding a corner moves no part measured along its edges; Gerber
arcs, DRC and the published board body agree within declared output precision.

#### Slice 12: labelled published datums

Labelled datums marked published (6.2) in `board/1`, so SolidCraft sketches can
follow them (6.7); a published datum that no longer resolves blocks publishing.

*Acceptance:* a SolidCraft sketch that follows a published connector datum
follows it through a placement change in the next revision; deleting the datum's
footprint blocks publishing, naming the datum.

### Order

1 needs #86's board solid (component bodies follow its model record); 2 needs 1
and SolidCraft's receiver. 3 and 4 can be reviewed in parallel with 1 and 2; 5
follows 3; 6 and 7 follow 5 (7 also #246); 8 is independent; 9 needs the
`profile/1` agreement; 10 follows 6 and 9; 11 follows #260; 12 follows 2 and 3.

## 8. Performance

Targets, measured with the #240 harness on original command-built fixtures and
revised from the data:

| Operation | Target |
|---|---|
| Evaluate holds after a change, 200 held parts | ≤ 2 ms (closed form, no iteration) |
| Whole update with carry, 100 held parts, nothing to re-route | ≤ 50 ms |
| Whole update with re-route of up to 10 connections | ≤ 200 ms, or a snapshot job with progress |
| Preview of a linked revision, excluding reading the file | Same as the update it previews |
| Publish a revision of a 200-part board, model bytes already hashed | ≤ 1 s, as a job off the editor thread; editing stays responsive meanwhile |
| Selection, hover, camera, opening a board | No evaluation, no geometry rebuild |

For reference, the author's separate implementation (section 10) measured 5–10 ms
for a whole edit including carry and re-route on a two-layer board with a USB
pair, and about 0.3 s in an extreme case where nearly every connection needed
re-routing. That was different code and an iterative solver, so these numbers
only suggest the targets are realistic.

## 9. Open questions

1. **The inbound profile payload.** Agree in #70 and the bridge RFC whether the
   enclosure outline arrives as a `profile/1` part (solved 2D curves with sketch
   entity names, datum points, plane and placement), or is derived by PCBCraft
   from a body's planar face and its named edges. This RFC recommends `profile/1`:
   no B-rep sectioning in PCBCraft, and names that do not depend on #17233's open
   edge-naming points.
2. **Which SolidCraft entities become datums.** All sketch points, or only points
   the SolidCraft user marks as mounting points? Marking keeps the exchange small
   and the intent explicit.
3. **Is closed-form evaluation enough?** This RFC limits holds to one per degree of
   freedom and acyclic dependencies, which needs no solver. Are simultaneous holds
   or in-app parametric shapes wanted later? If so, section 6.1's smallest option
   applies.
4. **Parametric shape without SolidCraft.** Is "draw it, or edit it in SolidCraft"
   acceptable, or is a small parametric rectangle (width, height, corner radius
   from parameters) wanted as a closed-form special case?
5. **Board parameters and text parameters.** Separate kinds (recommended: typed,
   numeric, board-local), or one parameter system with types? Are project-level
   numeric parameters needed across boards (#122)?
6. **Rotation precision.** Footprint rotation is whole degrees. A part held parallel
   to an enclosure edge at 12.3° cannot be represented. Refuse such holds, round
   with a reported residual, or raise rotation precision in a format step first?
7. **One record for holds and #80 dimensions?** A measured dimension is a hold
   with `driving: false`. One record keeps display, picking and orphan handling
   the same.
8. **Fit clearance (#153, question 3).** Inset the accepted outline in PCBCraft
   (a board-local value, possibly from a fabrication profile), or require the
   enclosure sketch to include it?
9. **When copper cannot follow.** Refusal is proposed (C7). Is an explicit opt-in
   ("accept and leave N connections unrouted") wanted for large enclosure changes,
   and is it a request flag or a second confirmation?
10. **Cross-application cycles.** Board datum → enclosure sketch → linked outline →
    hold → board datum. Detect in PCBCraft at publish and accept time, or specify
    in the bridge RFC for every receiver?
11. **Footprint identity.** Footprints use project counters. Holds work with
    those, but clipboard paste and merges of separate copies can collide. Does the
    counter-to-UUID migration need to precede slice 5? The published
    `component:<footprint>` names (6.7) rest on the same counters: a renumbered
    footprint arrives in SolidCraft as a removed part and a new one, with any
    references to it reported broken.
12. **Router operation for updates.** Should #247 provide "reconnect these fixed
    ends of one connection, deterministically, within this budget", or does the
    update compose it from walkaround calls?
13. **Synchronous or job.** Every update through the snapshot-job path (#243), or
    only above a size threshold?
14. **Where the link lives.** Per board (recommended, following #122), with one
    exchange file able to drive several boards of a project?
15. **RFC naming in PCBCraft.** Start `docs/RFC/README.md` with its own numbering
    (as SolidCraft's `docs/rfcs/README.md` does), with a prefix that cannot be
    confused with SolidCraft's RFC numbers?
16. **Publish saved state only?** Recommended (6.7): the assembly never shows a
    board that exists in no saved file, and the revision counter is written by the
    save that publishes. The alternative is publishing unsaved state, as a
    fabrication export may, with a revision kept outside the project.
17. **SolidCraft refresh and enclosure features.** When a refresh changes an
    enclosure feature that references the board, should SolidCraft recompute and
    report (recommended for the first milestone, as linked designs do today, one
    undo step) or preview before the feature changes?
18. **`source_id` for a board.** `<project UUID>/<board UUID>` is proposed. A
    copied project keeps its UUIDs, so two copies would publish under the same
    ID, and a SolidCraft link could take revisions from either. Should copying a
    project give it a new UUID, as Save As does for a SolidCraft design (the
    bridge RFC, section 5)?
19. **The second outbound milestone.** Which comes next after 6.7's first
    milestone: copper and mask bodies, keep-outs and height zones, continuous
    updates while dragging, or labelled datums?

## 10. Prior experience

The author built a parametric board in a separate project
([Sparksmith](https://github.com/DAmesberger/sparksmith), MIT OR Apache-2.0, an
independent implementation, not a PCBCraft fork). Notes:
[`docs/board-sketch-references.md`](https://github.com/DAmesberger/sparksmith/blob/main/docs/board-sketch-references.md),
[`docs/board-rules-and-shape.md`](https://github.com/DAmesberger/sparksmith/blob/main/docs/board-rules-and-shape.md),
[`docs/push-and-shove.md`](https://github.com/DAmesberger/sparksmith/blob/main/docs/push-and-shove.md).
There, the board outline is a sketch solved in-app with SolidCraft's sketch
solver, and parts are held by sketch dimensions in the same solve. The constraints
in section 5 come from that work: an edge-offset record replaced by general
relations (C1), corner-to-corner measuring after fillets (C3), pinned free values
(C5), the carry-then-re-route update with a refusal guard (C6–C8) and stored
results (C10).

This RFC deliberately differs in one place. PCBCraft has the bridge RFC and #70's
ownership rules, and a kernel-free engine, so the shape is solved where it is
owned and PCBCraft keeps only closed-form holds. That avoids pulling SolidCore
into the engine and the browser, and avoids two solvers for one shape. No code
from Sparksmith is proposed.

No KiCad or other GPL EDA source is an input to this document. Nothing here is
derived from commercial CAD products; the bridge's clean-room boundary
([SolidCraft #17766](https://github.com/tangency-labs/solidcraft/issues/17766))
is unaffected.
