# Native project storage

PCBcraft stores a project as independently editable documents. The editor and MCP
still share one project, command set and undo history; storage boundaries do not
create separate schematic and board sessions.

The [multiboard and sheet instance design](multiboard-and-sheet-instances.md)
and its [storage contract](multiboard-storage.md) describe board folders,
instances and associated-file ownership. The project/16 codec implements
board folders for zero or one board and stable identities for the existing
single-use sheet hierarchy. Repeated instances, additional boards and associated
files remain future work. See the [single-board codec](single-board-storage.md)
and [occurrence codec](sheet-occurrence-storage.md) for the supported boundaries.
Project/17 adds the [sparse net registry](net-registry-storage.md) in those same
instance homes, without changing the working-folder or `.pc` container layout.

## File encodings and save behavior

The working form is a project folder with a plain UTF-8 JSON `project.json` entry
point. Desktop New Project defaults to this form. A project's first Save asks
for a new folder, suggesting the project's name; one still named Untitled takes
the chosen folder or archive name as part of that save, with no undo step.
Cancelling or failing that save leaves it Untitled and unsaved. Open accepts the
folder, its entry point or a `.pc` archive. Save keeps the opened form;
Save As offers a project folder and a packed project. The folder destination is
a new or empty project directory, while archive Save As requires an unused file.
Unrelated root files such as `README.md` and `.git` remain untouched.
Regular `.DS_Store`, `Thumbs.db` and `desktop.ini` files are also left alone inside
project subdirectories; they are not project documents and are never published
or removed by a save. Other unknown files under `sheets/`, `library/` and `assets/` are refused.
Only the exact registered board and instance files are read under the selected board area;
unregistered folders and related user files are left alone and are not packaged.

```text
my-board/
  project.json
  sheets/<sheet-uuid>.json
  boards/<board-slug-and-id>/board.json  # only when a physical board exists
  boards/<board-slug-and-id>/instances.json
  library/symbols/<library>/<symbol>.r<revision>.json
  library/footprints/<library>/<footprint>.r<revision>.json
  assets/                       # reserved for future asset references
```

Opening `project.json` opens the entire project. Copy the folder or export its
`.pc` to share it; the entry point alone does not contain the other documents.
A project rename changes the name in `project.json`, without renaming its files.

The browser uploads packed projects and downloads
`.pc`. It cannot reconstruct a folder from an uploaded entry point alone.
Download encoding runs in batches of 256 records and archive hashing/copying in
256 KiB chunks, allowing the editor to yield between batches. The board-header
phase also writes its bounded setup of at most 64 material revisions, 128 layers,
127 strata and 256 nested prepreg plies. A requested download
does not confirm a durable write and does not clear the editor's dirty state.

## Board folders and first-save migration

New projects use `pcbcraft-project/16`, extending project/14's `layout_root: "."` and required
zero-or-one `boards` registry. Each registration contains its stable `id`, `name`
and portable `folder` segment instead of `file`. Physical data resolves to
`join(layout_root, "boards", folder, "board.json")`, including single-board
projects. The folder is a board-name slug plus 12 hexadecimal UUID characters;
a case-insensitive collision selects the full UUID suffix. Names remain fixed
through display-name changes. Slugs use lowercase ASCII letters/digits, collapse
other runs to hyphens, cap at 48 characters and fall back to `board`. Portable
segments also reject Windows device names and path separators.

Opening a supported flat `project.json` folder or container-v3 package does not
rewrite it. Its first Save migrates even without design edits: one journaled
folder transaction updates the entry point, writes the registered nested board
and removes the root board. The board ID/name and physical payload bytes stay
the same; sheets and pinned definitions retain their exact bytes. Subsequent
unchanged saves produce no diff. Project/14 combines the existing allocation
watermark, component metadata and optional schematic package choices; sheet,
board and library payload versions do not change merely because a path moves.
Project/16 adds one circuit use and one occurrence per sheet, either in the
registered board's `instances.json` or in `project.json.schematic_instances` for
schematic-only projects. First Save also migrates project/14 without moving its
board folder. Unaffected payload bytes and all component/package identities stay
unchanged; the [occurrence contract](sheet-occurrence-storage.md) defines the IDs,
validation and first-board transfer.

The first persistent net record selects `pcbcraft-project/17`. Board projects
then use `pcbcraft-instances/2` with `net_registry` beside the occurrence records;
schematic-only projects place it under `schematic_instances.net_registry` in the
entry point. The header and registry presence must agree, and empty/null
registries are refused. Existing projects without records retain their low-level
encoding; Open and inspection never promote nets. Older readers must refuse
these headers rather than discard identity, lineage or unresolved decisions.

