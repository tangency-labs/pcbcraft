# Intentional no-connect markers

This work implements intentional pin and wire-end termination for #285 and #82.
A marker records that one actual part terminal or physical open wire end is
intentionally unused. It is independent of the library's `no_connect` electrical type, terminal-to-pad mappings, net identity
and physical board state. The electrical graph continues to come from wiring,
named anchors and hierarchy; a marker never disconnects a wire or suppresses
other checks.

## Identity and edits

Each part instance owns sparse intent keyed by its exact logical terminal token,
with the electrical type captured when the intent is authored. The component ID
is the existing stable part-instance address used by wires and package mappings.
Coordinates, reference text and pin array positions are not identities. Every
placed appearance of the same logical terminal displays the same marker, using
the placement's resolved terminal position. Moves, rotations and placed-symbol
mirrors therefore require no separate marker transform. Removing an
appearance can leave an addressed missing-appearance finding until the intent
is removed or the unit is placed again; deleting the whole part deletes its
owned intent through the ordinary retirement path.

The shared set/remove command requires an expected revision. Adding intent
requires a live, placed terminal, while removing retained intent does not
require the terminal to remain compatible. Repeating an unchanged action is a
no-op. Marking an already connected terminal records the user's stated intent
and reports the conflict; it does not alter the graph. Undo/redo restores intent
with the owning part. Whole-part clipboard captures retain unresolved intent too;
partial-unit captures retain only intent for terminals in those units. Duplication
remaps the owning part through the existing component-ID mapping.
Hierarchy capture qualifies that intent across the complete selected subtree;
its internal sheet-by-sheet remapping preserves the resulting part records,
including intent owned by later units and unresolved records in a whole-part copy.
If a partial copy contains intent for an unknown terminal, resolve that intent or
copy all placed units; the engine refuses to guess which unit owns it.

Wire-end intent belongs to the live junction at a physical degree-one endpoint,
using its project-unique `junction-N` identity. It follows moves and fresh-ID
clipboard copies, including root routes and descendant sheets in a hierarchical
copy. Deleting the last owning wire removes the marker; Undo restores both.
New wires cannot start, finish or join at a marked pin or wire end. Remove the X
before wiring there, including with W, endpoint drops or MCP. The same rule
checks exact model positions, so coordinate and wire/at targets cannot bypass
intent. Existing wires may still move with their marked owner. Route healing
cannot silently drop or retarget a marker to accept a new connection. A net
name elsewhere on the net does not terminate its physical open end. An exact
named contact at an unmarked endpoint already supplies an electrical termination.

`schematic.set_no_connect` takes `target: {part, terminal}` or
`target: {junction: "junction-N"}`, plus `marked` and `expected_revision`.
`schematic.check_no_connects` is a read-only project query,
including when MCP requests `inspect:false`; both commands use project-owned
terminal or junction addresses. GUI and MCP share the same engine edits and
check implementation.

## Native format review

The split codec extension is `pcbcraft-project/19`, adding a nonempty
`no_connects` list to each affected component record. Each item contains a
terminal token and its captured electrical type, sorted by terminal. Project/18
is already proposed by open PR #294 for named variants, so this work does not
reuse that version. Other document paths and the container version are unchanged.
The first marker adopts the existing board-folder and single-use occurrence
metadata, following the net registry's transaction precedent. Project/19 can
carry either existing instance vocabulary, depending on whether a net registry
exists. It adds no parallel instance home or derived connectivity records.
Wire-end intent additionally uses `pcbcraft-sheet/8`, whose nonempty
`no_connect_ends` array stores live junction IDs in ascending order. It composes
with mirrored placements, field appearance and generated contact-leg provenance.
Older sheet headers reject this field, and cross-sheet or unknown endpoint IDs
are invalid. Removing the last wire flag lets the sheet use its previous header.

