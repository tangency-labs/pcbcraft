# Native project storage

PCBcraft stores a project as independently editable documents. The editor and MCP
still share one project, command set and undo history; storage boundaries do not
create separate schematic and board sessions.

The [multiboard and sheet instance design](../design/multiboard-and-sheet-instances.md)
and its [storage contract](../design/multiboard-storage.md) describe board folders,
instances and associated-file ownership. The project/16 codec implements
board folders for zero or one board and stable identities for the existing
single-use sheet hierarchy. Repeated instances, additional boards and associated
files remain future work. See the [single-board codec](single-board-storage.md)
and [occurrence codec](sheet-occurrence-storage.md) for the supported boundaries.
Project/17 adds the [sparse net registry](net-registry-storage.md) in those same
instance homes, without changing the working-folder or `.pc` container layout.
Project/20 adds the optional-at-project-level [net-class catalog](net-classes.md)
in the entry point.
[Designator locks](#designator-locks) were added in project/51 and combined in
project/56 with [alternate pin functions](#alternate-pin-function-records), retaining the
optional records and membership representations of the earlier envelopes.
Project/22 adds [project parameters](#parameter-records); project/23 adds generated
net numbers and can also store project parameters.
Project/24 adds the [part-group catalog](part-groups.md); its expanded
memberships live in the instance home. Historical instances/5 stored expanded
memberships and instances/7 added the compact UUID dictionary. Project/45 added
[explicit differential pair records](net-registry-storage.md#differential-pair-records)
to the registry alongside either membership representation. The
[current instance format](#board-instance-records) accepts these records
together and keeps the same header when optional records are removed.

Project/23 adds persistent generated net numbers to the same instance registry,
using instances/4 in its historical board-owned form. A stored record's positive `generated_number`
is unique across live and historical identities. This envelope can also carry
class catalogs, assignments, project parameters and no-connect intent. Older
envelopes reject the number field. Migrating the entry point alone does not
assign numbers to legacy nets. See
[net registry storage](net-registry-storage.md).

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

### Native write policy

Opening or inspecting a supported older project leaves its files untouched.
Folder saves and packed exports write `pcbcraft-project/61` for `project.json`,
and `pcbcraft-sheet/10` for each schematic sheet. A registered board uses
`pcbcraft-board/42` for `board.json` and `pcbcraft-instances/9` for `instances.json`.
Version 61 uses the version 56 record vocabulary with optional designator
locks. Earlier formats remain readable. The first save adopts the existing
folder and occurrence migrations if needed. Subsequent unchanged saves are
byte-stable, and clearing an optional record or undoing the edit that added it
keeps the current format.

Use one current write format per native document type. Add a version when the
storage contract changes, and represent optional features as optional records
within that version. Preserve stable identities and deterministic ordering.
Pinned library items retain their published format and revision. Changes to
authored library items go through publication of a new revision.

The project entry point and native library authoring use fixed current write
formats for [symbols](../libraries/symbol-schema.md),
[footprints](../libraries/footprint-schema.md) and [parts](../libraries/part-schema.md).
Placing a version-5 symbol in an older project adopts the folder and occurrence
metadata in the placement edit. Board setup profile writers still select
formats by the records present.

## Bundled library address migration

Native desktop/browser opens and headless storage opens normalize recognized,
unchanged `starter` symbols and footprints to the retained `basic-components`
revision with identical geometry. Component and sheet references, reusable-part
dependency addresses, package choices, physical placements and qualified accepted
board references move together. Revision numbers, terminal/pad IDs, mappings,
poses, copper and accepted capture IDs/digests are preserved. Historical logical
references with no remaining symbol body are retained rather than inferred.
Customized symbol/footprint content and occupied conflicting target addresses
keep their original identities. Power is unaffected.

Opening prepares this upgrade in memory after validation and does not write any
project files or add an electrical undo step. The editor retains its original
saved snapshot and bytes for stale-file checks. Save publishes the upgraded
references through the ordinary journaled folder or archive path; browser
Download encodes the same candidate without claiming durable publication.
Unrelated documents retain their original formatting. Reopening a migrated
project and saving it unchanged produces no additional diff. Raw decoding and
journal validation remain byte-preserving; hosts call the shared
`migrate_bundled_libraries` opening step explicitly.

## Sheet paper adoption

New sheets have paper (see [Sheet paper records](#sheet-paper-records)). Opening
a sheet with no saved paper gives it the smallest ISO A paper
selected to fit its drawing with a 20 mm margin, and the Default frame.
Landscape is tried before portrait at each size; a drawing larger than A0 gets
A0 landscape. Paper is centered around the drawing, with its top-left corner
snapped to the 1.27 mm grid and clamped to the schematic coordinate range. Near
those limits it may not be centered; a size is chosen only if the clamped paper
still holds the drawing and margin and the frame's title block covers none of
the drawing. Drawing extends outside the paper only with the A0 fallback. An
empty sheet gets the new-sheet paper. Nothing on the sheet moves.

Like the library migration, this happens in memory when the desktop app, browser
or headless storage opens a project. It adds no undo step and doesn't mark the
project as changed; the open notice says the next Save (or Download) will add
paper to sheets that lacked it. An ordinary save rewrites each older sheet
document once as sheet/10, including sheet/9 documents that already had paper.
`prepare_opened` runs both opening migrations. `adopt_paper` supplies missing paper.

## Board folders and first-save migration

The current project entry point uses the folder layout and occurrence records
introduced in `pcbcraft-project/14` and `/16`, including `layout_root: "."` and a
required zero-or-one `boards` registry. Each registration contains its stable `id`, `name`
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

Unnumbered legacy net records use `pcbcraft-project/17`. Board projects
then use `pcbcraft-instances/2` with `net_registry` beside the occurrence records;
schematic-only projects place it under `schematic_instances.net_registry` in the
entry point. The header and registry presence must agree, and empty/null
registries are refused. Open and inspection never promote nets. Older readers must refuse
these headers rather than discard identity, lineage or unresolved decisions.

Earlier project versions identified the records present. Version 20 added a
nonempty class catalog, /21 [net-class bindings](net-class-assignments.md),
/22 parameters, and /23 generated net numbers. Versions 24 and 39 added
expanded and compact part-group memberships, /36 assembly profiles, /45
differential pairs, /48 alternate pin functions, /51 active designator locks,
and /56 both alternate pin functions and active designator locks. Board-owned
bindings use instances/3, instances/4 with generated numbers, instances/5 or /7
with expanded or compact memberships, or instances/8 with pairs. Bindings
remain inside the same logical registry and carry class and origin net UUIDs,
while physical board and sheet formats stay unchanged. Older headers refuse the new vocabulary, and project/21
requires actual bindings.

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
[Device supply ranges](../libraries/part-schema.md#device-supply-range) live in the same
pinned generic part closure. Historical ratings use `pcbcraft-part/4` or
`pcbcraft-part/5`, and current part revisions use `pcbcraft-part/6`. Folders
and packed projects carry the same bytes.

Board format `pcbcraft-board/17` extends the physical vocabulary through board/16
with multi-pad terminal maps and explicitly `unconnected_pads`. Accepted input
uses `pcbcraft-board-baseline/2` for these mappings. A schematic-only extended
package choice uses project/15. Each new format rejects use under an older
version; projects without the new data preserve their earlier no-op encoding.
These formats require the newer app and do not migrate board folders or lists.
See [part-schema.md](../libraries/part-schema.md) for record and transaction details.

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

Accepted baseline format `pcbcraft-board-baseline/4` retains formats 1–3 and
requires a bounded nonempty `display_name` on each accepted net. This freezes
generated reference/pin text and project-wide occurrence qualification separately
from authored `names` and historical `terminals`; it does not add logical registry
memberships. Capturing nets through initialization or reviewed Update PCB selects
at least format 4. Empty-net baselines retain the earlier version selected by their other
data. Formats 1–3 reject display metadata, and format 4 rejects missing names,
control characters and names over 65,536 UTF-8 bytes, within the existing 16 MiB
baseline budget. Existing baselines are not enriched by inspection or Save.
A review may explicitly adopt readable-name metadata from an older accepted
snapshot; ordinary subsequent no-op updates retain the accepted capture.

Accepted baseline format `pcbcraft-board-baseline/5` adds `origin` only to
unnamed one-terminal nets (`schematic` or `pad_only`). Those nets have the same
membership shape but only the former can correspond to a schematic net ID. New
captures use format 5 only when at least one net needs this distinction. Formats
1–4 remain readable without changing their bytes on open or no-op Save; the
origin of their unnamed singletons is unknown. The enclosing board and project
formats do not change.

When a part is deleted, formats 3 and later retain its last visible reference,
canonical value and matching typed quantity as display metadata in the same
undoable deletion. Accepted package and connectivity state remain unchanged
until reviewed removal. Board Properties and the project browser format that
quantity through the live-value formatter. Untyped parts and older baselines
show their stored text, including when the part was already deleted; no type is
inferred or reconstructed from free text. Folder and archive saves carry the
same historical records.

### Current board records

All normal board saves and packed exports use `pcbcraft-board/42`. The
`footprints` array is required and may be empty. Setup is optional, so a
bare board or an older marker board can save without acquiring a stackup.
Authored rules, zones, copper, locks, origin, profiles, groups and package
records use their existing field vocabulary and prerequisites. Explicitly
stored optional arrays that require content, such as tracks, vias, zones and
named groups, remain nonempty. Clearing any of those records keeps the board
header at version 42.

The first Save, Save As or browser Download of an older board writes version 42
to its registered path. Pinned assets, accepted baselines, and object identities
retain their original content. Subsequent no-op saves preserve the current
bytes, including valid custom formatting. Explicit legacy layout conversion
still moves the historical board bytes intact, as specified in the conversion
contract above. The sections below describe the historical readers.

Board format `pcbcraft-board/19` adds validated `pcbcraft-rules/1` board defaults
with a stable UUID, hard limits and preferred routing dimensions. It supports the
earlier physical fields, including format 18's track/via locks, and requires a
rule record. Earlier historical headers reject rules.
See [board-rules.md](../pcb/board-rules.md) for validation, resolution and reset behavior.

Board format `pcbcraft-board/21` adds a `pcbcraft-origin/1` placement origin with a
stable UUID and one anchor: `board_zero`, `outline_lower_left`, or a `point` with
in-range `x_nm`/`y_nm` board coordinates. It accepts board rules and every earlier
physical field, including optional zone outlines, and requires the origin record.
Earlier historical headers reject it. Boards without an origin measure outputs
from board zero.

### Named layer groups

Historical board format `pcbcraft-board/41` requires a board setup with at
least one named group in `setup.stackup.layer_groups`. It retains every earlier authored board field, including teardrops,
circular copper and footprint locks. Within `setup.stackup`, `layer_groups` stores
only authored named groups (group UUID, name and copper-layer UUIDs); derived
groups are computed and never stored. Earlier headers reject `layer_groups`, even
when empty. In format 42 the field is optional and disappears when the last
group is removed. Formats 27–33, 36 and 38–40 remain unsupported.

### Straight-via teardrop intents

Board format `pcbcraft-board/37` requires a copper board setup and a nonempty
`teardrops` array. It accepts existing straight-board fields, including footprint
locks. Each record stores a stable UUID, the attached via and track UUIDs, and their sizing
parameters. Vertices, net membership and rule reports are never written to the
board. Encoding orders the records by UUID; decoding rejects duplicate
identities/sites, dangling attachments and unrepresentable parameters or shapes.
Older board formats reject the new array. Format 37 does not permit circular
tracks. Format 42 stores teardrops as an optional array.
See [straight via teardrops](../pcb/teardrop-records.md) for the command and current limits.

### Placed-footprint lock metadata

Board format `pcbcraft-board/34` adds optional boolean `locked` on placed
footprints and requires at least one `true` footprint lock. Missing means false.
Repeated `locked` members, null and nonboolean `locked` values are refused.
Format 34 works with or without setup when the other records allow it. For example,
material reports, via assignments, stack fields, assembly profile adoptions,
origin, rules, zones, silkscreen and copper retain their existing validation.
Formats 34, 35, 37, 41 and 42 accept active locks. Readers of other formats
accept an explicit `false` on an otherwise valid placement. Fresh canonical
encoding omits false. Native Save can retain an unchanged current board
document verbatim, including explicit false.

Historical format 26 requires [circular copper records](../pcb/circular-copper-records.md)
and rejects active footprint locks. Format 34 requires an active footprint lock
and rejects arcs. Format 35 requires both features. Format 42 allows them to
be added or removed independently.

The codecs store the flag alongside the existing placement ID, pose and pinned
definition reference. Geometry and connectivity use the same resolution paths
as for unlocked placements. Host Open and Save still apply the existing
migrations described above.

See [the layout guide](../pcb/locks.md#placed-footprint-locks) for the
placement commands that respect this flag. There is no footprint lock command or
GUI toggle yet. Diagnostic single-file encoding and the board clipboard explicitly
refuse active footprint locks rather than silently losing them.

### Assembly profiles and stack fields

Project format `pcbcraft-project/36` adds a required nonempty `assembly_profiles`
catalog and supports the earlier project fields, including optional parameters,
net classes, generated net numbers and an optional part-group catalog. When groups are present, expanded or compact memberships retain their existing instance headers and representation under project format 36. The catalog has identity-sorted `heads`
(`profile`, positive content `revision`, `removed`) and `revisions`, sorted by
profile UUID and content revision. An optional `revision_watermarks` list contains identity-sorted `{profile, revision}` reservations strictly above the corresponding authored head, or for identities whose pin has been undone. Its IDs are non-nil, revisions positive, and identities unique; the existing 64-identity limit counts the union of heads and reservations. Earlier files without this field derive their issued revisions from their heads. Each pinned record has `format:
"pcbcraft-assembly-profile/1"`, non-nil `id`, and `content`. Schema and content
versions are distinct. Content includes a positive 32-bit `revision`, `name`,
`provenance` (`authored`, or `imported` with a `source` hint), optional `vendor`
metadata, required `bottom_side` (`viewed_from_top` or `viewed_from_bottom`) and
optional nonempty `corrections` sorted by `library` then `definition`. Metadata
strings are 1–64 characters without surrounding whitespace or controls. Active
latest names are unique. Each correction has a rotation (1–359, omitted at zero,
never explicit zero or null) and/or nonzero centroid (integer nanometres, each
axis within 100 mm, never null). IDs, revisions and correction identities are
unique; unknown fields are refused. Stored numeric fields use canonical JSON
integer tokens (`0`, not `-0`, for zero), without decimal points or exponents;
shared commands additionally normalize integer-valued decimal spellings before
writing canonical records.

Board format `pcbcraft-board/22` adds required nonempty `assembly_profiles`:
identity-sorted `{profile, revision}` adoption records resolving exact pinned
content. It supports optional origin, rules, zones and earlier fields. Extended
stack fields, explicit via assignments, material reports, circular copper arcs
and footprint locks require at least formats 23, 24, 25, 26 and 34, respectively.
These higher formats accept adoptions, and headers before 22 reject them.
Adoptions are optional in format 42.

The project retains at most 64 identities (including retired watermarks), 128
content revisions and 4096 corrections across retained revisions. An edit retires
identities without invalidating adoptions; cleanup discards only unused
superseded or retired payloads in that transaction. Watermarks prevent reuse of
cleaned revisions. Undo/Redo preserve the greatest issued revision in this project
history without retaining undone payloads or changing restored board adoptions.
Project/61 retains catalogs with issued watermarks even after the last unused
payload is released. The catalog uses the same validation rules and leaves the
membership representation, folder and occurrence metadata intact. Each saved
copy uses its own recorded lineage. Immutable revision payloads are shared
with Undo snapshots. No external file or network is needed after adoption.
See the [shared commands](../automation/board-commands.md#assembly-output-profiles) for pin/preview/adopt behaviour.

Board format `pcbcraft-board/23` adds the stack fields: copper-stratum roles, base
copper and the tolerance and basis of finished copper thickness, and the stackup's
declared finished board thickness, tolerance and minimum. It accepts a placement
origin, assembly profile adoptions, board rules and every earlier physical field, and requires at least one of
the new fields. Older headers, including 21, reject them. Format 42 supports
these fields when present. Setup profiles use
`pcbcraft-setup-profile/2` for stack extensions without material reports. With
reports they use profile version 3, which can also carry the stack extensions. See
[board-materials.md](../pcb/board-materials.md#copper-strata-and-board-thickness).

Board format `pcbcraft-board/24` adds optional scoped `net_assignment` objects to
vias and requires at least one assignment. It accepts optional assembly profile
adoptions, extended stack fields and earlier board fields; no derived membership
is saved. Earlier headers, including assembly-profile board/22 and stack board/23,
reject via intent. Format 42 omits the assignment in Automatic mode.
Null and nil addresses are refused. Historical unresolved
targets remain readable. See [via net assignments](../pcb/via-net-assignments.md).

Board format `pcbcraft-board/25` adds thermal conductivity, thermal expansion,
dielectric strength and CTI reports to embedded materials. It supports every earlier
field, including stack fields, assigned via intent, assembly profiles, a placement origin and board rules, and
requires at least one such report. Older headers reject them. Formats 26, 34 and
35 can retain reports. Reports are optional in format 42.
Setup profiles with these reports use `pcbcraft-setup-profile/3`.
See [board-materials.md](../pcb/board-materials.md#thermal-and-insulation-reports).

### Zone outline records

Board format `pcbcraft-board/20` adds a required nonempty `zones` array and accepts
all board/19 fields, with board rules optional. A zone board with a placement
origin uses at least format 21; assembly profiles, extended stack fields, assigned
vias, material reports, circular copper arcs and footprint locks require at least
historical formats 22, 23, 24, 25, 26 and 34, respectively. Format 42 preserves
all of these records when present. An absent array in board/20, an empty or
null zone array in any supported format,
or zones in a format below 20, is refused. Zones are
UUID-sorted, one authored record per line, with `id`, `layer`, `net`, `points` and
`options`; no derived fill is stored. Layer UUIDs resolve to copper on that board,
and net UUIDs resolve to its persistent registry, including retained ambiguous
and retired identities. Inspection reports their state without retargeting them.

The [zone guide](../pcb/copper-zone-outlines.md#storage-and-limits) defines the integer
geometry and settings bounds. Options require priority, thermal gap, spoke width,
connection style and minimum island area; omitted clearance inherits the board's
rules. Explicit stored null and unknown fields are refused. Diagnostic single-file
encoding refuses zone intent; folders, archives and recovery use the production
split codec. Deleting the last zone removes the extension without dropping the
registry's retained identities.

## Document ownership

Plated slots change the pinned footprint document, not the board placement or
accepted baseline record. A footprint with at least one slot uses format 5 in
both folder and archive saves. The placement still refers to the same
library/item/revision key; a save without slot or non-plated hole data does not
advance the footprint format.

| File | Format | Contents |
|---|---|---|
| `project.json` | `pcbcraft-project/61` | Project UUID, name, numeric allocation watermark, board registration, ordered sheets and components, and optional authored records. Earlier project formats /5–/7, /9, /11–/17, /19–/24, /36, /39, /45, /48, /51 and /56 remain readable. |
| `<layout_root>/boards/<folder>/instances.json` | `pcbcraft-instances/9` | Board UUID, one circuit use and one occurrence per sheet, optional net registry and either membership representation. See [board instance records](#board-instance-records). |
| `sheets/<sheet-uuid>.json` | `pcbcraft-sheet/10` | Sheet UUID, hierarchy, named anchors, unit placements, junctions and wires, with optional appearance, intent and page records. Versions /1–/9 remain readable. See [current sheet records](#current-sheet-records). |
| `<layout_root>/boards/<folder>/board.json` (historically `board.json`) | `pcbcraft-board/1` through `/26`, `/34`, `/35`, `/37` or `/41` | Footprints; version 2 adds layer/stackup setup, version 3 adds material reports and construction; version 4 supports footprint orientation; version 5 adds pinned physical footprint references and terminal mappings; version 6 stores physical silkscreen settings; version 7 separates component package assignments from placements; version 8 is the earlier local rectangle; version 9 stores layer-addressed Board Outline graphics; version 10 explicitly enables accepted update baselines; version 11 distinguishes pending desired package assignments from accepted geometry; version 12 stores copper tracks; version 13 adds through vias; version 14 adds bottom-side footprints; version 15 adds local reference offsets and printed text/appearance overrides; version 16 adds physical stroke widths on non-copper graphics; version 17 adds multi-pad and NC mappings; version 18 adds copper locks; version 19 adds board defaults; version 20 adds authored zone outlines; version 21 adds the placement origin; version 22 adds explicit assembly output profile revision adoptions; version 23 adds copper roles, base copper, copper thickness qualifiers and finished board thickness; version 24 adds explicit via net intent with optional profile and stack fields; version 25 adds thermal conductivity, thermal expansion, dielectric strength and CTI reports with optional earlier fields; version 26 adds exact circular copper tracks; version 34 adds placed-footprint lock metadata (storage only), with reports and earlier fields optional; version 35 carries arcs and active footprint locks together; version 37 adds straight-via teardrop intents and does not permit arcs; version 41 adds named copper-layer groups (written only when named groups exist) and retains earlier extensions; versions 27–33, 36 and 38–40 remain unsupported |
| `library/symbols/<library>/<symbol>.r<revision>.json` | `pcbcraft-symbol/1` | One full pinned symbol copy, including its library ID |
| `library/footprints/<library>/<footprint>.r<revision>.json` | [Footprint versions](../libraries/footprint-schema.md#compatibility-and-versioning) | One full pinned physical footprint copy, including provenance, copper lands and finished holes |

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
metadata rather than interpreting version-12 fields. Ordinary Save adopts the
current schemas and supported folder layout. Subsequent sheet changes keep
the adopted folder layout. Legacy single-file encoding refuses
multiple or renamed sheets, hierarchical labels, child symbols, named net anchors,
component metadata, field appearance, mirrored placements, live generated
named-contact legs and adopted occurrence identities rather than discarding them.

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

### Board instance records

A registered board's `instances.json` uses `pcbcraft-instances/9` with
`project.json` at project/61. Its `board` UUID must match the registration.
The instance document requires one `circuit_uses` entry and a
`sheet_occurrences` entry for each sheet. It may carry `net_registry`,
including class bindings, generated numbers or differential pairs, and either
`part_group_memberships` or `compact_part_group_memberships`. Absent records
are omitted. Compact memberships use the same bounds and UUID checks as the
historical compact format. Schematic-only projects keep the same records under
`project.json.schematic_instances`.

The reader retains the format checks for instances/1–5, /7 and /8. The first
Save or Download rewrites these documents at the registered path. Later
unchanged saves preserve their bytes. The writer keeps instances/9 when optional
records are cleared or when expanded memberships exceed the byte bounds and the
encoder retries with compact records.

### Current sheet records

The native writer uses sheet/10 for all sheets and retains the sheet UUID and
registered path. Every sheet has `net_labels`, `labels`, `subsheets`,
`symbols`, `junctions` and `wires` arrays, including empty arrays when
appropriate. Optional fields include symbol field layout and mirroring,
net-name appearance, generated contact legs, no-connect wire ends, paper and
sheet parameters. The reader applies the same record, ownership and geometry
checks as in the earlier formats. Present generated-leg and wire-end lists must
be nonempty. Parameters require a page. Paper may be absent from snapshots
saved without the opening migration.

The first desktop Save or browser Download rewrites older sheets at the same
paths even when their records are unchanged. Later unchanged saves preserve
the exact bytes. Removing optional records or using Undo and Redo keeps the
sheet/10 header.

#### Historical appearance and generated legs

Sheet/4 retains sheet/3's required `net_labels`, `labels` and `subsheets` arrays,
even when empty. Each placement may add non-null `field_layout`, keyed by
`reference`, `value` or an existing optional field ID. A layout stores `offset`
(null for automatic placement, otherwise integer `x_nm`/`y_nm` in unrotated
symbol space), `visible`, `show_name`, `rotation` (0/90/180/270), `align`
(`start`/`middle`/`end`) and `height_nm` (0.1–10 mm). Offsets are bounded by
the schematic center range. Manual offsets turn with the symbol; explicit text
orientation is independent. Unspecified reference/value layouts retain legacy
automatic rendering, and optional fields start hidden. Absent metadata/layout
stays omitted. Earlier project and sheet headers reject fields introduced in
project/11 or /12 and sheet/4. Reference/value content
edits stay in `project.json`; label placement edits stay in the owning sheet.

Sheet/5 extends sheet/4 with optional non-null `text` on each named anchor. It uses
the same field-layout vocabulary and bounds. Offsets are relative to the electrical
anchor in its unrotated space; the text angle is explicit and its height scales
with zoom. Missing `text` uses the automatic layout. Appearance edits leave
contacts and net names unchanged.

Sheet/6 extends sheet/5 with an optional non-null boolean `mirrored` on a unit
placement. A mirrored unit reflects left-to-right about its center before the
stored quarter-turn rotation, so pin positions and drawing transform agree.
`mirrored` is emitted only when true. Field layouts keep their authored local
offsets, which are reflected and turned with the drawing. Explicit text
orientation stays as authored and is not changed by turns or mirroring.
Older readers reject sheet/6 rather than silently dropping the reflection. The diagnostic legacy single-file encoding refuses mirrored
placements because its placement vocabulary has no mirror field.

Sheet/7 extends sheet/6 with a required, non-empty `generated_legs` array on a
sheet that retains live provenance. Each record stores the generated wire's
project-unique `wire` ID, its `end` and `other` endpoints in drawing order, and
its exact `bends` array. The record lets a later mirror remove only the
connecting leg it created and preserve authored routes after a folder or archive
reopen. A record is emitted only while its endpoints and bends still exactly
match that wire; after any route edit the wire becomes authored again and the
record is dropped. When there are no live records, the writer omits the array.
Historical sheet/7 requires at least one live record. The
diagnostic legacy single-file encoding refuses live generated legs because its
wire vocabulary has no provenance field.

### Alternate pin function records

`pcbcraft-project/48` extends the modern folder or packed project entry point.
A part instance with explicit selections stores a nonempty `pin_functions` list
in `project.json`, ordered by terminal number. Each `{ "terminal": "1", "function": "ADC_IN" }`
selects an exact name from that terminal's pinned [symbol definition](../libraries/symbol-schema.md#alternate-pin-functions).
All appearances of the same part terminal share the selection. Omission uses the
primary function. Empty, null, duplicate or unresolved records are refused; at
most 4,096 selections may be stored per project.

Projects can pin version-5 symbols with or without an explicit selection.
Project/61 composes with part-group memberships and assembly
profiles without changing board, sheet, or instance document headers. Placing a
version-5 symbol and changing pin functions require current storage; neither edit
migrates projects. Pin function Preview and Apply both reject a changed selection
that would exceed the native document size limits. A rejected edit does not
advance the revision or add an undo step. Single-document diagnostic export
refuses choices rather than omitting them.

`pcbcraft-project/56` stores the same records when a project also has an active
[designator lock](#designator-locks): it accepts both `pin_functions` and
`designator_locked: true` under the rules of project/48 and project/51.
Project/61 accepts either field independently, subject to the same validation.

### Intentional no-connect records

Project/19 extends the existing occurrence and optional net-registry homes with
sparse `no_connects` on affected component records and allows wire-end intent
on sheet/8. It adds no document paths.
Each nonempty list is ordered by exact terminal token and captures that pin's
`electrical_type` when authored. There are at most 1,024 records per part (128 in earlier releases) and
4,096 combined pin/wire-end records per project; absent intent is omitted.
Older project headers reject this field, as do unsupported diagnostic single-file encodings.

The marker owns no coordinates or net membership. Every live unit appearance of
the addressed part terminal resolves its current mark position. Missing or
type-changed targets remain addressed unresolved intent and can be removed;
they do not silently acquire another pin. Explicit null, empty lists, duplicate
or malformed terminal tokens and unknown fields are rejected. Folder/archive
round trips use the same codec. Projects without markers omit the records.
Package `unconnected_pads` records carry separate physical intent.

Sheet/8 requires a nonempty `no_connect_ends` array of junction IDs, written in
ascending order, and otherwise retains sheet/6 fields plus optional nonempty
generated-leg records from sheet/7. Each ID must be a live junction on that exact
sheet; duplicate, unknown, foreign-sheet and out-of-range IDs are rejected.
The endpoint owns the mark position, so moving it requires no separate marker
coordinate. Earlier sheet headers reject the field. Historical project/19
requires at least one pin or wire-end marker across the project. Current saves
omit the field when removing the last marker.

See the [no-connect contract](../schematic/no-connect-markers.md) for commands and checks.

### Sheet paper records

Sheet/9 introduced a required `page` record. Current sheet/10 saves place it
after `id` when the sheet has paper:

```json
"page": {"paper":"a4","orientation":"landscape","at_nm":[-148590000,-105410000]}
```

`paper` is `a4`, `a3`, `a2`, `a1` or `a0`; `orientation` is `landscape` or
`portrait`; `at_nm` is the paper's top-left corner in sheet nanometres (Y points
down). The whole paper must lie inside the ±1 m schematic range. An optional
`template` names the sheet's frame:

```json
"page": {"paper":"a4","orientation":"landscape","at_nm":[-148590000,-105410000],"template":{"library":"basic-frames","name":"default","revision":1}}
```

New sheets and sheets given paper on open store the Default frame,
`basic-frames/default` revision 1. Without `template` the sheet has a plain border
5 mm inside the paper. A template revision never changes once released, so a
sheet keeps its frame across app updates. Only the app's bundled templates exist
so far; a sheet naming any other template is refused rather than opened without
its frame. Templates have no files in the project folder yet.
Unknown fields, null (for `page` or `template`) and other sizes are rejected, and
sheet formats before /9 reject `page`. Sheet/9 and /10 both support optional
`generated_legs` and `no_connect_ends` arrays under the sheet/7 and sheet/8 rules.
Changing one sheet's paper changes only that sheet's document. The diagnostic
single-file encoding refuses paper.

`sheet.set_page` takes `sheet`, `paper`, `orientation` and an optional `at`
point. Without `at` the paper's top-left corner stays where it is, and the frame
is kept. `sheet.set_template` takes `sheet` and a `template` address or `null`.
Each is one revision-checked project undo step, and setting the current value
changes nothing. Inspection reports each sheet's `page` in the same shape.

### Designator locks

Project/51 stores optional `designator_locked: true` on each locked part-instance
record in the existing `components` array. The missing field means false;
explicit false, null, other types and unknown fields are refused. Project/51
requires at least one true flag, and earlier headers reject lock fields;
project/56 applies the same rule alongside alternate pin functions. Older
readers reject project/51 rather than silently dropping intent. The diagnostic
single-file codec refuses locks; folder and `.pc` codecs retain them.

Locks belong to the existing part-instance identity, not a unit placement or
library definition. A lock-only edit in an already-adopted project leaves geometry,
terminal identities, package mappings, logical board ownership and sheet formats
unchanged. First adoption can change storage paths and add occurrence metadata;
that storage layout change does not transfer logical board ownership. The shared
field command can also carry other edits, whose geometry, appearance and board-state
effects are separate from the lock flag. Project/51 retains the optional
project parameters, net-class catalog, assignments, net registry, no-connect
intent, part groups and assembly profiles under their existing validation rules.
Projects with differential pairs keep those records when a lock is added.
Current board-owned records use [instances/9](#board-instance-records).
Schematic-only projects keep the same embedded registry home.

Setting the first lock adopts storage, allocation-watermark and sheet-occurrence
metadata in that same undoable transaction if the older project lacks them.
Opening, inspection and an unchanged edit never create a lock. Unlocking the
last part omits the last lock flag while keeping project/61. Undo restores the
prior authored content. If no intervening Save has adopted storage metadata,
Undo also removes layout and occurrence metadata added by the lock edit, and
restores the exact prior documents for an already-watermarked project. Undo
retains an adopted numeric allocation watermark under the existing monotonic
identity policy. If Save adopts a folder layout or occurrence metadata into
history, Undo clears the lock but keeps that saved metadata. Lock inspection
always reports a boolean, including false. See [Properties and shared commands](../quickstart.md#designator-locks).

Ordinary Apply validates the model but does not encode the entire native project.
Adding a lock can push an otherwise valid document over the 16 MiB per-document
or 48 MiB document-set limit. Preview performs both encodes after ordinary command
validation and refuses if either does not fit, without changing the live project.
A successful Apply is therefore not a promise that Save will fit: a refused Save
reports the encoding error, leaves the authored session dirty and does not publish
the new document set. Undo the edit or reduce the document before trying Save again.

### Parameter records

Parameters are named text values that texts show as `@project.Name`,
`@sheet.Name` or `@instance.Name`; see
[sheet frames and parameters](../design/sheet-frames-and-parameters.md#parameters). Only
values that differ from the defaults are stored, so a project that sets none
keeps its earlier bytes.

Sheet/9 and /10 documents may carry a nonempty `parameters` object directly after
`page`. The current project entry point accepts an optional nonempty
`parameters` object alongside other authored records. The writer places it
directly before `sheets`:

```json
"parameters": {
  "Company": "Tangency Labs",
  "Revision": "B"
},
```

Each object holds at most 256 entries sorted by name, one per line, so changing
one value changes one line. A name is an ASCII letter followed by up to 63
letters, digits or `_`; a value is a string of at most 1000 bytes and may hold
references. Editing ignores case, so a name that differs from another stored
name, or from a standard name of its list (`Revision`, `Date`, `Company`,
`DrawnBy` for the project; those and `Title` for a sheet), only in case is
rejected. Empty objects, null and other value types are rejected. Sheet parameter
objects require paper. Project parameter objects are supported in project/22,
/23, /24, /36, /39, /45, /48, /51, /56 and /61. Project/22 is a superset
of project/21 whose net-class catalog, class assignments, net registry and
no-connect intent are optional, each with its existing rules. Project/23 requires
generated net numbers and permits those same optional fields, including project
parameters. Project/24 adds the part-group catalog and keeps every record
of project/23 optional, generated net numbers included. Project/39 keeps those
same rules with compact part-group memberships. Project/36 permits the same optional records with either membership representation, alongside its required assembly profile catalog. Project/45 permits the same
records and an optional profile catalog alongside its required differential
pairs. Project/51 requires a designator lock and permits the same records with or
without pairs. All these formats require adopted storage and occurrence metadata;
setting the first project value adopts them in the same undo step.
Removing the last project value omits the parameters object. The diagnostic
single-file encoding refuses parameters.

### Part-group records

Both project/24 and project/39 require a nonempty `part_groups` catalog
(`pcbcraft-part-groups/1`) directly after any `net_classes` record. Project/36,
/45, /48, /51, /56 and /61 also accept the catalog, with either membership
representation.
Both membership representations compose with pairs in the
[current instance format](#board-instance-records).
Project/24, the expanded representation, is a superset of project/23: generated net numbers, net classes and
their assignments, project parameters and no-connect intent are optional, each
with its existing rules, and it requires adopted storage and occurrence metadata.
Under project/24, explicit memberships are stored as `part_group_memberships`,
after the sheet occurrences and any net registry in the instance home:
`schematic_instances` without a board, or the board's `instances.json`, which
selects `pcbcraft-instances/5` exactly when expanded memberships exist. Records are
`{"part_instance":N,"group":UUID}`, sorted by part instance and group, unique and one per
line. Empty lists, null, dangling records and these records under older headers
are rejected. `N` is the part instance's number, as in `components` and
`component-N`; see [part groups](part-groups.md).

Compact memberships replace that list with `compact_part_group_memberships`.
Current board-owned records use instances/9 for either representation.
Historical instances/7 carries compact memberships without pairs. Instances/8
allows either representation and requires pairs.
Schematic-only memberships remain embedded in the entry point. The compact
record contains a sorted UUID `groups` dictionary
and sorted `part_instances` rows (`part_instance` plus sorted numeric `groups`
references into that dictionary). UUIDs remain the identity authority. The
reader refuses duplicate, unused, nil or foreign dictionary UUIDs, duplicate or
unordered rows/references, out-of-range or noninteger indices, empty payloads,
excess expansion and simultaneous expanded/compact fields. Encoding falls back
once to compact storage on a byte-limit overflow; the limits do not increase,
and Save adds no precondition. Legacy files that fit retain their expanded
representation; compact files retain their encoding choice after reopening.
Unchanged saves preserve canonical encoder bytes in either representation;
accepted noncanonical input may normalize. Both homes support all 4,096 part
instances in all 256 groups.

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
unit, position in integer nanometers, optional left-to-right mirror before
rotation and rotation in degrees (0, 90, 180, 270).
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
material and mapping identities. Legacy single-file export refuses rotation.
Moving or rotating an isolated footprint changes its placement record. Any track
ends carried with the footprint also change. Every project has its sheet documents. A new schematic-only
project has no board document; an explicitly created PCB and legacy implicit-board
projects have a physical board document even when no footprints are placed;
its path follows the registration after folder adoption. The board registry
supports zero or one board. Newer physical-board versions add data for that board,
rather than implementing multiple boards.

Physical footprint references require board version 5. Each instance carries
`library`, `definition`, `revision` and an explicit `mapping` from terminal ID to
pad ID, together with its position and optional rotation. The complete
[footprint definition](../libraries/footprint-library.md) is saved separately and must match
its path identity. Missing, conflicting or unused definitions are refused; loading
never substitutes a built-in definition. Version 5 allows absent setup or the
version-3 setup vocabulary. Legacy marker instances can coexist with physical
placements, and legacy single-file export refuses physical definitions. A header
migration preserves marker placements and their existing geometry. Deleting the last placement using a physical definition removes its
unused pinned copy on save.

Pinned footprints hold up to 1,024 pads and 256 artwork objects, pinned symbols
up to 1,024 terminals, and a mapping up to 1,024 terminals. The formats keep their
numbers; earlier releases refuse records above their old limit of 128.

Board format 6 requires a `silkscreen` object with `text_height_nm`,
`stroke_width_nm` and `pad_clearance_nm`, as defined in the
[footprint contract](../libraries/footprint-library.md#silkscreen-settings). It permits the
same optional setup, placement orientation and pinned definitions as version 5.
Versions 1–5 refuse this field. Missing settings use the stable default style
for definitions that request physical silk. Editing the style changes only the
board document. Legacy single-file export refuses these
settings rather than discarding them.

Board format 7 adds a nonempty `assignments` array before `footprints`. Each
record carries the numeric component ID, library/item/revision and an explicit
terminal-to-pad mapping, with no placement ID or coordinates. Every assignment
must resolve a component and pinned physical definition; duplicate components,
incomplete/nonbijective mappings and disagreements with an existing placement
are refused. Unplaced assignments retain their pinned files even with an empty
`footprints` array. The incremental encoder writes one assignment per record
budget and emits shared definition copies once. Setup and silkscreen remain
optional under version 7, with their existing validation. Legacy single-file
export refuses independent assignments rather than losing them.

Board format 10 is explicitly enabled by `pcb.initialize_update_baseline` and
stores accepted historical components, packages and terminal membership. It can
retain a footprint after its live component is deleted and preserves assets
referenced only by accepted history. The [single-board update foundation](../pcb/single-board-update.md)
defines the codec and current engine-only limitations; older readers refuse this
format instead of dropping retained physical state.

Board format 11 retains that baseline while allowing desired assignments to differ
from accepted placements. Format 11 requires pending package intent.
Format 42 supports both states. Apply or resetting intent removes the difference. Desired
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

Board format 26 requires a nonempty `arcs` array and accepts every field of
supported board formats through 25. Each [circular track](../pcb/circular-copper-records.md)
stores UUID `id` and copper `layer`, integer `width_nm`, integer-nanometre
`center`, `start`, `end` points and `clockwise`/`counterclockwise` `direction`.
The radius is derived from center/start, not stored. Each arc occupies one line
in UUID order, at most 4,096 per board. End radial error is at most 1 nm inclusive;
zero radius, zero sweep, full circles, invalid dimensions/layers and duplicate
copper IDs are rejected with exact integer predicates. Formats 26 and 35 require
nonempty `arcs`. Formats 41 and 42 support arcs as an optional array.
Native documents and archives round-trip the exact records; the diagnostic
single-file writer refuses them. Geometry consumers refuse rather than flatten
these records until circular conductive contours are implemented.

Board format 12 adds a `tracks` array and may carry every earlier field, so routing
a board never drops its baseline, pending intent, graphics or setup. Each record is
one straight segment on its own line: a UUID `id`, a copper `layer` UUID from the
setup, an integer `width_nm` from 10,000 to 10,000,000, and integer-nanometer
`start` and `end` points that differ. Records sort by UUID, so adding or deleting a
segment changes only its own line. The codec rejects duplicate IDs, non-copper or
unknown layers, out-of-range widths and more than 65,536 segments (4,096 in earlier
releases, which reject larger boards with that error rather than truncating them). Format 12 requires
at least one track and rejects `vias`, so readers released with it keep working.

Board format 13 adds a `vias` array in the same style and is otherwise format 12: a
UUID `id`, an integer-nanometer `at` point, and `diameter_nm` and `drill_nm`, with the
drill at least 50,000 and smaller than the pad and the pad at most 10,000,000. A via
spans every copper layer and needs a setup with at least two; a board holds at most
16,384 (2,048 in earlier releases), within the native board document size limit.
Format 13 requires at least one via, and its `tracks` array is optional. Either
array is omitted when empty, and formats before 12 reject both fields.
Deleting the last track or via leaves the current header at format 42.
Net membership is not stored; it is derived from copper geometry and the accepted nets.
Legacy single-file export refuses tracks and vias.

Board format 14 marks bottom-side footprints with `"bottom": true` on their placement
records and is otherwise format 13, so tracks and vias stay optional. It requires at
least one bottom-side footprint, and earlier formats reject the field; top-side records
omit it. A bottom footprint mirrors its local X before rotation, and its surface pads
and technical graphics use the back layers. Flipping the last footprint back
to the top omits the flag, with the same format-42 bytes as before the flip.
Legacy single-file export refuses bottom-side footprints.

Board format 15 adds optional `reference_offset` and `reference_text` fields on a
footprint placement. The offset is measured in local nanometers before rotation
or side mirroring. The sparse print object overrides words, font, paired text
height/stroke, local rotation, alignment and visibility while retaining the
component's identity and reference. Format 15 requires at least one nonzero
offset or nonempty print override. Resetting both omits them from the next
format-42 save.
The [footprint format specification](../libraries/footprint-schema.md) defines its placement schema and coordinate contract.

Board format 16 adds optional integer `stroke_nm` to layer graphics, bounded
from 10,000 to 10,000,000 nanometres. New non-copper technical graphics require
an explicit width; omitted widths retain legacy Board Outline hairlines, and
explicit nulls are rejected. The format carries every field supported by format
15, including reference-position and print overrides, and requires at least one
stroked graphic. Older formats reject the new field. Graphics reference existing
non-copper layer UUIDs; conductive copper linework uses the existing track records
and their connectivity/DRC/export rules. Removing the last stroked graphic
leaves the current header at format 42.

Board format 18 adds optional `locked: true` on track and via records and
requires at least one authored lock. It is a superset of format 17, including
multi-pad mappings, physical graphics, accepted baselines and pending intent.
Absent copper lock flags mean false. Canonical saves omit false and use format
42 regardless of active locks. Historical headers below 18
refuse active copper locks, and older readers refuse format 18.
Flags survive folder/archive reopen and undo/redo without changing object IDs,
geometry or connectivity. The shared lock command and protection policy are in
[the layout workflow](../pcb/locks.md#fixed-tracks-and-vias).

The earlier local format 8 rectangle remains readable. Its first geometry
edit materializes a layer-addressed drawing. Header migration alone preserves
the rectangle record. The current reader rejects incompatible newer data, but tolerates a
noncanonical `locked: false` as described in [footprint lock metadata](#placed-footprint-lock-metadata).
Legacy single-file export refuses edge graphics. Board document additions do not introduce a broader
multiboard migration; the entry point separately records explicit zero-or-one
ownership in project/9 or project/12.

An unconfigured board uses version 1 only if no authored field requires a later
format. A legacy layer and stackup setup needs version 2 or the compatible setup
of a later board format. Version 2 requires a `setup` with a `layers` catalog and
a `stackup` profile. Layers have UUIDs, names, typed purposes and optional surface
sides; the profile has a UUID and front-to-back `strata`. Copper strata reference
catalog layer UUIDs, while dielectric strata own UUIDs and optional material
names. Profile, layer and dielectric IDs must be non-nil and mutually unique.
Catalog order is canonical UUID order; physical order belongs only to strata.
The legacy material name is descriptive text only. Version 3 introduces
material-aware setup: a project-local `materials` catalog, exact ID/revision
references, sourced Tg and Dk/Df reports, core/prepreg construction, ordered plies
and optional symmetric finished-gap tolerance. The [material contract](../pcb/board-materials.md)
defines field meanings, units and bounds. Current canonical version-2 documents
retain their bytes and unknown construction in codec round trips. The
material-aware fields introduced in version 3 are not accepted by version 2,
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
codecs. The [representation plan](../design/pcb-representation.md) explains future regions,
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
outside project documents; see [native library sources](../libraries/native-library-sources.md).

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

[Named arrangements](../design/board-workspace-plan.md#named-arrangements-and-reopening-a-project)
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
and adds its saved symbol file. After layout and header migration, a no-op save rewrites no project documents.
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

Writers use Store, fixed timestamps, fixed `0644` permissions recorded as made on
Unix, and a deterministic entry order (manifest first, then paths in lexical
order), so every platform writes the same archive bytes. Readers accept Store and
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

## Unsaved-work recovery store

The save transaction above protects saved files. `pcbcraft_storage::recovery`
provides an API for recording unsaved edits, with a per-user recovery store
as its default. Callers can choose another directory; the API does not check
whether it is outside a project. Hosts should keep recovery separate from
project files, the `.pcbcraft-save` transaction and the save lock. The desktop
attaches a recorder to its open project when Recovery snapshots is on (it is Off
by default); the CLI and headless MCP do not yet
attach one. Offering recovery entries on start is not implemented.
The API's default store is `PCBCRAFT_RECOVERY_DIR` when set, else `recovery` under
`%LOCALAPPDATA%\PCBcraft`, `~/Library/Application Support/PCBcraft` or
`$XDG_DATA_HOME/pcbcraft` (`~/.local/share/pcbcraft`). The browser has no store.

A host that attaches a recorder gives each editing session its own entry,
named by the project UUID, the process and a time stamp so two sessions never
share it:

| File | Contents |
|---|---|
| `<id>.pc` | Versioned recovery envelope around a packed project snapshot |
| `<id>.journal` | Successfully recorded changes since that snapshot |
| `<id>.lock` | Held locked by the writing session while it runs |
| `.entries.lock` | Folder gate for opening and removing entry locks; kept after entries are deleted |

[`Session::observe_edits`](../../crates/engine/src/lib.rs) reports every edit
committed through the shared command path, from the GUI, CLI or MCP, including
prepared copper edits, and each new history root: an open, or the name and
layout a completed save adopts into the live project. `JournalRecorder` encodes
and writes them on its own thread,
so neither editing nor saving waits for it. Each record names its command,
command version and parameters, and stores the documents the edit added, changed
or removed with the same codec a save uses, plus fingerprints of the complete
document set before and after. Records hold results rather than commands to run
again, because commands that create objects draw fresh random identities.

Both snapshot and journal start with `pcbcraft-recovery/2`, followed by a framed
JSON header with `generation` and `base_sha256`. The snapshot then holds the
ordinary packed project bytes. Its generation is its staging filename, with 24
random characters, and changes on each publication even if the documents are
identical. The version and framed snapshot header fit within 4096 bytes; the
packed project retains its ordinary archive limits. A journal is replayed only
when both generation and document hash match the snapshot. Unknown header fields
are ignored within this version; required fields must be present.

Each frame has a little-endian u32 payload length, the payload and its SHA-256.
The writer attempts to flush each edit record before processing the next
notification. Recovery reads the snapshot and applies records in order. It
stops at the first torn, damaged or inconsistent record and keeps the last
complete readable state. A kill can lose edits being written or still queued.
Earlier edits can also be absent after encoding or write failures, or after
removal of the recovery entry; receipt of a notification is not proof of
persistence. `sync` waits for prior notifications to be processed and returns
the first error since the previous call, rather than guaranteeing every edit
was written. The snapshot is published first, so a stale journal is ignored even
when its base documents equal the new snapshot's. Unsupported versions,
including the earlier unwrapped snapshot and `pcbcraft-recovery/1` journal, are
refused for the caller to offer for discard. These are recovery files, not
ordinary `.pc` project files; saved project formats are unchanged. The store's
layout and record encoding are private to PCBCraft and carry no compatibility
promise beyond that refusal. Directory flushes have the same Unix-only limit
as saves.
Windows refuses to rename a file over one that is open, so the writer closes its
journal before publishing the next one. While another process has the snapshot
or journal open, for example while reading the entry, publishing over it fails
there; recovery can read the previous recorded state, or the new snapshot
already holds the state supplied for publication. The writer tries another
snapshot at the next edit.

The journal has a size limit. When it grows past twice the size of its snapshot,
or past 1 MiB if that is larger, the state after the edit that crossed the limit
becomes a new snapshot and the journal starts again empty. Recovery then reads
the snapshot, records up to the larger of twice its size and 1 MiB, and the one
record that crossed the limit. While a project stays about the same size,
rewriting snapshots adds at most about half again to the bytes the journal
writes; one large edit, such as a big paste, can make the new snapshot as large
as the old snapshot and the journal together. A failure before the new snapshot's
rename leaves the old snapshot and journal unchanged, already holding that edit,
and the next edit tries again. A failure after the rename, including the folder
flush, adopts the new snapshot and closes the old journal. The next edit starts
another snapshot rather than appending records recovery would ignore. While
snapshots keep failing and writes to the open journal still succeed, edits
continue to be appended, so the journal can grow past the limit until a snapshot
succeeds, an append fails or the session ends. A full recovery filesystem can also make an
append fail; then the writer closes the journal and the next edit tries a new
snapshot. Removing the recovery folder while a session runs
is different: on Unix it deletes the snapshot and journal, and bytes written to
the still-open, unlinked journal cannot be recovered. The next changed append
checks whether the journal path is missing; if it is, the entry is disabled and
the error is held for `sync`. Recreating the folder does not re-enable that
entry, since the original lock handle cannot protect a recreated lock path; a
new recorder and entry are needed. This check detects a missing path, not an
arbitrary replacement or a concurrent removal after the check. Other write
failures with no journal open, including a record or new journal that could not
be written or an opened or saved project that could not be snapshotted, leave
edits unrecorded until a snapshot succeeds; recovery returns the last recorded
state. A recorder keeps the entry lock even if its first snapshot fails.

Each recorded edit encodes the whole project and fingerprints the complete
document set, on the writer thread; the editor thread only queues the shared
project. A recorder started with `RecorderOptions::coalesce_edits` encodes only
the newest of a run of edits still queued when the writer gets to them, so the
writer encodes at most one state per completed write however fast edits come.
Replay is unchanged, since each record holds the documents it produces, but
`Recovered::commands` then names only the last command of each coalesced run.
No timing has been measured.

With `RecorderOptions::retention`, the writer first prunes entries no running
session holds: those last written longer ago than `max_age`, then all but the
newest `max_entries`. Retention verifies each candidate's recovery format header
before applying age or count limits, so ordinary `.pc` project archives and
other non-recovery files in the directory are never counted or deleted. Held
entries are never touched, at most 16,384 folder entries are examined, and
folder-gate contention skips the prune without waiting. The desktop keeps 16
such entries for up to 14 days and attempts to delete its own entry on a clean
exit (below); cleanup failures are logged. `JournalRecorder::start` records
every edit and prunes nothing.

### Recovery snapshots and timed saves

**Preferences → Saving → Recovery snapshots** (`autosave_minutes`) is Off (0)
by default until the recovery offer on start exists. Off attaches no recorder
and writes nothing; turning it Off retires the running recorder and attempts to
delete its entry. Set to 1, 5 or 10 minutes, the desktop app records its open
project when a recovery folder is available, journaling committed edits and
taking a snapshot every interval after the first change since the last one.
Encoding and I/O failures have the limits described above; the timer does not
guarantee persistence. A successful recovery snapshot replaces `<id>.pc` with
the current state and starts an empty journal. A snapshot waits until editing
has paused for 2 seconds, but never more than a second interval. A synchronous
GUI edit timed by `PcbApp::run` at 1.5 seconds or longer requests one on the
next frame without waiting for the interval. A snapshot is skipped only when its
journal is open and the state matches both the last snapshot and the last
recorded state.
Native timer logic does not require the window to paint. If recording fails,
the status bar says why unsaved work isn't being kept and the editor retries a
snapshot after five seconds, even with no new edits. Detected removal disables that entry until the project
is replaced or reopened.

Only an approved project close attempts to delete the current entry. An
unexpected framework exit drains what it can and leaves the entry, as does
Close without waiting while a save or open is still running. On a project
replacement or reopen, the outgoing recorder does not receive the next project's
state. At most one recorder is active or retiring; if retirement stalls, the
replacement starts recording from its latest state once the old worker stops.
Pending notifications are bounded; a burst beyond that bound is condensed into
a fresh snapshot of the newest queued state.

Quit waits up to two seconds for recovery. If drain or gate acquisition misses
its deadline, deletion does not start and the entry stays. After deletion
begins, filesystem calls cannot be cancelled; failures may leave partial files
and are logged. The folder gate protects lock-path creation and close/unlink on
Windows. Cleanup never removes an unowned entry and does not wait on gate
contention. The empty `.entries.lock` gate file contains no project data.
A crash can leave an entry for a future recovery offer; that offer is not built.

**Save to project** (`save_to_project_minutes`) is Off (0) by default. Set to
1, 5 or 10 minutes, it runs the ordinary Save on the same timer, once the
project has unsaved changes and already has a current-format folder or `.pc`
file. An Untitled project is never saved on its own, and timed saves are held
back for legacy archives or split folders that would be upgraded on their first
write until an explicit Save performs the upgrade. A timed save waits while a
dialog or another file operation is open, or a project action is waiting to run. Like Save, it is
refused when the project changed on disk; the project stays unsaved and the
next try comes one interval later. Recovery encoding and disk I/O run on a
separate worker rather than making Save wait for them. A
timed save is an ordinary save, so **Save** is unavailable while it runs, as
during any save. The Saving page activates its changes only on **Apply** or **OK**.
Both settings are personal preferences, also set by `app.set_preferences`, and
are never stored in the project ([live preferences](../mcp.md#preferences-and-external-updates)).
The CLI and headless MCP do not record unsaved work yet.

## Compatibility and fixtures

App version 0.1.0 introduces this layout. Earlier working
folders used a plain JSON `project.pc` entry point, and split archives used
`.pc.zip` with container version 2. Older single-document projects could also
use `.pc` for plain JSON or container version 1. Normal desktop, browser and
headless opens accept the immediately previous split layouts without rewriting
their saved project documents. Closing without saving does not publish an upgrade.
A settled archive needs no lock or journal on Open; a folder can acquire the
persistent sibling lock described under [folder recovery](#folder-publication-and-recovery).
Completing an interrupted folder save on Open is recovery, not a format upgrade.
Previews are read-only too. On desktop, first Save upgrades
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
other destinations create a new folder. On Windows, a destination named like a
device, such as `con.pc`, `aux` or `COM1.pc`, is refused before anything is written.
This includes `COM¹`–`COM³` and `LPT¹`–`LPT³`, with or without extensions and
ignoring ASCII case, as specified by [Windows naming conventions](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file#naming-conventions).
Trailing ASCII spaces in the text before the first dot do not bypass the check:
`NUL ` and `COM¹  .pc` are also refused. The check does not rename the destination.
Other platforms accept these as ordinary destination names. Both forms validate
all managed documents and preserve stable IDs, pinned libraries and exact physical/sheet/library payload
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
archive round trips and the compatibility boundary. The separate
[frozen archive corpus](../../crates/storage/tests/fixtures/native-open/README.md)
checks that normal native Open still reads previously saved bytes and preserves
logical connectivity and object counts. The two previous split archive forms
are also checked after explicit conversion. When a release adds a format,
append fixtures saved by that release rather than regenerating the existing
corpus. See [release notes](../../CHANGELOG.md).

## Design references

The validation and round-trip requirements draw on independently authored
PCBcraft Next material at revision `568cd495397a91563e563e7163efb226d84fa83a`
under MIT OR Apache-2.0. The proposal uses Tangency's existing session and
terminal model to specify a new fixture.
[Persistence design](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/docs/specs/persistence.md),
[codec reference](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/src/persistence.rs),
[schematic test contracts](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/tests/schematic.rs),
[project test contracts](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/tests/sheets.rs).