The first named assembly variant selects `pcbcraft-project/18`, a superset of
format 17. Its nonempty `assembly_variants` registry stores stable variant and
board UUIDs in `project.json`; each registered board-local
`variants/<variant-uuid>.json` stores the name and sparse fitted overrides keyed
by stable component ID. Base remains implicit in component metadata, so projects
without named variants retain their existing format and files. First-save board
folder migration moves registered variant documents beside the migrated board,
preserving their bytes and identities. Older readers must refuse format 18 rather
than lose population data.

If any unowned `boards`, `builds`, `documents` or `outputs` name already exists
at the root, save selects an unused `pcbcraft-data-<project-id-prefix>` root,
starting with 12 hex digits and extending the prefix on case-insensitive
collisions. It leaves foreign contents untouched and persists the choice.
Newly occupied namespaces detected while staging fail before journal publication;
a retry chooses a free namespace. Existing registered roots/folders stay fixed.
No empty optional areas or additional boards are introduced. A foreign
`instances.json` beside a registered project/14 board causes migration to refuse
publication, preserving that file and the prior project.

Layout adoption is save metadata, so a successful native save applies it across
Undo/Redo without changing electrical revision or discarding intervening edits.
A failed or cancelled save does not adopt it. Browser Download writes a migrated
copy in bounded encoding batches, while the active session remains unchanged
because a download request cannot confirm durable publication. Older readers
that lack project/16 must refuse it rather than discarding occurrence metadata.

## Schematic-only package choices

Split project format `pcbcraft-project/13` extends project/12's component and
allocation vocabulary for schematic-only package intent. It requires `boards: []`,
the numeric allocation watermark and a non-null, nonempty `footprint_choices`
array after `components`. Each choice has the same stable component ID, exact
library/item/revision and complete terminal-to-pad mapping as an unplaced board
assignment. Full pinned footprint definitions use the existing library paths;
there is still no `board.json`, physical target, placement, outline or stackup.
Choices are ordered by component ID and written incrementally, one per record
budget. Missing assets, duplicate or unknown components, incompatible maps,
null/empty lists and simultaneous board ownership are refused before adoption.

Older split formats reject this field. Project/14, /15 and /16 allow the same nonempty
optional list only with `boards: []`; deleting the last choice omits the field
while retaining their project format. Historical low-level project/13 encodings return to
project/9 or /12 when the last choice is removed, until first-save migration. Explicit first-board creation transfers choices
and pinned definitions to the existing board/7-or-later assignment encoding in
one undoable transaction. Undo restores project-owned choices. Existing board
files keep their current encoding and behavior. Folder and packed saves carry
these choices, while the diagnostic single-document writer refuses lossy export.

Split project format `pcbcraft-project/15` adds an optional `part` selection to
each native part instance, retaining the existing board registration semantics.
New projects keep format 14's `layout_root` and board-folder registry. Supported
historical flat snapshots can acquire native part selections without a physical
package; ordinary first Save adopts board folders and occurrence identities as format 16
and every pinned dependency. A native selection also adopts the numeric ID
watermark, including schematic-only placement in pre-watermark projects.
The small selection references deduplicated `library/parts/`,
`library/categories/` and `library/manufacturer-parts/` documents using the same
`<library>/<id>.r<revision>.json` convention as symbols and footprints. Every
package footprint is pinned, including unselected variants. Missing or
conflicting dependencies reject opening before adoption. Native part selections
also travel with schematic clipboard fragments; diagnostic single-document
export refuses to discard them.

Board format `pcbcraft-board/17` extends the physical vocabulary through board/16
with multi-pad terminal maps and explicitly `unconnected_pads`. Accepted input
uses `pcbcraft-board-baseline/2` for these mappings. A schematic-only extended
package choice uses project/15. Each new format rejects use under an older
version; projects without the new data preserve their earlier no-op encoding.
These formats require the newer app and do not migrate board folders or lists.
See [part-schema.md](part-schema.md) for record and transaction details.

Accepted baseline format `pcbcraft-board-baseline/3` extends formats 1 and 2
with an optional `primary_quantity` on each historical part instance. It uses
the native typed value record, for example
`{"type":"quantity","value":"0.0000001","unit":"F"}`. The decimal and base
unit must be canonical, and their combined text must equal the record's existing
`value`. Other value types, explicit nulls and this field under formats 1/2 are
refused. The snapshot needs no live category or library source to validate it.

