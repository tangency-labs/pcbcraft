# Native project storage

PCBcraft stores a project as independently editable documents. The editor and MCP
still share one project, command set and undo history; storage boundaries do not
create separate schematic and board sessions.

The [multiboard and sheet instance design](multiboard-and-sheet-instances.md)
and its [storage contract](multiboard-storage.md) describe board folders,
instances and associated-file ownership. The current encodings below do not
accept that new layout until the codec migration lands. The bounded
[optional single-board codec](single-board-storage.md) adds explicit zero-or-one
ownership while retaining the root board path.

## File encodings and save behavior

The working form is a project folder with a plain UTF-8 JSON `project.pc` entry
point. Desktop New Project defaults to this form. A project's first Save asks
for a new folder, suggesting the project's name; one still named Untitled takes
the chosen folder or archive name as part of that save, with no undo step.
Cancelling or failing that save leaves it Untitled and unsaved. Open accepts the
folder, its entry point, a `.pc.zip` archive or a legacy project. Save keeps the opened form;
Save As offers a project folder and a packed project. The folder destination is
a new or empty project directory, while archive Save As requires an unused file.
Unrelated root files such as `README.md` and `.git` remain untouched.
Regular `.DS_Store`, `Thumbs.db` and `desktop.ini` files are also left alone inside
project subdirectories; they are not project documents and are never published
or removed by a save. Other unknown files in managed subdirectories are refused.

```text
my-board/
  project.pc
  sheets/<sheet-uuid>.json
  board.json                    # only when a physical board exists
  library/symbols/<library>/<symbol>.r<revision>.json
  library/footprints/<library>/<footprint>.r<revision>.json
  assets/                       # reserved for future asset references
```

Opening `project.pc` opens the entire project. Copy the folder or export its
`.pc.zip` to share it; the entry point alone does not contain the other documents.
A project rename changes the name in `project.pc`, without renaming its files.

The browser uploads packed projects and legacy single-file projects and downloads
`.pc.zip`. It cannot reconstruct a folder from an uploaded entry point alone.
Download encoding runs in batches of 256 records and archive hashing/copying in
256 KiB chunks, allowing the editor to yield between batches. The board-header
phase also writes its bounded setup of at most 64 material revisions, 128 layers,
127 strata and 256 nested prepreg plies. A requested download
does not confirm a durable write and does not clear the editor's dirty state.

## Document ownership

| File | Format | Contents |
|---|---|---|
| `project.pc` | `pcbcraft-project/6`, `/7`, `/9`, `/11` or `/12` | Project UUID, name, ordered sheets and components; version 7 adds the numeric allocation watermark, version 9 adds explicit zero-or-one board registration, version 11 adds component metadata for legacy ownership, and version 12 combines metadata with explicit board registration |
| `sheets/<sheet-uuid>.json` | `pcbcraft-sheet/1` through `/5` | Sheet UUID, unit placements, junctions and wires; version 2 adds hierarchical labels and child sheet symbols; version 3 adds named electrical anchors; version 4 adds per-unit field appearance; version 5 adds net-name appearance |
| `board.json` | `pcbcraft-board/1` through `/11` | Footprints; version 2 adds layer/stackup setup, version 3 adds material reports and construction; version 4 supports footprint orientation; version 5 adds pinned physical footprint references and terminal mappings; version 6 stores physical silkscreen settings; version 7 separates component package assignments from placements; version 8 is the earlier local rectangle; version 9 stores layer-addressed Board Edge graphics; version 10 explicitly enables accepted update baselines; version 11 distinguishes pending desired package assignments from accepted geometry |
| `library/symbols/<library>/<symbol>.r<revision>.json` | `pcbcraft-symbol/1` | One full pinned symbol copy, including its library ID |
| `library/footprints/<library>/<footprint>.r<revision>.json` | `pcbcraft-footprint/1` | One full pinned physical footprint copy, including provenance, land contours and optional circular plated drills |

