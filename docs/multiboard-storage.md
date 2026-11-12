# Multiboard storage and migration

This is part of the [multiboard design](multiboard-and-sheet-instances.md) for
#122. It defines intended behavior, with versioned codecs, migration cases and
focused acceptance tests required before shipping.

## Project layout and document ownership

The working-folder design keeps one project entry point and one library of
sheet definitions. The example below uses `layout_root: "."`; when it is a
subdirectory, all four owned areas (`boards`, `builds`, `documents`, `outputs`)
move beneath that root, while existing shared sheet/library paths stay registered
where they are. Board folders collect physical design data and related files;
they do not introduce another project, session or undo history.

```text
my-project/
  project.json
  sheets/
    <sheet-definition-id>.json
    <sheet-definition-id>.json
  builds/
    <system-build-id>.json
  boards/
    controller-a1b2c3d4e5f6/
      instances.json
      board.json
      variants/
        <variant-id>.json
      documents/
        assembly-notes.pdf
      outputs/
        <export-name>/
          export.json
          bom.csv
          placement.csv
          cam/
    display-0f1e2d3c4b5a/
      instances.json
      board.json
      outputs/
        <export-name>/
          export.json
          bom.csv
          cam/
  documents/
    system-notes.pdf
  outputs/
    <export-name>/
      export.json
      schematic.pdf
      system-bom.csv
  library/
    symbols/
    footprints/
  assets/
```

Only create optional directories when they contain something. The filenames and
export grouping above illustrate ownership; they do not define an accepted
schema. Schematic exports can also be filtered to a particular board and placed
with that board's outputs. Shared assets and pinned definitions should have one
stored copy rather than being copied into every board folder.

`project.json` registers each board's stable ID, display name and one relative
board folder, together with shared circuit and
sheet definitions. Board-local `instances.json` records circuit uses, sheet
occurrences, component instance mappings, references and sparse overrides. Its
header names the board ID, which must match the registry. Physical `board.json`
owns layout, setup, the accepted physical baseline and board export configuration.
Schematic-only uses, occurrences, component mappings/overrides and logical net
registry live once in `project.json` under `schematic_instances`. The empty board
registry `boards: []` is valid; do not create a dummy board or an `instances.json`
file requiring a board ID. System overview/assembly records are separate from
this ordinary schematic scope.

Electrical components remain part of the shared project model and command
transactions even though their instance records are serialized with their board.
Storage location does not create a separate session or duplicate authoritative
ownership. Each instance has exactly one storage home: the project schematic
records or one registered board instance document. Its PCB binding is derived
from that home, with no independent nullable board field on every component.
Validate references and reject duplicate instance IDs across all homes before
adopting the project. The logical net
registry persists named nets, authored rules/constraints and explicit stable-ID
promotions. Accepting a board baseline does not promote all unnamed nets: its
historical membership and snapshot-qualified net keys stay in `board.json`.
Current unnamed unconstrained connectivity remains derived even after layout.
Each persistent net has at most one primary and three fallback anchors, with no
full membership list in `instances.json`. Non-anchor connectivity edits do not
refresh those anchors or serialize every derived net. Exports capture their own
derived addresses in the source snapshot manifest. A file per
occurrence or a directory for every hierarchy level is unnecessary.

For a shared circuit, editing Display's component value or part override changes
its derived `board_root/instances.json`; editing its placement changes
`board_root/board.json`.
Editing shared wiring changes the shared sheet definition and affects its uses.
Resolved defaults and derived pending-update summaries are not copied into all
instance files. A structural definition edit can legitimately change several
instance documents when it adds or removes physical instance mappings, but those
changes publish together through the same transaction and save journal.

### Schematic-only storage and later board creation

New projects may have no boards. Shared sheet definitions and pinned symbol data
retain their usual paths; live schematic-only instances remain in the project
entry point. Optional physical folders are absent until used. `layout_root`
reserves the owned areas but does not require creating empty directories, a
stackup or any physical board state. Saving/packing a schematic-only project is
a complete operation, not a partial PCB export or a missing-document condition.