Initialization or a real accepted board update captures exact primary quantities
from typed parts and chooses format 3 when any are present; multi-pad/NC mappings
remain supported. Opening, inspecting, saving and accepting a no-op update do not
enrich or upgrade an older baseline. Formats 1/2 omit the new field and retain
their previous bytes on no-op saves. Older apps reject format 3 instead of
discarding its metadata; no enclosing board/project vocabulary changes.

When a part is deleted, format-3 history retains its last visible reference,
canonical value and matching typed quantity as display metadata in the same
undoable deletion. Accepted package and connectivity state remain unchanged
until reviewed removal. Board Properties and the project browser format that
quantity through the live-value formatter. Untyped parts and older baselines
show their stored text, including when the part was already deleted; no type is
inferred or reconstructed from free text. Folder and archive saves carry the
same historical records.

Board format `pcbcraft-board/19` adds validated `pcbcraft-rules/1` board defaults
with a stable UUID, hard limits and preferred routing dimensions. It supports the
earlier physical fields, including format 18's track/via locks, and requires a
rule record. Older headers reject rules. Boards without rules retain their earlier
encoding; removing rules restores the format required by the remaining data.
See [board-rules.md](board-rules.md) for validation, resolution and reset behavior.

## Document ownership

| File | Format | Contents |
|---|---|---|
| `project.json` | `pcbcraft-project/6`, `/7`, `/9`, `/11`, `/12`, `/13`, `/14`, `/15`, `/16`, `/17` or `/18` | Project UUID, name, ordered sheets and components; version 7 adds the numeric allocation watermark, version 9 adds explicit zero-or-one board registration, version 11 adds component metadata for legacy ownership, version 12 combines metadata with explicit board registration, version 13 adds schematic-only package choices, and version 14 combines these fields with `layout_root` and board-folder registrations; version 15 adds native part selections, version 16 adds required single-use occurrence identities, version 17 adds the persistent net registry, and version 18 adds board-local assembly variant references |
| `<layout_root>/boards/<folder>/instances.json` | `pcbcraft-instances/1`, `/2` with a net registry | Matching board UUID header, one circuit use and one occurrence per sheet, optional sparse net registry; schematic-only projects embed these records in `project.json` instead |
| `<layout_root>/boards/<folder>/variants/<variant-uuid>.json` (or `variants/<variant-uuid>.json` for a flat board) | `pcbcraft-assembly-variant/1` | Stable variant and board UUIDs, editable name and sparse fitted overrides keyed by stable component ID |
| `sheets/<sheet-uuid>.json` | `pcbcraft-sheet/1` through `/5` | Sheet UUID, unit placements, junctions and wires; version 2 adds hierarchical labels and child sheet symbols; version 3 adds named electrical anchors; version 4 adds per-unit field appearance; version 5 adds net-name appearance |
| `<layout_root>/boards/<folder>/board.json` (historically `board.json`) | `pcbcraft-board/1` through `/19` | Footprints; version 2 adds layer/stackup setup, version 3 adds material reports and construction; version 4 supports footprint orientation; version 5 adds pinned physical footprint references and terminal mappings; version 6 stores physical silkscreen settings; version 7 separates component package assignments from placements; version 8 is the earlier local rectangle; version 9 stores layer-addressed Board Outline graphics; version 10 explicitly enables accepted update baselines; version 11 distinguishes pending desired package assignments from accepted geometry; version 12 stores copper tracks; version 13 adds through vias; version 14 adds bottom-side footprints; version 15 adds local reference offsets and printed text/appearance overrides; version 16 adds physical stroke widths on non-copper graphics; version 17 adds multi-pad and NC mappings; version 18 adds copper locks; version 19 adds board defaults |
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
sheet belongs to user-local presentation state and is never saved in `project.json`.