A sheet-list entry contains `id`, `name` and `file`. The file and the document's
UUID must agree. Version 6 supports 1–64 ordered sheets with non-nil,
project-unique UUIDs and names unique without case. Names are trimmed, nonblank,
control-free and at most 200 characters. Each sheet owns its symbols, junctions,
wires and connectivity; wire endpoints cannot reference another sheet. Components,
reference allocation and board placements remain shared project data. The existing
project-wide object and geometry bounds apply across all sheets.

`sheet.create`, `sheet.rename` and `sheet.delete` are revision-checked project
commands. Deletion removes the sheet’s components and associated board footprints
atomically, and cannot remove the last sheet. Schematic editing commands and
footprint placement take an explicit `sheet` UUID when more than one sheet exists;
unknown or ambiguous targets never fall back to another sheet. A view’s active
sheet belongs to user-local presentation state and is never saved in `project.pc`.

Version 5 remains readable with exactly one sheet. Its original default name
`Schematic`, historically displayed as Main, migrates to `Main`; other names stay
intact. Formats 1–4 migrate to one Main sheet with their original sheet UUID.
Legacy snapshots retain version 6 until numeric allocation or retirement adopts
the version-7 watermark, an eligible physical edit persists version-9 board
registration with the same compatibility ID, or component metadata requires
version 11. New projects start at version 9 with no physical board, unless New PCB
explicitly requests one. Component metadata with explicit zero-or-one ownership
uses version 12, which retains the watermark and registry; version 11 keeps legacy
implicit ownership and rejects a `boards` record. Version 9 rejects component
metadata rather than interpreting version-12 fields. Sheet encodings remain
sheet/1 for ordinary sheets, sheet/2 for
sheets with hierarchy records and sheet/3 for sheets with named anchors, retaining existing board encodings. An unchanged save preserves original version-5 document bytes, and
editing another document preserves an unaffected entry point. Adding or renaming
a sheet writes the version-6 entry point. Legacy single-file encoding refuses
multiple or renamed sheets, hierarchical labels, child symbols, named net anchors,
component metadata and field appearance rather than discarding them.

### Component fields and text appearance

Project/11 and Project/12 add an optional non-null `metadata` object to each component and
requires the numeric allocation watermark. A metadata edit upgrades a legacy
project with a watermark covering its retained IDs; Undo may retain that
allocation metadata while removing the component extension. Project/7 remains
the allocation-only vocabulary and rejects component metadata. The new object
contains `fields`, keyed by stable standard IDs (`datasheet`, `description`,
`manufacturer`, `manufacturer_part_number`) or non-nil UUIDs for custom fields,
and `attributes`. Each field has a display `name` and `value`; names are unique
without case, nonblank and at most 64 characters, and values are single-line
text of at most 2048 characters. Up to 64 optional fields are allowed. Attributes
are booleans `do_not_populate`, `exclude_from_bom`, `exclude_from_board`,
`exclude_from_simulation` and `exclude_from_position_files`. All default false.
A component excluded from the board cannot receive a new placed footprint. An
existing placement may remain under an accepted baseline pending reviewed removal;
legacy boards without a baseline refuse that exclusion. Package
assignment can remain pinned for later inclusion. Other flags retain authored
intent for their respective future output/simulation consumers.

Sheet/4 retains sheet/3's required `net_labels`, `labels` and `subsheets` arrays,
even when empty. Each placement may add non-null `field_layout`, keyed by
`reference`, `value` or an existing optional field ID. A layout stores `offset`
(null for automatic placement, otherwise integer `x_nm`/`y_nm` in unrotated
symbol space), `visible`, `show_name`, `rotation` (0/90/180/270), `align`
(`start`/`middle`/`end`) and `height_nm` (0.1–10 mm). Offsets are bounded by
the schematic center range. Manual offsets turn with the symbol; explicit text
orientation is independent. Unspecified reference/value layouts retain legacy
automatic rendering, and optional fields start hidden. Absent metadata/layout
stays omitted, preserving older canonical document encodings. Older readers
reject the project/11 or /12 and sheet/4 extensions instead of silently discarding data. Reference/value content
edits stay in `project.pc`; label placement edits stay in the owning sheet.