When assigning complete circuit uses to a board, move their existing use,
occurrence and component records from `schematic_instances` to the board instance
document in one revision-checked transaction and journaled save. Preserve those
IDs/overrides/model links and update affected scope-qualified net/rule references.
If a closed group includes only part of a use, first apply the
[partial-use extraction contract](circuit-instances.md#assignment-of-a-partial-circuit-use):
allocate the new definition/containing use and rebind the selected occurrences
and components while retaining their IDs. Move that new use, leaving unselected
roots and the original use in their existing home. Preserve nested reusable
definitions and use IDs; do not split one use between instance documents.

Detaching an eligible unplaced group uses the same complete/partial-use rules in
reverse. Whole-use moves preserve the use ID; partial-use restructuring allocates
a new one in either direction. Other instances and shared definitions remain
untouched except for the explicitly previewed extraction/fork. No object is
inferred from a folder or duplicated at both homes.

Simulation model references/defaults are shared logical definition data;
compatible overrides live with their instance. Project-level analysis settings
refer to logical targets, with pinned model assets stored under the existing
asset/library rules. Their later simulator-specific schema must not require a
board or put the only copy in physical `board.json`.

### Persisted incomplete intent

`project.json` owns `pending_pcb_assignments` and `pending_boundary_resolutions`,
separate from `schematic_instances`. Schematic-only work creates neither merely
because no board exists. These are authored intent records, not permission to
adopt malformed project data or an alternative source of connectivity.

Each record has a stable ID, a defined reason/intent kind and the affected live
use/occurrence/terminal or anchor addresses. A pending PCB assignment may retain
an import/source board hint as descriptive provenance, but cannot store a bogus
live BoardTarget for a board that does not exist. A boundary record identifies
the required connection/separation decision and current affected scopes/anchors.
Any prior-scope or deleted-source evidence carries its source snapshot identity
as historical context, distinct from its live target fields.

The scope-change transaction creates or updates these records with the actual
instance moves. Resolving an intent applies the explicit scope/connector plan
and removes or resolves its marker atomically. Deleting an affected live target
must revise/remove the marker in the same impact plan or fail validation; Pending
does not make dangling references valid. Unknown intent kinds require a compatible
reader rather than being silently ignored.

Both record families round-trip through save/reopen and undo/redo and appear in
inspection and the appropriate readiness checks. Unresolved required boundary
intent prevents a complete connectivity/assembly claim, while unaffected editing
continues. Before implementing these operations, define the exact record schemas,
resolution commands and dependency checks; a generic string status is insufficient.

### Portable board folder names

Generate one board folder segment `<slug>-<id-prefix>` per board. Build the
slug from ASCII lowercase letters/digits in the display name, replace other runs
with `-`, trim separators and cap it at 48 characters; use `board` if empty.
Append 12 hexadecimal characters of the board UUID, and use the full UUID suffix
if the resulting path collides under case-insensitive comparison. Validate all
segments against Windows reserved names, forbidden characters and trailing
dots/spaces, in addition to the existing traversal/type rules. The suffix means
names such as `aux` become portable ordinary segments rather than `aux` itself.
The implementation must test `con`, `aux`, case-only names and non-ASCII names.

Register a `layout_root` in `project.json` for the owned board, build, document and
output areas. For a new empty project it is `.`. When migrating an existing
folder, do not claim unregistered `boards/`, `builds/`, `documents/` or `outputs/`
directories, even if their names or contents look useful. If any is user-owned, choose
a free `pcbcraft-data-<project-id-prefix>/` root, extending the ID suffix if
needed. Preview the selected root and persist it once. Moving unknown user
folders or adopting their contents requires a separate explicit operation.

The board registry's `folder` field is exactly one portable segment, such as
`controller-a1b2c3d4e5f6`. It contains no root prefix, `boards/`, slash, backslash,
`.` or `..` segment. It is neither a project-relative path nor an absolute path.
Use these project-relative derived paths everywhere:

| Derived path | Definition |
| --- | --- |
| `board_root` | `join(layout_root, "boards", board.folder)` |
| Physical board | `join(board_root, "board.json")` |
| Instance records | `join(board_root, "instances.json")` |
| Board variant | `join(board_root, "variants", variant_id + ".json")` |
| Board documents and outputs | `join(board_root, "documents")`, `join(board_root, "outputs")` |
| System build | `join(layout_root, "builds", build_id + ".json")` |
| System documents and outputs | `join(layout_root, "documents")`, `join(layout_root, "outputs")` |

Resolve each derived path beneath the project directory with the existing path
and file-type checks. `join` is a validated portable-path operation, not permission
to accept a child path that replaces its parent. Owner IDs must agree with the
registry, and two boards cannot claim the same segment or overlapping design
paths. No variant or board document stores an independently editable directory.
Registered associated-file paths remain project-relative and must lie in their
derived owner's document area. Existing legacy associated paths require an
explicit preservation/migration entry, not an automatic move.

Display-name changes do not automatically move folders or change identity. An
explicit Relocate board folder operation validates the new portable path and
updates all affected registered associated paths in the same recoverable save.
Unknown user files require an explicit file-operation plan and are not moved or
deleted by implication. Abbreviated examples below use readable fixed suffixes;
IDs, not those folder strings, establish ownership.

## Retired-board preservation

Before removing an active board registration, register a `retired_board_areas`
entry in `project.json` with its own stable ID, former board ID/name and the same
single-segment folder. Its root is derived by the same
`join(layout_root, "boards", folder)` helper. In the successor snapshot that area
is owned by the retirement record, not by an active board or the project document
area. Rebind registered associated-document ownership to this retired-area ID
while retaining its path under that root's `documents/` area. Existing export
records keep their original historical board/source identities.

Create the retirement entry, rebind document registrations and remove only the
eligible engine-owned empty board/instance files in the same validated save.
The preserved root reserves its path and former board identity; active board
registrations cannot overlap it. Board commands resolve only active boards.
Artifact discovery may present matching former-board runs as historical through
the retirement record, never as a restored live board or current release.

Preserve unknown files without registering or packaging them automatically.
Retirement is not a backup of the deleted physical design and does not promise
that every old source snapshot remains available. Normal packages retain
registered associated files under their valid retired owner; generated history
still needs explicit inclusion. The retired registry is bounded by the project
record/byte budgets, and exhaustion fails the deletion before publication.

Prune a retirement entry only after a bounded check finds no registered files,
output runs or unknown user contents, or after an explicit reviewed relocation
plan has moved/retagged the retained data to another valid preservation owner.
Remove only actually empty directories, with failure preserving the entry/data.
A former-board folder is never silently relabeled as project-owned documents
while bypassing the normal project document-path rule.

## Numeric ID allocation

Persist a project-wide `numeric_id_high_water` in `project.json`, covering every
numeric design-object ID ever allocated under the new format, including objects
later deleted or undone. Allocation reserves IDs strictly above this watermark
and all live/referenced numeric IDs, with checked exhaustion. Save the watermark
atomically with the documents using those IDs. Value and geometry edits that
allocate no identities do not advance it solely because the session revision
changed.

The watermark is monotonic allocation metadata, not ordinary undoable electrical
state: undo/redo, deletion and restoring an earlier content snapshot must not
lower it within the current project lineage. An allocation followed by Undo can
therefore leave a metadata-only change for the next save. Opening and saving a
project without allocations still produces no diff. Preserve the session's
existing stale-request protection separately from persisted object allocation.

Current code reconstructs its allocation basis from live objects when a fresh
session opens; it does not retain deleted historical IDs. Migration initializes
the watermark from all retained live and baseline/reference records. Historical
IDs already deleted before migration cannot be recovered from an old file, so
the no-reuse guarantee begins with the adopted watermark; it must not be claimed
retroactively for legacy history. A delete-highest/save/restart/create test and
an allocate/undo/save/restart/create test are mandatory before new records rely
on this guarantee. Concurrent Git edits still require identity-aware conflict
validation; a watermark is not a distributed allocator.

### Merging allocation metadata

Automatic parallel-branch additions to one design project are unsupported in
the first slices, until schema-aware identity reconciliation exists. This is a
limitation of native design-file collaboration, not of working on the application
repository. Disjoint edits to existing records can still merge when validation
passes, but concurrent new-object allocation may require replay and is not a
promised ordinary Git merge. The mergeability goal remains; the initial numeric
allocator does not satisfy it for concurrent additions.

Concurrent branches can allocate the same numeric IDs and edit the same watermark
line. Equal counter values can merge textually while still hiding duplicate IDs;
a textual conflict is not the only failure mode. Resolve a merge against the
common base: distinguish surviving base objects from independently added objects,
set the watermark at least to the maximum of both branch watermarks and every
retained referenced ID, then allocate fresh IDs for colliding branch additions.
Update all typed references to the remapped objects in the same merge transaction,
including instance mappings, variants, anchors and physical baselines. Merely
choosing the larger counter or retaining both records is insufficient.

Run schema-aware identity reconciliation before project adoption. If that tool
is not available in an initial slice, refuse colliding merged documents and
report the conflicting objects; recover by replaying one branch's additions
through the shared commands on the other branch's validated snapshot. Do not
hand-edit IDs or apply a global text replacement to project JSON. Historical
exports retain their original source snapshot and are never rewritten to pretend
that their IDs were allocated after the merge. Conflict recovery needs tests for
both differing and equal branch watermark values.

### Unsaved snapshots and historical references

The watermark guarantees non-reuse within the saved lineage after adoption and
within the live session, including Undo. It cannot reserve allocations in a file
that was never saved, and an export must not secretly save the project to reserve
IDs. Content hashes alone also cannot distinguish separate unsaved allocations
that later happen to have identical IDs and bytes.

Every immutable source capture therefore receives a fresh `capture_id` UUID,
independent of content, path and session revision. Its source identity is
`(capture_id, content_digest)`. An exported object address includes that pair,
project ID, owner kind/ID and object ID. The capture identity manifest records
the same pair and the mappings used to produce the output. Two separate captures
receive different IDs even when their canonical content digests are equal.

The content digest covers the complete frozen project/source bundle, including
the dependencies captured for coordinated jobs. Each artifact also records an
`input_digest` for its relevant projection of that bundle: board, resolved
schematic, physical state, dependencies and selected configuration. Different
jobs can have different input digests while sharing one capture identity. These
projected digests support freshness checks without confusing source identity
with content equality; they cannot establish correspondence between separately
allocated objects in different captures.

Several coordinated export tasks from one frozen capture share its capture ID;
copying or packaging an existing run preserves its source identity. Neither a
new capture nor an export writes this ID into the live project or changes its
dirty state. Historical references persisted by baseline acceptance, lineage or
pending-intent operations use the same SnapshotRef pair for their captured source;
the authoring operation persists that reference only when it actually commits.

An unsaved component can be exported, discarded and recreated with the same
numeric ID and identical content. The recreated source capture has a different
capture ID, so its historical address is distinct even if its hash is identical.
Never resolve either address against the current project by numeric ID or digest
alone. Use the exact frozen capture, or report that source unavailable. Applying
feedback to a different capture requires explicit validated identity reconciliation,
not an equality assumption based on IDs/content. This also covers divergent Git
branches and read-only exports without a writable reservation sidecar.

Test export, discard, reopen and byte-identical recreation with equal object IDs:
assert equal content digests but different capture IDs/addresses and unchanged
source-project bytes. Also test that CAM/BOM tasks coordinated from one frozen
capture carry the same capture ID and that copying a run creates no new source.
The numeric no-reuse claim remains scoped to saved lineage and the live session,
not to discarded unsaved histories.

## Record ownership and examples

The record boundaries below guide implementation. Production schemas use the
next available version of each changed format, reject unknown required fields
and validate the whole project before publishing it. The version number is
assigned against the implementation base, not guessed from a stale design note.
These examples use readable ID aliases in place of UUIDs; existing numeric
component IDs remain numeric. They are not hand-editable project fixtures.

| Document | Required relationships |
| --- | --- |
| `project.json` | Project identity, numeric ID watermark and layout root; ordered zero-or-more board registry; schematic instances/net registry; typed pending PCB assignments and boundary resolutions; sheet/circuit definitions; system parts/connections/net registry; associated-file, retired-board-area, board-variant and system-build registrations |
| `sheets/<id>.json` | Shared geometry, local symbol/unit references to circuit component definitions, local labels and stable child-use or subsheet slots |
| `<board_root>/instances.json` | Board ID; circuit uses; sheet occurrences with use and optional parent/slot; component instances with references and overrides; logical net registry, anchors, lineage and unresolved logical rule assignments |
| `<board_root>/board.json` | Same board ID; setup and physical objects; package/terminal-to-pad mappings and accepted update baseline |
| `<board_root>/variants/<id>.json` | Board and variant IDs; name; sparse assembly population and compatible BOM differences keyed by component instance ID |
| `<layout_root>/builds/<id>.json` | System build identity; explicit board-to-variant selections and system-component population/compatible BOM selections |

An abbreviated board registry in `project.json` makes file ownership explicit:

```json
{
  "layout_root": ".",
  "boards": [
    {
      "id": "board-controller",
      "name": "Controller",
      "folder": "controller-a1b2c3d4e5f6"
    },
    {
      "id": "board-display",
      "name": "Display",
      "folder": "display-0f1e2d3c4b5a"
    }
  ]
}
```

An abbreviated Display instance document identifies its use of the shared Power
circuit and the override belonging only to Display:

```json
{
  "board": "board-display",
  "circuit_uses": [
    {
      "id": "display-power",
      "definition": "power-circuit",
      "parent_use": null,
      "child_slot": null
    }
  ],
  "sheet_occurrences": [
    {
      "id": "display-power-sheet",
      "definition": "power-sheet",
      "circuit_use": "display-power",
      "parent": null,
      "parent_slot": null
    }
  ],
  "components": [
    {
      "id": 202,
      "circuit_use": "display-power",
      "definition": 17,
      "reference": "U1",
      "reference_number": 1,
      "overrides": {"value": "Regulator B"}
    }
  ]
}
```

Controller has a different circuit-use ID, sheet-occurrence ID and component ID,
but references the same `power-circuit`, `power-sheet` and component definition
17. Its instance can also be `U1`, with no value override. The owner of component
202 is the board named by its registered document, not a parsed folder name.
Full records also carry format headers and any required ordering/allocation data.

Within an ordinary subsheet, `parent` addresses the parent's occurrence and
`parent_slot` addresses the stable subsheet symbol in its definition. A nested
reusable circuit use has `parent_use` and `child_slot` linking it to its caller.
The loader checks these relationships against the shared definition. It rejects
multiple parents, cycles, missing slots, duplicate mappings or occurrences
claiming a different use than their slot. A system overview's board-block slot
is an explicit reference to the board occurrence; it is not a direct electrical
wire across board ownership.

Intentionally schematic-only content is valid project-owned data, not an
unresolved import. A separate pending-board-assignment marker may identify uses
with incomplete PCB intent; it affects only operations requiring that physical
assignment. A system overview is another role entirely, with its own electrical
restrictions. None of these states implies a board based on array position.

## Migration and implementation order

Start by recording optional board association and independent board identities,
then introduce board registries and folders through a reviewed storage migration. Existing
single-board projects must retain their current compatibility board ID when it
becomes a persisted record; allocate independent IDs for additional boards.
Preserve placements, package mappings, layers, stackups, materials, Board Outline
graphics and silkscreen settings. Preserve existing component and terminal IDs
where their meaning is unchanged.

The instancing migration gives each existing sheet one occurrence and preserves
its hierarchy and physical correspondence. All existing flat roots and ordinary
subsheets initially share one legacy circuit-use scope, so units of the same
component remain one package even when they appear on different flat sheets.
Existing component IDs become the physical instance IDs; component definitions
retain the corresponding legacy local keys. Preserve references, values, package
choices and terminal mappings exactly. No synthetic visible parent sheet is
created. Existing values become defaults for those initially single-use component
definitions, without creating unnecessary instance overrides.

Opening a supported flat project does not rewrite it. Its first Save migrates
the board-folder layout atomically, even without design edits; subsequent
unchanged saves have no diff. Successful adoption remains across Undo/Redo,
which restore design content without returning storage to the flat layout. New projects
use the new split-document form with zero boards by default after the feature
ships; board folders appear only when explicitly created. Preserve the original
board's compatibility ID and retain its current display name `Board`, without
renaming it to the project or folder name. Give it a registered portable folder
using that ID's suffix, such as `board-<id-prefix>`. Derive its path as
`join(layout_root, "boards", folder)`; the stored segment never includes that
prefix.
Migration moves only known design documents and leaves associated or unknown
user files in place unless explicitly handled by the associated-file contract.

Legacy circuit definition, circuit-use and occurrence compatibility IDs must be
stable across opens before the first migrated save. Derive them with the existing
domain-separated SHA-256/UUID-v8 technique, using distinct role/version strings
and the project ID, plus the original sheet ID for occurrences. Persist those IDs
on migration. Newly created entities receive independently allocated UUIDs.
Validate collisions rather than falling back to a different object. The codec
brief must fix the byte encodings and test vectors, as for the existing legacy
board compatibility ID.

The format can introduce the definition/occurrence distinction before the editor
supports repeated uses, but readers must not treat a repeated definition as one
physical instance. Never infer a second board or occurrence from a folder copied
by hand. Board creation and circuit instantiation are explicit commands that
allocate registered identities and validate all component mappings.

The implementation can proceed in bounded slices:

1. Specify the board registry, ownership records and single-board migration,
   then make setup, views, commands and storage resolve the addressed board.
2. Specify occurrence and component instance identities, including multi-unit
   components; migrate the existing one-use hierarchy without changing behavior.
3. Add board creation and whole-branch assignment with explicit validation,
   independent setups and correctly scoped connectivity.
4. Add repeated circuit instances, definition editing and update review, with
   instance-aware cross-probing and board synchronization.
5. Add associated-file packaging and reproducible per-board and project exports,
   coordinated with the BOM, fabrication and variant work linked from #122.

Before shipping each slice, specify its versioned records, reference allocation,
label scopes and physical transfer behavior and test the migration. Preserve
stable ordering, small diffs and no-op saves; the second save after migration
must produce no diff. Publish cross-document changes atomically and reject
unsupported boards or instances in older readers rather than truncating them.
