# RFC 0001: Parametric board

| | |
|---|---|
| Status | Draft, open for comment |
| Started | 2026-11-19 |
| Issues | [#153](https://github.com/tangency-labs/pcbcraft/issues/153) (linked outline), [#70](https://github.com/tangency-labs/pcbcraft/issues/70) (Craft Bridge), [#259](https://github.com/tangency-labs/pcbcraft/issues/259) and [#260](https://github.com/tangency-labs/pcbcraft/issues/260) (arcs and curved outlines), [#80](https://github.com/tangency-labs/pcbcraft/issues/80) (dimensions), [#86](https://github.com/tangency-labs/pcbcraft/issues/86) (3D and STEP), [#246](https://github.com/tangency-labs/pcbcraft/issues/246)/[#247](https://github.com/tangency-labs/pcbcraft/issues/247) (walkaround and shove) |
| Discussion | the pull request that adds this file, then one issue per slice |

Nothing here is built. This is a proposal for the shape of the feature and the
order of work, so that the outline, placement, routing, zone and 3D work that
touches it can be built to fit from the start. Every name below (commands,
records, fields) is a proposal for review, not a contract.

## 1. What the user sees at the end

A board whose size and shape are driven by a few named values, and whose parts
stay where the designer meant them to be when those values change.

- **Board parameters.** Board Setup gains a Parameters table: `width = 60 mm`,
  `height = 40 mm`, `corner_r = 2 mm`, `usb_y = height / 2 - 1 mm`. Values are
  typed lengths, angles or counts, entered with the [shared unit parser](../quantity-units.md),
  and may be expressions over other parameters.
- **A parametric outline.** The Board Outline can be a sketch: lines (and arcs,
  once [#259](https://github.com/tangency-labs/pcbcraft/issues/259)/[#260](https://github.com/tangency-labs/pcbcraft/issues/260)
  land) with constraints and driving dimensions bound to parameters. Its edges
  have stable identities, and the designer can give them names ("left",
  "USB edge"). Cutouts and mounting holes are further loops of the same sketch.
- **Parts held to the board.** A footprint can be held by a relation to an
  outline edge or point, or to another footprint: "J1's origin 2 mm from the
  left edge, at `usb_y`", "J1 rotated parallel to the left edge", "U2's pad 1
  3 mm right of R5's pad 2". A held part shows that it is held, and why.
- **One update.** Changing `width` from 60 to 65 mm re-solves the outline and
  the held parts, carries their tracks and vias, re-routes the copper that the
  carry left too close to something, re-derives zone outlines that follow the
  board, and re-runs the board-edge checks. It is one undo step. If any copper
  connection would be lost, the whole edit is refused with the nets it would
  break named, and the board is unchanged.
- **The same from MCP.** An agent changes `width` through the same command and
  gets the same result or the same refusal, with a reply that lists what moved.
- **Later, linked sources.** The outline (or individual edges and holes) can
  follow an external SolidCraft sketch or DXF ([#153](https://github.com/tangency-labs/pcbcraft/issues/153)),
  through a preview that names what would move, bound to both revisions
  ([#70](https://github.com/tangency-labs/pcbcraft/issues/70)). Board datums
  are published so that an enclosure can follow the board in the other direction.

None of this changes how a board without parameters works. A drawn outline and
free footprints stay exactly as they are today.

## 2. Non-goals

- Porting another implementation. The author has built this once in a separate
  project (section 9); that experience informs the constraints below, but no code
  is proposed for porting. The design is for PCBCraft's engine, records and
  commands as they are on main today.
- A second solver for part placement, a second geometry store or a second copy
  of the board for 3D.
- Live or silent synchronization with another application. Linked updates are
  always previewed ([AGENTS.md](../../AGENTS.md#extension-and-cross-application-boundaries)).
- Parametric footprints, parametric schematics or parametric copper. Tracks and
  vias are never participants in a relation; they follow the parts they connect.
- Relations to faces of 3D component models. That depends on #86's model record
  and is left as a later extension (section 8, question 13).
- Anything from a SolidCraft fork. Every dependency named here is upstream
  SolidCraft at PCBCraft's current pin, `73bdb6c`. Anything upstream lacks is
  listed as an explicit upstream prerequisite (section 5).

## 3. Where PCBCraft is today

The pieces this builds on, all on main:

| Area | Today | Relevance |
|---|---|---|
| Board outline | Layer-addressed Board Outline drawings: open/closed point chains (`pcb.add_drawing`, `pcb.set_drawing`), UUID per drawing, vertices without IDs ([identity inventory](../object-identity.md)) | The outline has no stable edge or vertex identity yet |
| Curves | Exact circular copper arc records are stored ([#259](../circular-copper-records.md)); conductive consumers refuse them until they support them. Curved outlines are [#260](https://github.com/tangency-labs/pcbcraft/issues/260) | Arcs and fillets in the outline wait for this contract |
| Footprints | Counter IDs (`footprint-N`), pads as owner tokens; exact X/Y/rotation/side; `pcb.set_footprint_transform`, `pcb.move_items`, `pcb.align_footprints` | Stable enough to reference within a project |
| Copper carry | Moves carry attached track ends and refuse new shorts in one undo step ([layout workflow, slice 1](../pcb-layout-workflow.md#1-footprint-moves-carry-their-copper)); locked copper refuses the move | The carry is the base of the board update |
| Routing | Deterministic, bounded single-layer walkaround with `reached`/`blocked`/`incomplete`/`canceled`/`stale` outcomes ([route planning](../route-planning.md)); shove is next under #247 | Re-routing inside an update needs exactly these properties |
| Checks | DRC with outline containment and edge clearance; [board rules](../board-rules.md) resolved with sources | The update's refusal guard reuses the same predicates |
| Zones | [Zone outlines](../copper-zone-outlines.md) are stored; fill is in progress (#232, draft PR #395) | A zone can later follow the board |
| Origin | `pcb.set_placement_origin` with an `outline_lower_left` anchor that follows the outline | Already a relation to the outline, in miniature |
| Parameters | Text parameters (`@project.Name`) for sheets and title blocks | Not numeric; board parameters are a different kind (question 4) |
| Edits | Revision-checked commands, `dry_run` on some, one undo step, preview/apply with a plan token for board updates ([single-board update](../single-board-update.md#preview-and-apply)) | The update and the linked preview follow these patterns |
| Kernel | `crates/geometry` is the only crate that links SolidCore; the engine is kernel-free and the browser leaves the kernel off ([geometry kernel](../geometry-kernel.md#boundary)) | Decides where a solver can live (section 5) |

## 4. Design constraints

These are the rules the feature should hold to. Each comes with the reason,
most of them learned by building it the other way first.

**C1. One solver for every relation.** The outline's constraints, parts held to
edges, parts held to parts and (later) zones held to edges are one constraint
system, solved by one solver. *Reason:* a special-purpose "edge offset" record
for parts next to a general outline sketch looks cheaper, but it grows its own
anchors, alignment modes and edge cases, and then has to be replaced by the
general relation once a part needs to be held to another part or to an arc. We
did exactly that and paid for the migration.

**C2. References name stable identities, never names, positions or order.** An
outline edge is referenced by the sketch curve's ID, a point by the sketch
point's ID, a part by its footprint ID, a pad by its owner token. Designer-given
edge names ("left") are display labels. *Reason:* this is already PCBCraft's
rule ([AGENTS.md](../../AGENTS.md#extension-and-cross-application-boundaries));
edge indices and vertex order change on every structural sketch edit, and a
relation that silently retargets to a different edge moves a connector to the
wrong side of the board.

**C3. A rounded corner keeps its virtual corner.** When a corner is filleted,
the two edges still run corner to corner for the purpose of relations and
dimensions. *Reason:* otherwise changing `corner_r` moves every part that is
dimensioned along an edge from its end, which nobody expects.

**C4. A lost reference stops driving and says so; nothing jumps.** If an edge,
point, footprint or pad a relation names is deleted, the relation keeps its last
geometry, stops driving, and is reported as a finding that blocks fabrication
output until it is fixed or removed. *Reason:* the alternatives (deleting the
relation, or snapping to "the nearest" edge) lose design intent without telling
anyone.

**C5. Free degrees of freedom keep their values.** A part held only in X keeps its
stored Y and rotation. A distance never rotates a part; only a relation that sets
a direction (angle, parallel, perpendicular) does. *Reason:* a least-squares
solver will otherwise "helpfully" turn or slide parts nobody asked to move.

**C6. The update is one atomic transaction that carries copper.** A parameter or
relation edit runs: evaluate parameters → solve outline → solve held parts →
carry copper → re-route offending carried copper → refusal guard → commit. It is
one revision, one undo step and one reply. *Reason:* moving parts and leaving
their tracks behind produces a board full of clearance errors and dangling copper
that the designer then cleans up by hand, which defeats the point of a parametric
board.

**C7. Refuse rather than silently break copper.** After the carry and re-route,
compare the explicit-copper connectivity before and after (tracks, vias, pads;
not derived zone fill). If a connection is lost, a new short appears, a track or
via is left dangling, or locked copper would have to move, refuse the whole edit
and name the nets and the stage that failed. The document, revision and history
stay unchanged. *Reason:* a "successful" update that quietly turns three routed
nets back into airwires is worse than a refusal, because the designer finds out at
DRC or at the fab. This guard is cheap compared with the solve and re-route.

**C8. Routing inside an update is deterministic and bounded.** The same board
and the same edit give the same copper, within a fixed work budget, and the
result's outcome is explicit. *Reason:* an update is replayed by undo/redo,
by MCP retries and by a preview that must match the apply; a nondeterministic
or unbounded router makes the preview a lie and can hang an edit. PCBCraft's
walkaround already has these properties; shove (#247) should keep them, and the
update needs one more operation from the router: "re-route this connection
between these fixed ends" (question 9).

**C9. One resolved board for every consumer.** DRC, Gerber, zone fill, the
canvas, picking, the future 3D view and STEP export (#86) all read the same
solved outline and the same copper through `BoardView`. No consumer re-solves or
re-derives the outline on its own. Selection and camera changes never trigger a
solve. *Reason:* two derivations drift, and a 3D board that disagrees with the
Gerber by a fillet radius is a mechanical failure found too late.

**C10. Store authored inputs and their solved results.** The file keeps the
parameters, the sketch, the relations, and the resolved outline and placements
they produced. Opening, rendering, DRC and fabrication output never need the
solver. *Reason:* files open in builds without a solver (the browser without
the kernel, older readers that refuse new records cleanly), saves without
changes stay byte-identical, and a moved part shows as one changed line in Git.
The solver runs only when an input changes.

**C11. No invented limits.** Every limit the update checks against (edge
clearance, copper clearance, minimum width, outline fit tolerance for curves)
comes from the board's [effective rules](../board-rules.md) with their sources,
or from a [fabrication profile](../pcb-representation.md#project-settings-and-fabrication-profiles)
once those exist. The parametric layer adds no constants of its own.
*Reason:* a hidden 0.1 mm in the carry that differs from the DRC rule produces
updates that pass and then fail DRC.

**C12. Preview before apply for anything linked.** A local parameter edit
applies directly (with `dry_run` available, as on `pcb.set_arc_tracks`), because
undo covers it. An update driven by an external source (#153, #70) always goes
through a preview bound to both sides' revisions, like
[`pcb.preview_update`/`pcb.apply_update`](../single-board-update.md#preview-and-apply).

**C13. Measured budgets, labelled as targets until measured.** Solve, carry and
re-route are measured with the [#240 harness](../performance-testing.md) on saved
fixtures, before and after each slice. Proposed targets are in section 7.

**C14. Agents see what the update did.** The reply lists the parameters
evaluated, the parts moved (old and new pose), the copper carried and re-routed
per net, and any relation that no longer drives. IDs created by a sketch edit are
returned so the next call can name them. *Reason:* without it an agent has to
diff the whole board to find out what its edit did, and humans need the same
summary for the status line.

## 5. Foundations

### 5.1 Solver: reuse SolidCraft's, behind a boundary

SolidCraft's `solidcraft-sketch` crate (MIT OR Apache-2.0) is at PCBCraft's pin.
It has points, lines, arcs and circles with string IDs, geometric constraints and
driving dimensions, a clean-room Levenberg–Marquardt solver with
degree-of-freedom analysis, fillets, closed-profile detection and loose-end
reporting. That covers C1–C5. Writing a second solver in PCBCraft would duplicate
it and drift from the behaviour SolidCraft users already know.

It has two properties that matter for PCBCraft's architecture:

1. **It pulls in the kernel.** `solidcraft-sketch` depends on `solidcraft-geom`,
   which depends on `solidcore-ops` and `solidcore-tess`. PCBCraft's engine is
   kernel-free by design, and the browser build leaves the kernel off because
   kernel panics abort on wasm32 ([geometry kernel](../geometry-kernel.md#boundary)).
2. **It works in `f64` millimetres.** PCBCraft stores integer nanometres.

Proposal:

- The **engine defines a small solver trait** (solve a board sketch; solve a set
  of held parts against fixed board geometry with drags) in its own nm types, and
  receives an implementation from the host, as the app composes other
  capabilities today. The engine stays kernel-free.
- The **implementation lives with the other SolidCraft-backed code**, in
  `crates/geometry` or a sibling crate (question 2), converting nm to mm once on
  the way in and rounding once to the nearest nm on the way out, the same rule the
  geometry crate already uses.
- **Without a solver** (browser without the kernel), boards open, render, check
  and export from their stored results (C10). Editing a parameter or a relation
  answers with an explicit "parametric editing needs the geometry kernel in this
  build" error, never a partial edit.
- **Upstream prerequisite, optional, separate:** propose to SolidCraft that the
  sketch solver and profile detection be usable without `solidcraft-geom`'s
  kernel dependencies and without system font access (the crate's font module
  reads font files from disk), for example behind default features. That would
  let the browser solve without enabling the kernel. This is a request to the
  SolidCraft maintainers, filed and reviewed there; nothing in PCBCraft waits on
  it except browser-side parametric editing.

### 5.2 Board parameters

A board-local, ordered list of records `{ id, name, kind, expression }` where
`kind` is `length`, `angle` or `count`. `name` is a display key, unique per board,
used in expressions; `id` is a UUID so a rename does not break references held by
relations. Expressions allow numbers with units, `+ - * /`, parentheses,
parameter names and a short fixed function list (`min`, `max`, `abs`). Evaluation
is in exact integers where possible (nm, micro-degrees), rejects cycles and
unknown names, bounds depth and count, and stores the evaluated value next to the
expression (C10).

The expression language is question 3: a small PCBCraft evaluator on top of
`pcbcraft-units` is enough for the slices here; reusing SolidCraft's expression
engine would require extracting it from SolidCraft's application crate first.

### 5.3 The outline sketch

A board optionally owns one **outline sketch**: the sketch's points, curves,
constraints and dimensions (dimensions may reference parameter IDs), plus the
curve IDs the designer named. After a solve, profile detection must give exactly
one outer loop; every further closed loop is a cutout (or, later, a slot or a
mounting hole by an explicit role, never by guessing from size). An open,
self-crossing or over-constrained sketch is refused with the reason and the
loose ends.

The solved loops are stored as the board's resolved outline, and `BoardView`
serves them to every consumer in place of Board Outline drawings (C9). A board
has either a sketched outline or drawn Board Outline geometry, never both
(question 5). Converting a drawn outline to a sketch is one explicit command that
keeps the shape and adds the dimensions that fix it.

Arcs and fillets enter the sketch only when #260's curve contract exists, so that
DRC, Gerber and zone fill consume the same curves (C9). Until then the outline
sketch is lines only, which already covers rectangles, notched boards and
polygonal cutouts.

### 5.4 Relations

A relation is a board record `{ id, kind, refs, value, driving }`:

- `kind`: distance (point–point, point–line, line–line along the normal),
  horizontal/vertical distance, angle, parallel, perpendicular, coincident,
  point-on-line, symmetric about a line. These map one-to-one to sketch
  constraints and dimensions (C1).
- `refs`: one to three of an outline sketch point or curve ID, a footprint
  feature (origin, its X axis, a pad center, a courtyard side), and later a
  zone/keepout vertex or an external datum.
- `value`: a literal or a parameter expression; absent for purely geometric
  kinds.
- `driving: false` makes it a measured, non-driving dimension, so that #80's
  documentation dimensions and driving relations are the same record and render
  the same way (question 6).

The held-parts solve builds one scratch system per connected group of relations
the edit touched: the solved outline fixed, each held footprint as a rigid frame
(x, y, rotation) carrying its referenced features, the relations as constraints.
Degrees of freedom no relation fixes are pinned at their stored values (C5).
Footprint features are rebuilt from the pinned definition for each solve and
never stored.

**Moving a held part** is a solver drag: a part held only to the left edge slides
along it. A move that cannot land is refused with the reason and the way out
("J1 is held 2 mm from the left edge; edit or remove that relation to move it").
Locked footprints (#81) are fixed in the solve.

### 5.5 The board update

One engine operation used by every command that changes a parameter, the
outline sketch or a relation, and by drags of held parts:

1. Evaluate parameters.
2. Solve the outline sketch, if it changed or a parameter it uses changed.
3. Solve the held-part groups that reference anything that changed.
4. Carry copper for every footprint whose pose changed, with the existing carry
   (`pcb-layout-workflow` slice 1), extended to carry whole copper islands rigidly
   when every pad they join moves with the same motion.
5. Re-route carried copper that now violates clearance or edge clearance, one
   connection at a time in ID order, with the deterministic router (C8).
6. Run the refusal guard (C7) and the scoped short check.
7. Commit as one revision and one undo step, or refuse with nothing changed.

Steps 1–6 are a pure function of the captured snapshot and the request, so a
`dry_run` (and, for linked sources, a preview token) returns exactly what apply
would commit. Large updates can run as a [bounded snapshot job](https://github.com/tangency-labs/pcbcraft/issues/243)
with the existing stale and cancel rules; apply rechecks the revision.

### 5.6 Zones and keepouts

A zone outline gets a `region`: `drawn` (today's behaviour, the default) or
`board` (the solved outline, inset by the board-edge clearance at fill time).
A board-following zone has no editable vertices. Fill stays derived (#232), so
after an update the fill is simply stale and refills; it is never part of the
refusal guard's comparison (C7). Keepouts are not stored yet; when they are, they
get the same `region` and can be relation participants.

### 5.7 Published datums for linked designs

For #153 and #70's `board/1`, the board publishes **datums**: named outline
edges, sketch points marked as datums, mounting-hole centers, and footprint
origins/axes, each with a stable ID and slot that survive edits and renames. An
external design references a datum, never a vertex index. A datum that no longer
resolves is named in the user's words and blocks publishing until it is resolved.
The inbound direction (an enclosure sketch driving the outline) adds an external
reference kind to relations, resolved with RFC 0001's outcomes (exact, merged,
split, geometric match, missing); a split or approximate match never retargets a
relation silently.

## 6. Slices

Each slice lands on its own, is useful or at least inspectable on its own, and
has its own acceptance tests. Slices 1–4 need nothing that PCBCraft does not
already have. Format changes follow [native storage](../native-project-storage.md):
a new board format only when the new record is present, older readers refuse it,
no-op saves are byte-identical.

### Slice 0: solver qualification (no user-visible change)

A short qualification note and a boundary crate, like the kernel qualification
for #86: `solidcraft-sketch` at the existing pin, its added dependencies and
licences through `cargo deny`, native build, wasm32 build with `wasm-kernel`,
the nm↔mm conversion rule, and the solver trait in the engine with a test double.

*Acceptance:* `cargo deny` clean; solving a fixed set of original fixture sketches
(rectangle, notched polygon, part held to two edges, part held to a part) gives
identical nm results on native and wasm32 with the kernel, or the note records the
difference and its bound (question 12); kernel-less browser build unchanged in size
and behaviour.

### Slice 1: board parameters

Records, evaluator, `pcb.set_board_parameters` (complete list, `expected_revision`,
`dry_run`), inspection, Board Setup table. No consumers yet besides inspection.

*Acceptance:* units and suffixes per #75; expressions over other parameters;
cycles, unknown names, division by zero, overflow and over-length input refused
with nothing changed; rename keeps ID; one undo step; no-op edit keeps revision;
save/reopen byte-stable; board without parameters keeps its format and bytes.

### Slice 2: line outline sketch

Outline sketch record, `pcb.edit_outline_sketch` (a batch of add/remove/constrain
operations returning created IDs, C14), `pcb.convert_outline_to_sketch`, edge
naming, closed-loop detection, resolved outline stored and served by `BoardView`.
Canvas: a sketch mode reusing the existing outline tools, dimensions editable in
place. Changing a parameter changes the outline; nothing else follows yet, so DRC
reports parts that end up outside, honestly.

*Acceptance:* rectangle driven by `width`/`height`; notched polygon; one cutout;
open, crossing and over-constrained sketches refused with the reason; DRC edge
clearance, Gerber outline and placement origin `outline_lower_left` all follow a
parameter change with no other code path; renaming an edge changes no reference;
deleting an edge leaves the outline invalid and refused, never partially applied;
one undo step per edit; reopen without a solver shows the same outline.

### Slice 3: parts held to the board, carrying copper

Relations between footprint features and outline edges/points; held-parts solve;
solver drag for moves of held parts; held state in Properties ("Held 2 mm from
left edge") with Remove; the update (5.5) without step 5: carried copper that ends
up violating a rule makes the update refuse, naming the nets (C7).

*Acceptance:* when `width` grows, a connector held to the left edge stays and one
held to the right edge moves with it; a part held in X keeps
Y and rotation (C5); a held part dragged slides along its edge; a part held from
two sides refuses a drag with the way out; routed copper follows with one undo
step; a move that would lose a connection or short a net is refused with the net
names and the board byte-identical; locked copper refuses; deleting a referenced
pad or edge leaves the relation non-driving with a blocking finding (C4); stored
results reopen without a solver.

### Slice 4: parts held to parts

Relations between features of two footprints; groups solved together; rigid
carry of islands whose pads all move together.

*Acceptance:* two parts held to each other move as one when one is held to an edge;
a chain of three parts follows a parameter; a routed group keeps its internal
routing shape exactly; cyclic and conflicting relations refused, naming them.

### Slice 5: re-route inside the update

Step 5 of the update, on top of #246 walkaround (and #247 shove when it lands):
remove only the offending carried segments, reconnect their ends deterministically
within the router's budget; if any connection cannot be restored, refuse (C7).

*Acceptance:* the board fixtures from slice 3 that were refused now succeed where a
legal route exists and still refuse, naming nets, where none exists; the same edit
twice gives byte-identical boards; undo restores the board exactly; result agrees
with full DRC; update latency recorded with the #240 harness against slice 3.

### Slice 6: zones follow the board

`region: board` for zones; fill marked stale by an update and refilled (#232).

*Acceptance:* a board-following zone grows with `width`; its vertices are not
editable; switching it to `drawn` keeps the current shape; fill stays out of the
refusal guard.

### Slice 7: curves in the outline (after #259/#260)

Arcs and fillets in the outline sketch, with virtual corners (C3), consumed by DRC,
Gerber and zone fill through #260's curve contract.

*Acceptance:* `corner_r` changes the fillet and moves no part dimensioned along an
edge; arc edges as relation references; Gerber arcs and DRC agree within the
declared output precision.

### Slice 8: mounting holes and cutout roles

Explicit roles for inner loops (cutout, plated or non-plated hole, slot);
mounting-hole footprints held to sketch points (#153 step 4b: position from the
sketch, hole type stays an ECAD decision).

*Acceptance:* an inner loop without a role is a cutout, never inferred as a hole
from its size; a mounting-hole footprint held to a sketch point follows a parameter
change with its copper; Excellon output and DRC hole checks read the same roles.

### Slice 9: external sources (#153, then #70)

First a one-way DXF import into the outline sketch as fixed geometry (the triage
note's suggested first step), then a linked source with a stored link
(`source_id`, path hint, accepted revision, content hash, transform), a preview
that lists outline/hole/part/copper changes and reference outcomes, apply as one
undo step, and published datums (5.7).

*Acceptance:* per #70's checklist, plus: the preview and apply give the same board;
a stale preview is refused; a missing source keeps the last accepted geometry and
reports it; no update is ever applied without a preview.

### Slice 10: 3D and STEP from the same board (#86)

The 3D view and STEP export build the board body from the solved outline and the
same copper as DRC and Gerber (C9). Covered by #86; listed here so the order is
clear.

Slices 1 and 2 can be reviewed in parallel after slice 0; slices 3 and 4 follow
2; slice 5 follows 3 and #246; slice 6 follows #232; slice 7 follows #260.

## 7. Performance

Targets, not measurements. They are measured in the slices above with the #240
harness, on original command-built fixtures, and the targets are revised from the
data.

| Operation | Target |
|---|---|
| Re-solve after a parameter edit, 100 held parts | ≤ 16 ms on the reference desktop |
| Whole update including carry, 100 held parts, nothing to re-route | ≤ 50 ms |
| Whole update with re-route of up to 10 connections | ≤ 200 ms, or run as a snapshot job with progress |
| Selection, hover, camera | No solve and no geometry rebuild |
| Opening a parametric board | No solve |

For reference, the author's separate implementation on SolidCraft's sketch solver
measured about 8–9 ms for 100 held parts solved together (release build, Apple
silicon, loaded machine) and 5–10 ms for a whole edit with carry and re-route on a
two-layer board with a USB pair, with one extreme case (a large module rotated so
that nearly every connection needed re-routing) at about 0.3 s. Those numbers are
from different code and only suggest the targets are realistic.

## 8. Open questions

These need a maintainer decision before or during the named slice.

1. **Reuse SolidCraft's solver or write one?** (slice 0) This RFC recommends reuse
   behind an engine trait. Alternative: a smaller PCBCraft solver for distances
   and angles only, at the cost of C1 once curves and fillets arrive.
2. **Where does the solver implementation live?** In `crates/geometry` (the one
   crate that links SolidCore today) or a sibling crate with the same rules?
3. **Expression language.** A small PCBCraft evaluator on `pcbcraft-units`, or
   extract SolidCraft's? Which functions are in scope for the first version?
4. **Board parameters and text parameters.** Separate kinds (recommended: board
   parameters are typed, numeric and board-local), or one parameter system with
   types? Are project-level numeric parameters needed for several boards (#122)?
5. **Sketched and drawn outline together?** This RFC proposes either-or per board.
   Is there a case for a sketch plus extra drawn Board Outline geometry?
6. **One record for driving relations and #80 dimensions?** Recommended yes, with
   a `driving` flag, so documentation dimensions and relations render, pick and
   update the same way.
7. **Footprint identity.** Footprints use project counters. Relations work with
   those inside one project, but clipboard paste and merges of separately edited
   copies can collide (the identity inventory's gap). Does the counter-to-UUID
   migration need to happen before slice 3?
8. **When copper cannot follow.** This RFC proposes refusal (C7). Should there be
   an explicit opt-in ("apply and leave N connections unrouted") for large
   reshapes, and if so, is it a request flag or a second confirmation in the UI?
9. **Router operation for updates.** Is "reconnect these fixed ends of one
   connection, deterministically, within this budget" a router API #247 should
   provide, or does the update compose it from walkaround calls?
10. **Live or deferred drags of held parts.** Solve on every pointer move for the
    preview (cheap at the targets above), or on release only?
11. **Synchronous or job.** Should every update run through the snapshot-job path
    (#243) for uniformity, or only above a size threshold?
12. **Cross-platform results.** The solver is floating point; native and wasm32
    can differ in the last bits of transcendental functions. Is "the stored result
    is authoritative; a re-solve within 1 nm of it is a no-op" acceptable, or must
    native and browser solves be proven identical?
13. **3D model features as references.** Holding a connector by its shell face
    (#86's model record) is useful for enclosures. Separate RFC after #86?
14. **Who owns which value in a linked design (#70).** A dimension lives in the
    file whose geometry it drives; one piece of geometry driven from both apps is
    a cycle. Should PCBCraft detect cross-file cycles itself, or does that belong
    in the Craft Bridge RFC?

## 9. Prior experience

The author built a parametric board in a separate project
([Sparksmith](https://github.com/DAmesberger/sparksmith), MIT OR Apache-2.0,
an independent implementation, not a PCBCraft fork). Its notes are in
[`docs/board-sketch-references.md`](https://github.com/DAmesberger/sparksmith/blob/main/docs/board-sketch-references.md),
[`docs/board-rules-and-shape.md`](https://github.com/DAmesberger/sparksmith/blob/main/docs/board-rules-and-shape.md)
and [`docs/push-and-shove.md`](https://github.com/DAmesberger/sparksmith/blob/main/docs/push-and-shove.md).
The constraints in section 4 come from that work: an edge-offset record replaced
by general relations (C1), corner-to-corner edges after fillets (C3), solver drags
and pinned free values (C5), the carry-then-re-route update with a refusal guard
(C6–C8), and stored results (C10). It also uses SolidCraft's sketch
solver. No code from it is proposed here; PCBCraft's slices are designed against
PCBCraft's own engine, records and commands.

No KiCad or other GPL EDA source is an input to this document.
