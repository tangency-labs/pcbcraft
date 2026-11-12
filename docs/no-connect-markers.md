# Intentional no-connect markers

This work implements the terminal-intent slice of #285 and #82. A marker records
that one actual part terminal is intentionally unused. It is independent of the
library's `no_connect` electrical type, terminal-to-pad mappings, net identity
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
If a partial copy contains intent for an unknown terminal, resolve that intent or
copy all placed units; the engine refuses to guess which unit owns it.

`schematic.set_no_connect` takes `target: {part, terminal}`, `marked` and
`expected_revision`. `schematic.check_no_connects` is a read-only project query,
including when MCP requests `inspect:false`; both commands use project-owned
part/terminal addresses. GUI and MCP share the same engine edits and check implementation.

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

Older headers must reject the new field, and the diagnostic single-document
encoder must refuse to drop it. Explicit null, empty or duplicate records,
malformed tokens and unknown fields are invalid. Retained records whose terminal
is missing or has changed type remain inspectable as unresolved intent rather
than silently retargeting. Folder and archive readers use the same native codec;
projects without markers retain their supported low-level no-op encoding.

## Checks and presentation

The intent check returns source revision, stable rule/address keys and navigable
terminal/sheet positions. It distinguishes valid intentional disconnection,
unmarked unconnected terminals, connected marked terminals, missing targets and
incompatible electrical types. It does not claim the complete type-conflict or
power-driver checks tracked by #47, or incremental scheduling under #245/#243.
Normal netlist inspection continues to describe the actual graph.
There are at most 128 marker records per part and 4,096 per project. All
transactions, including compound pastes, validate these limits before publication.
Reports count every marker and finding but retain at most 256 detailed records
of each kind, with an explicit truncation flag and project/revision authority.
The UI shows that limit when reached. Marker-specific latency and large-design
ERC scheduling budgets are unmeasured; this focused reference does not certify
the responsiveness targets in the [performance guide](performance-testing.md).

The schematic marker is original domain geometry. Controls and report surfaces
reuse the pinned Craft toolkit through chrome. Interaction follows the published
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