Version 5 remains readable with exactly one sheet. Its original default name
`Schematic`, historically displayed as Main, migrates to `Main`; other names stay
intact. Formats 1–4 migrate to one Main sheet with their original sheet UUID.
Legacy snapshots retain version 6 until numeric allocation or retirement adopts
the version-7 watermark, an eligible physical edit persists version-9 board
registration with the same compatibility ID, or component metadata requires
version 11. New projects start at version 16 with no physical board, unless New PCB
explicitly requests one. Supported flat and project/14 and /15 encodings upgrade to project/16 on Save. Component metadata with explicit zero-or-one ownership
uses version 12, which retains the watermark and registry; version 11 keeps legacy
implicit ownership and rejects a `boards` record. Version 9 rejects component
metadata rather than interpreting version-12 fields. Sheet encodings remain
sheet/1 for ordinary sheets, sheet/2 for
sheets with hierarchy records and sheet/3 for sheets with named anchors, retaining existing board encodings. The low-level codec preserves original version-5 document bytes until an
entry-point change, while ordinary Save adopts project/16 and retains unchanged
payload bytes. Subsequent sheet changes keep the adopted folder layout. Legacy single-file encoding refuses
multiple or renamed sheets, hierarchical labels, child symbols, named net anchors,
component metadata, field appearance and adopted occurrence identities rather than discarding them.

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
edits stay in `project.json`; label placement edits stay in the owning sheet.

Sheet/5 extends sheet/4 with optional non-null `text` on each named anchor. It uses
the same field-layout vocabulary and bounds. Offsets are relative to the electrical
anchor in its unrotated space; the text angle is explicit and its height scales
with zoom. Missing `text` preserves the automatic layout and old sheet encoding.
Only sheets with authored net-name appearance use version 5; older readers reject
that version. Appearance edits do not move contacts or alter net names.

### Named net records

Sheet/3 extends sheet/2 with a required, non-null `net_labels` array, one record
per line in stable object-ID order. Each record contains a project-unique `id`,
case-sensitive `name`, `style` (`local` or `global`), `at`
point in integer nanometers and `rotation` (0, 90, 180 or 270 counterclockwise).
Names contain 1–200 characters without surrounding whitespace or controls; each
sheet permits at most 256 text labels. Power symbols are ordinary components and
unit placements with pinned native definitions and terminal endpoints. The
former `power`, `power_bar` and `ground` annotation styles are no longer accepted;
the decoder rejects them without migrating or silently omitting records.

An explicit wire endpoint `{"net_label":<id>}` refers only to an anchor owned by
that sheet. A named anchor also contacts exact coincident pins and points on
wire segments; it is an authored connection marker, so placing one at a crossing
joins the contacted routes. Other crossings and coincident distinct endpoints
retain their existing behavior. Names join within a sheet, including its
hierarchical interfaces; global labels and definition-declared power terminals
additionally join matching global names across sheets. Derived net names and graphs are never persisted as
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
projects have a physical board document even when no footprints are placed;
its path follows the registration after folder adoption. The board registry
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
Layers in this section are technical Board Outline additions for boards without a
matching configured layer; configured boards reference their existing layer UUIDs.
Creating graphics does not invent copper stackup or change footprint bindings.
Each path has 2–256 vertices (at least three when closed), no adjacent duplicates,
and no repeated closing vertex. The codec validates non-nil unique drawing IDs,
layer references and bounded resources (512 drawings, 4096 total vertices and
128 combined layers); records sort by UUID. Optional setup, silkscreen, assignments
and placements retain their existing validation and identities. Empty technical
metadata remains after deleting the last graphic, preserving layer addresses.

Board format 12 adds a `tracks` array and may carry every earlier field, so routing
a board never drops its baseline, pending intent, graphics or setup. Each record is
one straight segment on its own line: a UUID `id`, a copper `layer` UUID from the
setup, an integer `width_nm` from 10,000 to 10,000,000, and integer-nanometer
`start` and `end` points that differ. Records sort by UUID, so adding or deleting a
segment changes only its own line. The codec rejects duplicate IDs, non-copper or
unknown layers, out-of-range widths and more than 4096 segments. Format 12 requires
at least one track and rejects `vias`, so readers released with it keep working.

Board format 13 adds a `vias` array in the same style and is otherwise format 12: a
UUID `id`, an integer-nanometer `at` point, and `diameter_nm` and `drill_nm`, with the
drill at least 50,000 and smaller than the pad and the pad at most 10,000,000. A via
spans every copper layer and needs a setup with at least two; a board holds at most
2048. Format 13 requires at least one via, and its `tracks` array is optional. Either
array is omitted when empty, and formats before 12 reject both fields. Deleting the
last via returns a board to format 12, and deleting the last track and via returns it
to the format its other data requires, with the same bytes as before the first route.
Net membership is not stored; it is derived from copper geometry and the accepted nets.
Legacy single-file export refuses tracks and vias.