Sheet/5 extends sheet/4 with optional non-null `text` on each named anchor. It uses
the same field-layout vocabulary and bounds. Offsets are relative to the electrical
anchor in its unrotated space; the text angle is explicit and its height scales
with zoom. Missing `text` preserves the automatic layout and old sheet encoding.
Only sheets with authored net-name appearance use version 5; older readers reject
that version. Appearance edits do not move contacts or alter net names.

### Named net records

Sheet/3 extends sheet/2 with a required, non-null `net_labels` array, one record
per line in stable object-ID order. Each record contains a project-unique `id`,
case-sensitive `name`, `style` (`local`, `global`, `power`, `power_bar` or `ground`), `at`
point in integer nanometers and `rotation` (0, 90, 180 or 270 counterclockwise).
Names contain 1–200 characters without surrounding whitespace or controls; each
sheet permits at most 256 named anchors. Ground/power markers are electrical
annotations without component IDs, references or footprint assignments.
`power` retains the arrow drawing; `power_bar` selects a flat horizontal bar with
the same global-name semantics. The style persists independently of its editable
net name. Builds predating `power_bar` reject that unknown style; existing styles
and their encodings remain unchanged.

An explicit wire endpoint `{"net_label":<id>}` refers only to an anchor owned by
that sheet. A named anchor also contacts exact coincident pins and points on
wire segments; it is an authored connection marker, so placing one at a crossing
joins the contacted routes. Other crossings and coincident distinct endpoints
retain their existing behavior. Names join within a sheet, including its
hierarchical interfaces; global/power/ground markers additionally join matching
global names across sheets. Derived net names and graphs are never persisted as
a second set of electrical identities. All aliases remain in inspection, with
global names preferred for display and lexical ordering within each scope.

Sheets with named anchors use version 3; sheets without them retain version 1
or 2 as appropriate. Earlier versions reject named-label fields, and the legacy
single-file writer refuses named anchors rather than losing them. Named-anchor
records do not themselves change the entry-point registry; the independent
single-board ownership slice adds the zero-or-one `boards` list in project/9
and project/12. Moving/renaming a label
changes only its record; an unchanged save retains all document bytes. Folder
and packed-project validation rejects foreign endpoints, ID collisions, missing
or null arrays, unknown fields/types and invalid text or geometry before adoption.

### Hierarchical sheet records

The project list remains flat and authoritative for document identities. Nesting
belongs to `subsheets` records in the parent sheet/2 document, each containing a
global object `id`, the `child` sheet UUID and its top-left `at` point. Each child
has at most one parent, cycles are refused, and removing a parent symbol promotes
the child to a root without deleting its document. Deleting a sheet removes its
incoming symbol and wires attached to its parent pins, while promoting its own
children to roots. These operations share the ordinary project undo transaction.

Sheet/2 requires both `labels` and `subsheets` arrays. Each label stores its global
object `id`, case-sensitive `name` (unique within that sheet, 1–200 characters),
`direction`, child-side anchor `at` and stable side-local `slot` (0–63). At most 64
labels are allowed per sheet. Directions are `input`, `output`, `bidirectional`,
`tri_state` and `passive`. Output/tri-state pins appear on the right; other pins
appear on the left. Names and directions on the parent derive from child records,
so synchronization never relies on matching independently editable strings.
Slots preserve other pins' positions when a label is removed. Symbol geometry is
16 by 10 schematic grid steps by default and grows vertically to contain occupied
pin slots. A sheet symbol can optionally store an authored `size` point, whose
`x_nm` and `y_nm` are positive width and height. Missing size retains legacy
defaults; explicit null is refused. Authored extents must be at least 8 by 6 grid
steps and fit within schematic bounds. Adding child labels can grow the effective
height; explicit resizing refuses a rectangle smaller than current pin clearance.
`schematic.resize_sheet` accepts a complete origin/width/height rectangle and
commits it atomically, preserving all endpoint identities. Right-side pins follow
the authored width while vertical slots remain stable.