Older headers must reject the new field, and the diagnostic single-document
encoder must refuse to drop it. Explicit null, empty or duplicate records,
malformed tokens and unknown fields are invalid. Retained records whose terminal
is missing or has changed type remain inspectable as unresolved intent rather
than silently retargeting. Folder and archive readers use the same native codec;
projects without markers retain their supported low-level no-op encoding.

## Checks and presentation

The intent check returns source revision, stable rule/address keys and navigable
terminal/sheet positions. It distinguishes valid intentional disconnection,
unmarked unconnected terminals and dangling wire ends, connected marked targets,
missing targets and incompatible electrical types. It does not claim the complete type-conflict or
power-driver checks tracked by #47, or incremental scheduling under #245/#243.
Marking a wire end suppresses only that endpoint's `dangling_wire` warning;
`no_connect_not_dangling` reports retained connected ends from older projects. Normal netlist inspection
and physical board DRC continue to describe the actual connectivity.
There are at most 128 marker records per part and 4,096 per project. All
transactions, including compound pastes, validate these limits before publication.
Reports count every marker and finding but retain at most 256 detailed records
of each kind, with an explicit truncation flag and project/revision authority.
The UI shows that limit when reached. Marker-specific latency and large-design
ERC scheduling budgets are unmeasured; this focused reference does not certify
the responsiveness targets in the [performance guide](performance-testing.md).

Markers participate in the ordinary disposable selection model. Click an X,
combine hits with Shift/Ctrl, or include it in a window/crossing selection or
Select All. The No connects filter is independent of symbol and wire filters.
Delete/Backspace and context Delete remove selected intent atomically, without
removing its electrical owner; Undo/Redo restore the exact marker records.
Selection itself does not edit the project or create history. Moving or copying
a selected marker requires its owner in the selection, because an X has no
independent coordinate and cannot be left detached. Markers also continue to
travel implicitly with copied owners. Normal marks use a dedicated blue canvas
token; selected outlines and red conflicts remain distinct.

The placed mark is an X centered on the attachment, with no added lead. Q and
the Capture tool use a larger X cursor in place of the arrow and snap to a pin
endpoint, the drawn pin lead, or a nearby open wire endpoint. Empty-space and
wire-interior clicks cannot author detached markers. The schematic marker is
original domain geometry. Controls and report surfaces reuse the pinned Craft
toolkit through chrome.

In selection mode, marked endpoints are excluded from automatic wire targets
throughout the magnetic radius, not only over the X strokes. Hover and fast clicks
there do not preview or start wires. Explicit Wire mode shows a blocked cursor
and rejects the click without changing the draft's starting point or the model.
The immutable model policy is captured once per engine edit or view target-cache
revision and reused for both source/destination checks and merges.
Interaction follows the published
[KiCad schematic editor manual](https://docs.kicad.org/10.0/en/eeschema/eeschema.html)
for intentionally unused pins; no implementation source, artwork or external
fixture is used.

## Verification brief

Exercise a multi-pin device, both shared and distinct multi-unit terminals,
add/remove/no-op, invalid and stale edits, direct and named connections, missing
unit appearances, incompatible retained intent, moves/rotations, duplication,
flat/hierarchical clipboard and undo/redo. Check exact folder/archive reopen and
unchanged legacy bytes, including projects with a net registry. Use real UI
input and rendered wide/narrow views in both themes. Mirror regressions cover
both axes at every quarter turn, marker drawing and picking, undo/redo and exact
folder/archive reopen with project/19 and sheet/6 together.

Wire-end regressions additionally exercise nearby GUI picks, every rotated and
mirrored pin lead, report Show/Remove, invalid interior clicks, native Save/Open
completion, repeated save bytes, named-net endpoints, endpoint moves, later
connections, deletion, rotated flat copies and hierarchical copies. Native storage
recovery tests combine pin and wire intent, with and without a board/net registry.
Physical DRC findings and the actual netlist must remain unchanged by marker edits.