Board format 14 marks bottom-side footprints with `"bottom": true` on their placement
records and is otherwise format 13, so tracks and vias stay optional. It requires at
least one bottom-side footprint, and earlier formats reject the field; top-side records
omit it. A bottom footprint mirrors its local X before rotation, and its surface pads
and technical graphics use the back layers. Flipping the last footprint back to the top
returns the board to the format its other data requires, with the same bytes as before.
Legacy single-file export refuses bottom-side footprints.

Board format 15 adds optional `reference_offset` and `reference_text` fields on a
footprint placement. The offset is measured in local nanometers before rotation
or side mirroring. The sparse print object overrides words, font, paired text
height/stroke, local rotation, alignment and visibility while retaining the
component's identity and reference. Format 15 requires at least one nonzero
offset or nonempty print override. Resetting the last of both restores the
format required by the board's other data.
The [footprint format specification](footprint-schema.md) defines its placement schema and coordinate contract.

Board format 16 adds optional integer `stroke_nm` to layer graphics, bounded
from 10,000 to 10,000,000 nanometres. New non-copper technical graphics require
an explicit width; omitted widths retain legacy Board Outline hairlines, and
explicit nulls are rejected. The format carries every field supported by format
15, including reference-position and print overrides, and requires at least one
stroked graphic. Older formats reject the new field. Graphics reference existing
non-copper layer UUIDs; conductive copper linework uses the existing track records
and their connectivity/DRC/export rules. Removing the last stroked graphic returns
the board to the format required by its remaining data. Unchanged legacy boards
retain their previous encoding and bytes.