A stored `{"label":<id>}` endpoint addresses a child label; `{"sheet_pin":<id>}`
addresses its distinct parent-side pin. Both resolve only within their owning
sheet. Ordinary terminal/junction ownership checks remain in force. The engine's
project electrical graph adds an explicit link between each placed child's label
and corresponding parent pin; coincident coordinates and repeated names on
unrelated sheets never create cross-sheet links. Per-sheet net inspection shows local endpoints after named connections are resolved
project-wide, while `inspect_project.project.netlist` reports the complete electrical
graph, including interface identities. Direction is currently descriptive; ERC is
separate work.

Sheet/1 rejects hierarchy fields even when empty or null. Sheet/2 rejects missing
or null arrays, invalid or duplicate object IDs/slots/names, unsupported directions,
missing children, multiple parents, cycles, foreign endpoints and out-of-bounds
geometry before project adoption. A no-change save retains the existing document
bytes; each hierarchy record occupies its own line.

Components own their stable ID, library/definition/revision, reference number,
displayed reference and value. Unit placements own their ID, component link,
unit, position in integer nanometers and rotation in degrees (0, 90, 180, 270).
Wires retain terminal or junction endpoints and optional authored bends; legacy
auto-routed pin links may omit bends. The sheet always has a junction array.
Footprints retain their component link, definition/revision, pin-to-pad mapping
and position. Version 4 adds optional `rotation` in counterclockwise whole degrees
from 0 through 359, around the footprint origin with board Y increasing downward.
Quarter turns resolve exactly; other angles round pad offsets to the nearest
nanometer. The GUI steps by 90°, while the shared command accepts any supported
whole-degree orientation.
Omitted rotation means zero, and canonical records omit zero to preserve legacy
bytes. A nonzero orientation requires `pcbcraft-board/4` or a later supported board version; earlier versions
and legacy single-file encodings refuse it rather than lose orientation. Version
4 permits an absent setup or the version-3 setup vocabulary, preserving layer,
material and mapping identities. Encoding uses versions 1–3 when no footprint is
rotated, and legacy single-file export refuses rotated footprints. Moving changes
only the affected footprint record; rotating can additionally change the board
format header. Every project has its sheet documents. A new schematic-only
project has no board document; an explicitly created PCB and legacy implicit-board
projects retain `board.json`, even when no footprints are placed. The board registry
supports zero or one board. Newer physical-board versions add data for that board,
rather than implementing multiple boards.

Physical footprint references require board version 5. Each instance carries
`library`, `definition`, `revision` and an explicit `mapping` from terminal ID to
pad ID, together with its position and optional rotation. The complete
[footprint definition](footprint-library.md) is saved separately and must match
its path identity. Missing, conflicting or unused definitions are refused; loading
never substitutes a built-in definition. Version 5 allows absent setup or the
version-3 setup vocabulary. Legacy marker instances can coexist with physical
placements, and legacy single-file export refuses physical definitions. Existing
marker-only projects retain their bytes and are never upgraded to physical pads
implicitly. Deleting the last placement using a physical definition removes its
unused pinned copy on save.

Board format 6 requires a `silkscreen` object with `text_height_nm`,
`stroke_width_nm` and `pad_clearance_nm`, as defined in the
[footprint contract](footprint-library.md#silkscreen-settings). It permits the
same optional setup, placement orientation and pinned definitions as version 5.
Versions 1–5 refuse this field; untouched older boards keep their bytes and use
the stable default style for definitions that request physical silk. Editing the
style changes only the board document. Legacy single-file export refuses these
settings rather than discarding them.

Board format 7 adds a nonempty `assignments` array before `footprints`. Each
record carries the numeric component ID, library/item/revision and an explicit
terminal-to-pad mapping, with no placement ID or coordinates. Every assignment
must resolve a component and pinned physical definition; duplicate components,
incomplete/nonbijective mappings and disagreements with an existing placement
are refused. Unplaced assignments retain their pinned files even with an empty
`footprints` array. The incremental encoder writes one assignment per record
budget and emits shared definition copies once. Setup and silkscreen remain
optional under version 7, with their existing validation. Older boards retain
their prior encoding until a package assignment is authored. Legacy single-file
export refuses independent assignments rather than losing them.

Board format 10 is explicitly enabled by `pcb.initialize_update_baseline` and
stores accepted historical components, packages and terminal membership. It can
retain a footprint after its live component is deleted and preserves assets
referenced only by accepted history. The [single-board update foundation](single-board-update.md)
defines the codec and current engine-only limitations; older readers refuse this
format instead of dropping retained physical state.

Board format 11 retains that baseline while allowing desired assignments to differ
from accepted placements. It is required exactly when package intent is pending;
Apply or resetting intent to accepted values removes that difference. Desired
mappings validate against live logical terminals and pinned package pads, while
accepted footprint mappings validate independently against historical terminals.

Board format 9 stores `graphics` with `layers` and `items` arrays. Items have
independent UUIDs, a layer UUID, integer-nanometer `points`, and a `closed` flag.
Layers in this section are technical Board Edge additions for boards without a
matching configured layer; configured boards reference their existing layer UUIDs.
Creating graphics does not invent copper stackup or change footprint bindings.
Each path has 2–256 vertices (at least three when closed), no adjacent duplicates,
and no repeated closing vertex. The codec validates non-nil unique drawing IDs,
layer references and bounded resources (512 drawings, 4096 total vertices and
128 combined layers); records sort by UUID. Optional setup, silkscreen, assignments
and placements retain their existing validation and identities. Empty technical
metadata remains after deleting the last graphic, preserving layer addresses.

The earlier local format 8 rectangle remains readable and byte-stable on no-op
save. Its first actual geometry edit materializes a layer-addressed drawing in
format 9. Versions 1–7 keep their encoding until new data is authored; they reject
newer fields instead of silently discarding them. Legacy single-file export
refuses edge graphics. Board document additions do not introduce a broader
multiboard migration; the entry point separately records explicit zero-or-one
ownership in project/9 or project/12.

An unconfigured board without newer footprint, assignment, edge graphic, outline or silkscreen data
retains version 1 and its existing encoding. Initializing setup uses version 2, whose required `setup` contains a `layers` catalog and a
`stackup` profile. Layers have UUIDs, names, typed purposes and optional surface
sides; the profile has a UUID and front-to-back `strata`. Copper strata reference
catalog layer UUIDs, while dielectric strata own UUIDs and optional material
names. Profile, layer and dielectric IDs must be non-nil and mutually unique.
Catalog order is canonical UUID order; physical order belongs only to strata.
The legacy material name is descriptive text only. Material-aware setup uses
version 3, which adds a project-local `materials` catalog, exact ID/revision
references, sourced Tg and Dk/Df reports, core/prepreg construction, ordered plies
and optional symmetric finished-gap tolerance. The [material contract](board-materials.md)
defines field meanings, units and bounds. Unmodified version-2 projects retain
their bytes and unknown construction; material-aware fields require version 3
even when a field's collection is empty in the input.

The current uniform rigid profile alternates copper and dielectric, with front
and back copper at its ends and every catalog copper layer appearing once.
There are at most 128 catalog layers and 64 copper layers. Names are unique
case-insensitively, unpadded, nonblank, control-free and at most 80 characters;
material names use the same text bounds but need not be unique. Optional
`thickness_nm` is an integer from 1 through 100,000,000; the sum of specified
thicknesses cannot exceed 100,000,000 nm. Omission means unknown; explicit null
is rejected. The derived laminate thickness remains unknown until all strata
are specified, and is never stored as a second independently editable value.

Version 1 refuses setup, and versions 2/3 refuse missing setup. Unknown fields,
purposes, references and unsupported versions fail before project adoption.
Setup decoding preserves original JSON for typed validation, so duplicate keys
are refused rather than overwritten while checking the version's vocabulary.
Legacy single-document encoders refuse configured boards instead of losing
their setup; ordinary folder/archive saves use the current split-document
codecs. The [representation plan](pcb-representation.md) explains future regions,
footprint bindings, shared rules and the unit contract with #75.

Saved symbol copies include all terminals, strokes and pins. A project opens
without its source library. Loading never upgrades a pinned definition to the
latest revision. The last component using a copy determines its lifetime:
deleting that component removes the unused copy on the next save. Libraries and
symbols are identified by their IDs and revisions; paths locate those identities.

Symbols placed from a registered native source also retain an optional
`provenance` object with `library_name` and `license`. It contains portable source
attribution rather than a personal registration ID or machine path. The saved-copy
reader validates it before adoption; copies without this field preserve their
existing bytes. A new placement refuses different content under an already pinned
library/item/revision identity. Source registration and cached search data remain
outside project documents; see [native library sources](native-library-sources.md).

All documents reject unknown fields. Required lists and optional wire geometry
reject explicit null. Before adoption, validation checks the entire project:
component ownership and references, unit membership, wire endpoints, junction
usage, footprint mappings, exact definition revisions and unused copies. Object
IDs are unique across placements, junctions, wires and footprints; component IDs
have their own namespace. Versions 7, 9, 11 and 12 require `numeric_id_high_water`, an integer
between zero and the maximum supported object ID and no smaller than any live
numeric identity. Allocation reserves above that value and the runtime revision.
The watermark includes retired IDs, does not decrease on undo/redo or restoration
of older content within the current live project, and saves atomically with the
records that use it. Undo of an allocation can therefore leave a metadata-only
change to save. Nonallocating edits do not advance it merely because the revision
changed, and unchanged reopen/save remains stable.

Older snapshots omit the watermark and preserve their encoding through ordinary
nonallocating schematic edits. Their first numeric allocation or retirement adopts a
watermark covering both prior and resulting retained IDs. Previously deleted IDs
absent from legacy data cannot be recovered; non-reuse begins with adoption.
The retained single-document encoder uses `pcbcraft-project/8` when a watermark
is present on legacy ownership, or format 10 for explicit optional ownership;
unadopted snapshots retain format 4. Formats 7–12 require the field and earlier
versions reject it, including explicit null values. Physical registration migration
also adopts the watermark. Component metadata and field appearance remain refused
by single-document encoders. This is
not a distributed allocator for concurrent branch additions, nor a reservation
of IDs from discarded unsaved histories.
A component without a placed unit is still refused.

## Git behavior

[Named arrangements](board-workspace-plan.md#named-arrangements-and-reopening-a-project)
are kept in versioned user-local application storage. Camera, pane, tab and window
changes do not rewrite electrical project files or create undo entries. Successful
Save As copies the local arrangement record to the destination association;
failed or cancelled saves retain the original. Portable arrangement export remains
a separate future format and feature.

Commit the working folder. Each editable record has a stable ordering and its
own line; separator commas occupy separate lines so appending a record does not
rewrite the preceding record. Saved symbol files have one line per terminal,
stroke and pin. Timestamps, generated hashes, caches and thumbnails stay out of
the working documents.

Moving or rotating a symbol changes only its sheet record. Editing a reference
or value changes only its component record in `project.pc`. Board geometry stays
in `board.json`. Placing a new definition changes the project and sheet documents
and adds its saved symbol file. A no-op save rewrites no project documents.
Even after another document changes, unaffected documents retain their original
bytes, including valid custom formatting.

Independent edits to existing records merge with Git's normal text driver, as
exercised by a real `git merge-file` regression test. Concurrent additions can
still conflict at the end of a list and can allocate colliding IDs or references.
Resolve those conflicts explicitly, preserving distinct identities and updating
references to any reassigned IDs, then reopen for validation. Blindly retaining
both records is insufficient when their IDs collide. An ID-aware merge driver is
outside this change.

```gitattributes
*.pc text eol=lf
*.json text eol=lf
*.pc.zip binary
```

Ignore `.pcbcraft-save/`, `.pcbcraft-stage-*/` and `.*.pcbcraft-lock`. ZIP exports
are binary; optional text conversion can make local diffs readable but does not
provide normal text merging. The folder is the collaboration format.

## Packed projects and manifests

A `.pc.zip` contains the same relative document paths and a root `manifest.json`:

```json
{
  "format": "pcbcraft-container/2",
  "parts": [
    {"path": "board.json", "size_bytes": 66, "sha256": "<SHA-256 of the exact bytes>"}
  ],
  "assets": []
}
```

The example is abbreviated: `parts` lists every document except the manifest,
sorted by path, with its exact size and SHA-256 digest. Assets are reserved;
nonempty assets and asset files are refused until the engine supports their
references. The working folder carries no manifest or cross-file hashes, which
would otherwise change alongside every edit.

A present manifest is authoritative. Missing parts, unlisted files, duplicate
claims, malformed manifests, unknown versions and size/digest mismatches fail
the open. A broken manifest never falls back to manifest-less loading.

A hand-zipped project **without a manifest** is accepted when it contains exactly
a valid project layout. An optional single top-level folder, such as
`my-board/project.pc`, is stripped; multiple roots and unrelated files are
refused. Ordinary directory entries are accepted. In archives without a manifest,
`__MACOSX/` sidecar trees and regular `.DS_Store`, `Thumbs.db` and `desktop.ini`
files are discarded before locating the project root. They still undergo ZIP
path, type, CRC and size checks. A present manifest disables these exceptions.
All document schema and cross-document checks still apply. The open notice says
that the next save or download will add a manifest. PCBcraft always writes one.

Writers use Store, fixed timestamps, fixed permissions and a deterministic entry
order (manifest first, then paths in lexical order). Readers accept Store and
Deflate. Traversal, absolute/drive paths, duplicate or overlapping entries,
symlinks, special files, encryption, ZIP64 and split archives are refused. Reads
are bounded by both declared and actual expanded size. A no-op save of a current
manifested archive preserves its original bytes, including compression and
comments. A first save of a manifest-less archive intentionally adds the manifest.

| Bound | Limit |
|---|---|
| Legacy single JSON document | 4 MiB |
| Each split JSON document | 16 MiB |
| All split documents combined | 48 MiB |
| Project documents | 4,096 |
| Container v2 manifest | 1 MiB |
| Packed file | 64 MiB |

These bounds supplement existing symbol, wire, geometry and library-definition
limits; separating files does not remove those model constraints.

## Folder publication and recovery

Desktop file work runs off the editor thread against an immutable snapshot.
The shared `pcbcraft-storage` crate also gives the headless CLI the same folder
reader and recovery behavior. A nonblocking sibling advisory lock serializes
cooperating opens and saves, and saved baseline bytes detect stale writers.
A busy lock produces an error instead of leaving another operation waiting.
If filesystem permissions prevent opening the lock file for writing, an open
uses an existing readable lock in shared mode, or reads without a lock if none
exists. Both fallbacks require no recovery journal before and after reading;
saves and recovery always require the writer lock. This permits opening a
read-only project and using Save As to place edits in a writable location.
The lock file, `.<name>.pcbcraft-lock` beside the folder or archive, stays after
the project closes. Its leading dot hides it on Linux and macOS; on Windows it is
created with the hidden attribute instead. A lock file created before that keeps
its attributes and still works.

A save stages only changed or removed documents. It writes their new bytes and
old backups into a temporary directory, flushes them, and writes a journal with
relative paths, sizes and hashes. After rechecking the baseline, it atomically
renames the staging directory to `.pcbcraft-save`. This is the commit point:
interruptions before it retain the old project; after it, the next open completes
the new project before reading the project into the editor.

Recovery checks every journal path and old/new payload, verifies that affected
files still contain old or new bytes, and validates the proposed complete project
before applying remaining replacements and deletions. Unexpected external edits
or corrupt recovery data stop recovery without overwriting them. New files are
flushed before replacement, and directories are synced on Unix. Recovery scratch
files stay inside the transaction directory, so interrupted writes cannot leave
unrecognized files in `sheets/` or `library/`.

Once all replacements are complete, recovery renames the journal directory out
of its active location before cleanup. A crash during cleanup therefore cannot
leave an active journal with missing staging files. Leftover staging directories
are ignored and may be removed when no save is running. Interrupted initial saves
can be recovered by opening their folder or entry-point path.

Tests simulate interruption after staging writes, journal publication, each
replacement/deletion and completion. They verify that reopening yields the full
old or new state. These are process-interruption tests on Linux, not hardware
power-loss tests. Directory fsync is currently Unix-specific; other platforms
retain the journal recovery protocol without the same directory-durability
claim. Programs that ignore the advisory lock can still race with publication.

Failed opens leave the active project, revision and dirty state unchanged. Edits
made during a background save stay unsaved when its earlier snapshot completes.
Archive publication retains the existing single-file staging and baseline check.

On Windows, a folder save is refused before anything is written when a document
path needs a name that Windows reads as a device: `con`, `prn`, `aux`, `nul`,
`com1`–`com9` or `lpt1`–`lpt9` before the first dot, for example a pinned symbol
with ID `con` (`con.r2.json`) or a library with ID `aux`. The app reaches folders
through canonical `\\?\` paths, where Windows would write such a file, but Git for
Windows can't add it and programs that open it by its usual path reach the device
instead. Through a usual path the publish itself fails after the journal commit,
and every later open would fail in recovery. The message names the
file; saving as a `.pc.zip` archive works, because the archive stores documents as
ZIP entries. Other platforms save these names as ordinary files.

## Legacy migration and fixtures

Plain JSON formats 1–4, split format 5 and container v1 remain readable. Formats 1–3 resolve the
exact retained starter revisions; format 4 supplies its full symbol copies.
All migrate in memory to the same split documents without changing IDs, geometry,
wire connectivity or footprint mappings. Save asks for a new folder destination
by default; Save As can instead select `.pc.zip`. It never silently replaces a
legacy text `.pc` with an archive or a folder entry point requiring missing files.

The original `two-resistors-format-1.pc`, `two-resistors-format-2.pc` and
`two-resistors.pc` (format 4) fixtures remain legacy inputs. Version-5 split inputs live
under `crates/app/tests/fixtures/split/`, including the folder, a manifested archive
and a hand-zipped archive without a manifest. Generate a fresh comparison set with:

```sh
cargo run --locked -p pcbcraft-engine --example storage_fixtures -- /tmp/new-fixtures
```

The output directory must not already exist. The generator uses the production
encoder; regression tests compare migrations, document boundaries, library-copy
lifetimes and archive round trips. Legacy writers remain available to fixture and
benchmark code, while normal editor saves use the split format.

## Design references

The validation and round-trip requirements draw on independently authored
PCBcraft Next material at revision `568cd495397a91563e563e7163efb226d84fa83a`
under MIT OR Apache-2.0. The proposal uses Tangency's existing session and
terminal model to specify a new fixture.
[Persistence design](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/docs/specs/persistence.md),
[codec reference](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/src/persistence.rs),
[schematic test contracts](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/tests/schematic.rs),
[project test contracts](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/tests/sheets.rs).