Board format 18 adds optional `locked: true` on track and via records and
requires at least one authored lock. It is a superset of format 17, including
multi-pad mappings, physical graphics, accepted baselines and pending intent.
Absent lock flags mean false; canonical saves omit false. Existing boards with
no locks retain their previous supported encoding and no-op bytes. Unlocking the
last object returns to the format required by its remaining data. Lower format
headers carrying active locks are refused, and older readers refuse format 18.
Flags survive folder/archive reopen and undo/redo without changing object IDs,
geometry or connectivity. The shared lock command and protection policy are in
[the layout workflow](pcb-layout-workflow.md#fixed-tracks-and-vias).

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
or value changes only its component record in `project.json`. Board geometry stays
in `board.json`. Placing a new definition changes the project and sheet documents
and adds its saved symbol file. After the first layout migration, a no-op save rewrites no project documents.
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
*.json text eol=lf
*.pc binary
```

Ignore `.pcbcraft-save/`, `.pcbcraft-stage-*/` and `.*.pcbcraft-lock`. ZIP exports
are binary; optional text conversion can make local diffs readable but does not
provide normal text merging. The folder is the collaboration format.

Board inspection reports the resolved project-relative physical path, including
any custom layout-root prefix, in each board's `file` field. Diagnostic
single-document files do not carry layout metadata and continue to report the
flat compatibility path.

## Packed projects and manifests

A `.pc` contains the same relative document paths and a root `manifest.json`:

```json
{
  "format": "pcbcraft-container/3",
  "parts": [
    {"path": "boards/board-a1b2c3d4e5f6/board.json", "size_bytes": 66, "sha256": "<SHA-256 of the exact bytes>"}
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
`my-board/project.json`, is stripped; multiple roots and unrelated files are
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
are bounded by both declared and actual expanded size. An unchanged save of an already migrated,
manifested archive preserves its original bytes, including compression and
comments. A first save of a manifest-less archive intentionally adds the manifest.

| Bound | Limit |
|---|---|
| Legacy single JSON document | 4 MiB |
| Each split JSON document | 16 MiB |
| All split documents combined | 48 MiB |
| Project documents | 4,096 |
| Container v3 manifest | 1 MiB |
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
file; saving as a `.pc` archive works, because the archive stores documents as
ZIP entries. Other platforms save these names as ordinary files.

Recovery binds each document parent before checking and publishing its leaf.
Unix traversal and replacement use no-follow directory-relative operations;
Windows retains verified non-reparse directory handles without delete sharing
until publication completes. A parent swapped after resolution cannot redirect
the replacement to an external symlink target. A final visible-tree check stops
adoption and retains the journal if registered paths no longer match the completed
document set. Restoring the original directory permits ordinary recovery.

## Compatibility and fixtures

App version 0.1.0 introduces this layout. Earlier working
folders used a plain JSON `project.pc` entry point, and split archives used
`.pc.zip` with container version 2. Older single-document projects could also
use `.pc` for plain JSON or container version 1. Normal desktop, browser and
headless opens accept the immediately previous split layouts without rewriting
their source files. Previews are read-only too. On desktop, first Save upgrades
a split folder in place: one journaled transaction installs `project.json`, removes
`project.pc` and migrates any flat board payload. Desktop Save of a previous split
archive replaces it with a verified container-v3 package at its existing path.
In the browser, Download creates a new upgraded `.pc` file; it leaves the uploaded
archive unchanged. Reopen the downloaded file to use the saved copy.
Standalone JSON/container-v1 projects remain unsupported. The optional explicit
converter below creates a separate copy of the immediately previous split layout.

The fixed `project.json` filename identifies the new unpacked layout, and packed
projects use `pcbcraft-container/3`. Existing split document schema versions
remain supported under the new entry-point path; sheet, board and library
schemas are unchanged. New hand-zipped folders use `project.json`; previous
`project.pc` split ZIPs are also accepted for upgrade. Desktop Save or browser
Download writes a version-3 manifest. Container versions 1/2
are accepted only for version 2 with the previous `project.pc` split paths;
version 1 remains unsupported. ZIP contents are
validated independently of the filename, so a current version-3 package can
still be opened through an explicit path with another extension.

### Convert a previous split project

Build `pcbcraft-cli` and run either form from a terminal:

```sh
# New working folder (the destination must not exist).
pcbcraft-cli convert /path/to/old-project /path/to/converted-project
# New portable ZIP package.
pcbcraft-cli convert /path/to/old-project.pc.zip /path/to/converted-project.pc
```

A source folder or its `project.pc` entry point must contain a supported split
project. Archives must use container version 2, or be manifest-less ZIPs of that
previous split layout. A destination ending in `.pc` creates a container-v3 ZIP;
other destinations create a new folder. Both forms validate all managed documents
and preserve stable IDs, pinned libraries and exact physical/sheet/library payload
bytes. The entry point becomes project/16 with a board-folder registration and
single-use occurrence records; the entry-point path, physical board path and
packed manifest change. Unrelated root files such
as notes and `.git` are left in the source rather than copied into the project.

The converter never writes to or recovers the source. Old manifest sizes and
SHA-256 hashes are checked before renaming its entry-point path. Mixed layouts,
invalid or unsupported schemas, unsafe paths, symlinks, busy source locks and
pending save recovery are refused. A pending legacy save must be recovered with
a previous build before conversion. Existing destinations and paths inside the
source folder are refused. Folder publication uses the normal recovery journal;
an interrupted conversion can leave a new destination requiring recovery. It
never replaces an existing project. Open either the converted copy or the old
split source; saving the latter upgrades it in place.

Normal Open and explicit conversion also upgrade retired `power`, `power_bar`
and `ground` annotations to pinned native power symbols. The affected sheet and
project metadata change; unrelated board, sheet and library bytes are preserved.
Names, positions, rotations, junctions and authored wire routes survive. This
converter does not translate standalone JSON/container-v1 projects.

The original `two-resistors-format-1.pc`, `two-resistors-format-2.pc` and
`two-resistors.pc` fixtures remain historical diagnostic inputs. The low-level
single-document codecs, explicitly named `decode_diagnostic_file` compatibility
reader and container-v1 fixture writer remain available for codec tests and
benchmarks; they are not supported app/CLI import paths.
Split inputs under `crates/app/tests/fixtures/split/` now use `project.json`
and `.pc` packages, retaining their original document bytes. The checked-in
version-5 document exercises unchanged Open followed by first-Save migration;
low-level codec tests separately retain its original encoding. Generate a fresh canonical comparison set with:

```sh
cargo run --locked -p pcbcraft-engine --example storage_fixtures -- /tmp/new-fixtures
```

The output directory must not already exist. The generator uses the production
encoder; regression tests compare document boundaries, library-copy lifetimes,
archive round trips and the compatibility boundary. See [release notes](../CHANGELOG.md).

## Design references

The validation and round-trip requirements draw on independently authored
PCBcraft Next material at revision `568cd495397a91563e563e7163efb226d84fa83a`
under MIT OR Apache-2.0. The proposal uses Tangency's existing session and
terminal model to specify a new fixture.
[Persistence design](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/docs/specs/persistence.md),
[codec reference](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/src/persistence.rs),
[schematic test contracts](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/tests/schematic.rs),
[project test contracts](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/tests/sheets.rs).
