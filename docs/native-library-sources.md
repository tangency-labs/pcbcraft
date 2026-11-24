# Native library sources

This slice of #92 loads native symbols, footprints, category definitions and
part definitions from personal sources.
Native parts and symbols can be searched and placed through the Parts Catalog.
Native footprints are available in the assignment chooser, and manufacturer
records can be selected with their exact generic part and package. Registering, refreshing or removing a source
does not change the project, its revision, dirty state or undo history.

Open **Parts Library** from the workspace picker or **Libraries → Sources**.
The Browser separates **Project**, **Built-in** and **Sources**, with
Symbols, Footprints, Generic parts and Orderable parts under each library.
Project shows the exact definitions saved with the open project, grouped by their
original library, with deliberate local variants under **Project-only**. Its list
retains every pinned revision and shows usage and source status. Offline or
conflicting sources do not replace the project preview. **Edit copy** opens the
pinned symbol or footprint as an independent draft; reviewed application to
existing instances remains separate work. Refresh, Relocate, Remove and Delete
operate on source registrations, not Project rows.
Library folders start collapsed, and item types with no entries are hidden.
The Browser header menu offers **Expand all** and **Collapse all**. Drag the
Browser's right edge or the divider beside docked Preview to change their
widths; both use the shared splitter and support Escape cancellation. Compact hosts use the full pane and
retain that width for wider layouts. The tree does not scroll horizontally.
The central list has search and a **Type** choice. Generic parts expand to their
orderable records; Orderable parts shows the same records as a flat table.
Select an item to inspect its native drawing or summary, license, available
revisions and exact dependencies in Preview. File errors and quarantined
conflicts stay with their source. Browsing does not change project state.
Compact panes expose Browser, Items and Preview through a shared choice strip.
Library supports the normal split, open-beside, detach, dock and window paths,
including beside a Symbol Editor. Like definition editor panes, Library panes
are runtime workspaces excluded from saved project arrangements.

**Add library** offers a library folder or a `pcbcraft-library-bundle/1` bundle.
A folder contains `library.json` and optional flat `symbols/`, `footprints/`,
`categories/`, `parts/` and `manufacturer-parts/` directories. On desktop its
registration is saved in personal settings and reloaded in the background at
startup. **Refresh** rereads the selected folder; **Relocate** chooses its new
location. Missing sources remain listed with their names and errors.

On desktop a bundle is validated and installed into a new folder chosen in the
picker. Existing files/folders are never overwritten, and registration happens
only after the complete installation succeeds. A bundle whose every included
revision is already available with identical content is refused before writing.
Conflicting definitions follow the ordinary quarantine rules. A failed publication
cleans up its newly created folder where possible and reports any incomplete
folder that remains. There is no temporary bundle-loading mode on desktop.

In the browser, folder selection reads its native files once; bundle selection
reads one file. The page retains no disk handle or ongoing folder access, and
browser storage retains only registration IDs/names. After a reload each source
is listed as **Needs reopening** and supplies no items until **Refresh** chooses
its folder again. A saved project retains its pinned definitions and opens
without reopening the library. There is no separate session-upload group or
Upload bundle entry point.

**Export** (in the ribbon's Share group) writes the library selected in the tree to one
`pcbcraft-library-bundle/1` file containing its original native documents, including
`library.json` with the license. Desktop saves to a new file and never replaces an
existing one; the browser downloads the file. Export doesn't change the library or any
project and doesn't collect records from other libraries: the completion notice names
other libraries the bundle depends on and any dependency that doesn't resolve. Project
pins aren't exportable libraries.

**Remove** forgets the registration/cache and leaves its files intact. **Delete library**
is desktop-only for writable registered folders. Its confirmation names the
folder, counts native item revisions and explains that projects keep their
pinned copies. Pending editor changes from that library block deletion. The
host rechecks the source and folder before moving it to system trash; failure
retains the registration and reports an error. Linux/FreeBSD use the desktop
`gio trash` service, macOS uses Finder, and Windows uses the recycle-bin API
through PowerShell. An unavailable service has no permanent-delete fallback.
Built-in libraries cannot be refreshed, relocated, removed or deleted.

For an individual generic or orderable part, return to **Browse**, select its row
in a writable folder library, and choose **Item › Delete part**. Close that
part's authoring document first, including a clean published document. The
confirmation covers every revision of that identity in the selected folder;
symbols, footprints, categories and existing project-pinned copies are retained.
Known orderable references in loaded libraries and dependent open drafts block
removal. Libraries that are not loaded cannot be checked.

The host captures the native files, rechecks them under the publisher lock, and
stages the reviewed records as a small native library folder before sending that
folder to system trash. A failed trash operation restores unused source names;
if a newer file prevents restoration, the error identifies the retained recovery
folder. Source loading also reports any pending recovery folder, including after
an interrupted process, and blocks further publication/deletion until it is
explicitly restored or moved out of the library. This reports recoverable state;
it does not perform automatic crash recovery. There is no permanent-delete fallback. The library refreshes after the
operation, including failure. This per-part path currently requires a Unix
desktop; Windows and browser per-part deletion remain unavailable. A folder
restored from trash contains the original manifest and deleted native records;
there is no in-app undelete or project-Undo operation for library deletion.

Live clients prepare `app.library_delete_part` in the addressed Library pane,
then read `library_item_deletion` from editor inspection. Confirmation requires
`phase: "confirm"` and the reviewed `token`; `phase: "cancel"` dismisses the
review. The token binds the target, captured files and actual directory, and a
catalog change requires a new review. These phases share GUI validation and the
single native source-operation worker slot.

To try the original MIT OR Apache-2.0 examples, add `examples/native-library`
or `examples/native-parts`, or package one for transfer:

```sh
python3 scripts/package-native-library.py examples/native-library /tmp/workshop.pclib.json
```

The packager preserves existing output. The bundle's `documents` object maps
native relative paths to UTF-8 file contents, validated with the same engine
rules on both platforms. Press **A** in Schematic to use the existing Parts
Catalog placement picker; Library browsing itself never places an item.

The source loader accepts at most 32 registrations including the bundled libraries, 8,192 files per
source, 1 MiB per file and 32 MiB of decoded UTF-8 content per source. Bundles are
also bounded to 16 MiB before decoding. Folder reads reject symlinks, special
files and nested item directories. Other root files, such as a library's license
text, are not treated as native item documents; retain those notices with the
source and comply with the asset's redistribution terms when sharing it.

Definitions with the same library, kind, item ID and revision must have identical
content. Conflicting copies are reported and excluded from search and placement.
A conflicting latest revision does not silently fall back to an older one.
Refreshing a registration also detects content changes under an existing
revision. Correct the source or give a changed definition a new revision.
Removing a registration forgets its disposable source cache, but a project still
refuses a new placement that conflicts with its already pinned revision.

External symbols retain portable library-name and license attribution with their
pinned definition, alongside the existing library/item/revision identity.
Personal source IDs and machine paths are not copied into project documents.
Previously saved symbols without attribution retain their original encoding.
Opening a project never searches or refreshes a library to replace its geometry.

## Query available revisions

`library.available_revisions` takes no parameters and requires an open project.
It compares existing part instances with native revisions already loaded in the
session, without refreshing sources or accessing the network. It works through
the shared engine command, including live and headless MCP with `inspect: false`.
In the desktop app, Parts Library's **Review library updates** (also
**Libraries → Updates** on the ribbon) lists the instances this query reports
with a supported update; see
[adopt non-geometric revisions](#adopt-non-geometric-revisions).

The result contains `project` (UUID), runtime `revision`, `catalog_generation`,
`board` (UUID or null), `instances` and `outdated_instances`. Each instance row
has its stable `part_instance` ID, display `reference`, and `items`. Native part
instances compare their owning generic part, not independently advancing its
symbol, category or package dependencies. Plain symbol instances compare their
symbol and any standalone footprint choice, including before a board exists.
Accepted board-history geometry is not an independent instance update root.

Each item reports `pinned` and `latest` addresses (`library`, `kind`, `id`,
`revision`), `status`, `latest_digest` and `diagnostic`. `latest` is the highest
loaded revision of that exact identity and kind, or null. `latest_digest` is the
native record's content digest, not a part dependency-closure placement digest.
It is null if no candidate exists or the candidate conflicts or cannot resolve.
A diagnostic explains a conflicting or unresolved candidate or conflicting pin;
otherwise it is null. When both the pin and the candidate fail, the pin conflict
is reported first, followed by the candidate error. Rows retain exact pins;
instances and their IDs occur once, with items in address order.
`outdated_instances` contains only instances with at least one `Update available` item.

Statuses are `Matches source`, `Update available`, `Source unavailable`,
`Project-only`, `Conflict` and `Unresolved`. An absent or unavailable source does
not mean current. An older-only source is not an update; a missing exact source
pin can still have a newer loaded candidate. Deliberate project variants remain
`Project-only`. Same-address content conflicts take precedence over availability.
For a candidate part, this includes symbol, category or footprint content that
differs from a project pin at the same immutable address, even when the part
otherwise resolves.
A conflicting or unresolved highest revision is reported without falling back to
an older candidate; generic-part candidates must resolve their exact dependencies.
An unresolvable latest part is `Unresolved` even when it equals or is older than
the project pin, unless an immutable-address conflict takes precedence.
A valid newer revision means available, **not compatible or adopted**. Geometry,
pins, project revision, dirty state, source registrations and Undo/Redo are unchanged.
The pad preview below is read-only. Compatibility decisions for geometric
changes and pad map resolution remain separate work.

## Preview footprint pad changes

`library.preview_footprint_revision` takes the explicit `board` UUID and one
`part_instance` ID from `library.available_revisions`. It works through the same
engine command in live and headless MCP and returns the report with `inspect: false`.
It compares the currently placed native footprint with its latest available
update. A standalone footprint follows its exact library/item identity. A native
part follows its owning part revision and retains its chosen package ID; a removed
package is not silently replaced. A conflicting or unresolved latest revision
is refused. Unplaced parts and placements whose current package choice differs
from the placed footprint cannot be previewed.

The result identifies the project, board, instance, placed footprint, project
revision, catalog generation, exact pinned and candidate roots, candidate root
digest and both footprint addresses. `delta.pads` lists changed pad numbers in
lexical order, with `before` and `after` land geometry and board positions, their
change flags, and `center_shift_nm` for a same-number pair. An added or removed
land has one absent side. Renumbering appears as an addition and removal; the
query does not pair them or resolve the pad map. `delta.pad_numbers_changed`
reports whether the complete number sets differ. The mask/paste fields describe
nominal pad openings (native lands have no independent process expansions), not
separate technical-layer artwork. A geometry-identical revision has no pad rows.

A `loose_ends` row means an existing straight track cap or through via touches
the old pad on shared copper but would no longer touch its same-number
counterpart. Standalone circular track ends are not evaluated. Each row labels
the copper ID and end (`track_start`, `track_end` or `via`), its existing board
position and its track layer, if applicable. `gap_nm` measures separation between
filled outer contours, not pad center displacement. It is null if the pad is
removed or no longer shares a copper layer. This native approximation ignores
bores in pads and vias: a cap inside a bore may count as old contact without
touching the annular copper. A drill-only change does not trigger a contact
check. This is not a net connectivity or DRC report.
On an unconfigured board no copper contact is evaluated. Each reply can report
up to 10,240 loose-end rows, independent of board capacity. An over-limit request
returns `cap_exceeded`, not a partial list. If the full report exceeds the limit,
try an exact string pad number in `pad`. If that pad also exceeds the limit,
inspect its copper in the board editor; the query cannot split one pad's list.
The query does not move copper or alter pins, saved bytes, project revision or
Undo/Redo.

## Adopt non-geometric revisions

With a project open, choose **Review library updates** in Parts Library, or
**Updates** on the ribbon's Libraries tab, which opens the same review. Select
instances, choose **Preview selected updates**, and read the exact before and
after fields. **Apply reviewed updates** uses the same command and Undo history
as the schematic editor. **Undo adoption** reverses the change directly from
its result. A project or source change clears the current review or result;
open the review again to see the current state. A pending library folder
operation temporarily prevents review and adoption in the desktop and live
MCP editor; the pane then shows that refusal in place of the review button.
Previews are read-only and also run in read-only headless sessions.

`library.preview_adoption` reviews the latest loaded symbol or generic-part root
for the chosen `instances` (a nonempty list of distinct `component-N` IDs).
Use the `expected_revision` from project inspection. The result lists the exact
old and new pins, candidate digest and changed text per instance without making
an edit. Pass the returned `preview` fingerprint with the same parameters to
`library.apply_adoption`. It refuses a different change set and otherwise makes
one revision-checked edit with one Undo step. Both return the change record to
live and headless MCP even with `inspect: false`.

For plain symbols, this slice allows a new name or default value while preserving
all placed values, geometry and electrical behavior. Generic-part updates allow
a new name with identical selected dependencies and requirements. Explicit
manufacturer selections still pin the old generic part revision: use a separate
review for those. Standalone footprint updates, changed symbols' drawings or pins,
and changed package maps are not adopted by these commands. A conflict, unresolved
candidate or ineligible selection refuses the entire batch; there is no fallback
to a different revision.

## Find where a revision is used

With a project open, select an exact item revision in Parts Library and click
**Where used** in Preview. Category rows are available in **All** for loaded
sources and for categories pinned in the Project inventory. A standalone symbol
uses its exact category binding; a selected native part uses its own category,
not the symbol's default. The report lists actual instances with their reference
designators and board names where applicable. Run the query again after a project
or source change. The Project table's **Used by** column instead describes the
saved dependency inventory; it can include unselected package options and
accepted history that no longer has a placement.

`library.where_used` takes `library`, `kind`, `definition` and
`definition_revision`; `kind` is `symbol`, `footprint`, `part`, `category` or
`manufacturer_part`. This is the same read-only query in live or headless MCP,
including with `inspect: false` and in read-only headless sessions. The item
address includes its exact revision; a later revision is not substituted. An unused or unknown address has no
instances. The query does not refresh sources or change project state or Undo.

`instances` has one row per matching logical instance, even when multiple units
are placed or a footprint is both assigned and placed. Each row includes the
stable `part_instance` ID, display `reference`, optional generic `part` address,
and optional `board` with its `id`, `name`, `footprint` placement ID (or null)
and `pending_removal` flag. For footprint queries, `assigned` distinguishes a
current exact package choice from a physical footprint still awaiting a
reviewed replacement or removal. `assignment_excluded` marks a desired package
for a component excluded from the board. It has `board: null` unless a real
placement of that revision is still pending removal. A retained placement keeps
its former instance ID and reference; a boardless assignment also has
`board: null`. Unselected package options and history with no placement do not
count as uses.

The report also returns `project`, runtime `revision`, `catalog_generation` and
the requested `item`. `revision_info` follows [the loaded-source comparison
above](#query-available-revisions) for a project pin or exact loaded item; it is
null when neither can be inspected. `newest_revision` is true for `Matches
source`, false for `Update available`, and null when the loaded catalog cannot
establish either status. This is not a compatibility or adoption decision.
The query does not search other projects or address future model and reusable
module records.

## Edit and publish native items

On desktop, **New → Library** creates a native library in an unused folder and
registers it after its manifest is durably written. Choose its name, portable
library ID and license before selecting the folder. The `user` placeholder,
`project-` IDs and bundled identities cannot be publishing destinations. External
source manifests claiming `project-` IDs are refused; existing project-pinned
records keep their addresses.

Select a symbol or footprint in a writable library and choose **Edit** to open
that exact revision in its existing editor. **Edit copy** creates a derivative
under a fresh item identity and is also available for read-only libraries.
**New → Symbol / Footprint** starts an independent draft. Generic-part authoring,
comparison, export and Git/remote connections remain separate work.
Both definition editors use the shared editor zoom range up to 102400%, with the
same pointer anchoring and pan controls as PCB layout. Fit uses the available
canvas around panels without magnifying its screen margin with tiny artwork.

In either editor, **Library → Publish…** (or Ctrl/Cmd+S) chooses a writable
registered native folder. The first successful publication binds the editor to
that library and item; later changed publications retain its identity and add
immutable revisions. A known newer revision requires confirmation because the
older draft may omit intervening changes; publishing does not merge them.
A reopened draft without a recorded published base also requires confirmation
when the item already exists. The editor status bar shows its collection and
publication state. New items inherit the writable collection selected in Parts
Library, and their Publish dialog can change that suggestion.

Choose another collection in Publish to use **Publish copy**, or select
**Save as → New item** to make a derivative within the same collection. The
original stays unchanged, and the editor continues on the copy after a successful
write. **New library…** is in the collection selector; creation returns to Publish
with that collection selected. Cancelling or failing a copy leaves the original
editor identity and all published records intact. The separate **Edit copy**
action remains available when an independent draft workspace is preferred.
Symbols inherit the manifest license, so their recorded license must match the
destination; footprint records retain their own license.

Preparation validates captured geometry and returns native documents without
confirming a save. The host rechecks the manifest and latest disk revision before
publishing complete, synced bytes under an unused filename. Write failures leave
the draft and old revisions intact. A successful write followed by a failed
refresh is reported separately and does not republish the record. If durable
confirmation is uncertain, Publish verifies or resumes that same captured
revision before allowing another publication. Newer edits remain unsaved.
Publication and source refresh never change placed definitions or project history.

**Save Library Copy** saves an unfinished `.pcitem.json` draft outside the catalog;
it does not publish it. Published records have no companion editor file. Opening
a published item reconstructs repeatable session primitive IDs in native record
order while preserving terminal, pad and unit IDs. Browser editing and downloads
remain available; publishing and native library creation are desktop-only.

## Shared commands

GUI source loading and MCP use the same validated catalog transaction. Native
folder reads and desktop bundle parsing run on a single background worker;
MCP supplies already transported UTF-8 document text rather than asking the
engine to read a path.

| Command | Parameters and result |
| --- | --- |
| `library.load_source` | `source` is a personal portable ID; `documents` maps supported relative paths to file text. Returns source inspection. |
| `library.list_sources` | No parameters. Returns source identities, trusted `builtin` status, licenses, counts and file/conflict diagnostics. |
| `library.search` | Optional `query` and `kind` (`symbol`, `footprint`, `category` or `part`). Returns current unambiguous items, their exact keys, formats, provenance and content digests. |
| `library.remove_source` | `source`. Removes an external catalog source without changing pinned definitions; bundled sources are read-only. Use `app.library_source` in the live editor to also forget a personal registration. |

Part search also matches authored description and keyword text, but not link
labels or URLs; see [descriptive metadata](part-schema.md#descriptive-metadata-and-search)
for the normalization and bounds.

Use `execute` with normal inspection enabled to receive these results;
`inspect: false` intentionally returns only the unchanged project revision for
these source commands. The focused headless `library.preview_pin_remap` query is
an exception: it preserves its candidate/current-evidence report with `inspect:false`.
The live `app.library_pin_remap` window command instead follows normal editor
inspection suppression; use inspection to obtain its captured token/report.
See [reviewed pin renumbering](properties-and-library-editors.md#review-pin-renumbering).
For an explicit-library `schematic.place_symbol`, pass `library`, `definition`,
`definition_revision` and `definition_digest` from search, plus the usual sheet,
coordinates and `expected_revision`. Placement checks both project revision and
the exact definition content, so a source refresh during a preview cannot
substitute another symbol. Existing starter-only calls remain valid.

A removed source cannot be restored by a late worker result. A source change
through MCP while a worker is loading invalidates that delayed result. A blocked
filesystem read retains the single worker slot until it returns; removing its
registration discards the result, and application shutdown does not wait for the
reader. Project commands and editing remain independent of the reader.

Native part/category definitions and exact dependency resolution are described
in [part-schema.md](part-schema.md). Native part selection pins its complete dependency closure into project
instances. Reviewed project updates, additional library authoring tools, Git/providers and
external-format imports remain separate work. The native source, assignment and
offline handoff behavior is the bounded library MVP in #92.

## Part definitions and categories

The [native part example](../examples/native-parts/README.md) exercises the same
folder/bundle path with a resistor, connector and IC. Part/category files are
validated independently; complete resolution checks their exact cross-library
references and typed values. Unresolved items stay inspectable and show their
diagnostics rather than silently choosing another revision.

| Command | Parameters and result |
| --- | --- |
| `library.resolve_part` | `library`, `definition`, `definition_revision`. Returns the exact part, symbol, category and package footprints after semantic validation. |
| `library.parse_quantity` | `quantity`, `value`. Normalizes compatible unit-aware input into exact canonical base-unit JSON. |

Both are read-only shared engine operations and need no open project. Resolving
a definition neither pins it into a project nor changes assignment/placement.
Native part Value controls normalize the explicit primary quantity. The part
selection form edits typed requirements, package and MPN; symbol-only Value
controls retain free text.

`schematic.place_part` accepts the exact part identity and `definition_digest`
from `library.resolve_part`, coordinates and `expected_revision`. Optional
`package`, typed `requirements`, `manufacturer` reference and
`manufacturer_digest` select the initial combination. `part.set_selection`
accepts the instance `component` ID, optional changed selection fields and
`expected_revision`. Omitting a field preserves it; null clears a package or
manufacturer. A placed footprint must be removed before its package is cleared.
Manufacturer search uses `kind: "manufacturer_part"`; item inspection includes
the record and digest. A part's digest covers all its dependencies, so refreshing
a category or footprint invalidates a pending placement too.

## Live workspace commands

`app.set_workspace` accepts `workspace: "library"`, an optional source `view`
and `placement: "here"`, `"beside"` or `"window"`. `inspect_editor` reports
`workspace: "library"` and a per-view `library_workspace` containing source,
Type, query and selected exact key. `app.library_browse` changes those view-only
choices; `type` is 0 All, 1 Symbol, 2 Footprint, 3 Generic part, 4 Orderable part.
`app.library_source` uses the same source actions as the ribbon: `add`, `refresh`,
`relocate`, `remove`, `delete`, `export`, and desktop `new` (a metadata form and folder picker). Add/relocate/export open host pickers, and Delete retains
the ordinary confirmation. Source folder I/O never runs on the editor thread.

`app.library_catalog_edit` addresses a Parts Library `view` and opens its selected
exact symbol/footprint; `copy: true` creates a new identity. `app.library_browse`
accepts `definition_revision` with `item` to select an older revision explicitly.
`app.library_publish` takes `document`, `expected_revision`, a registered writable
`source`, and optional `allow_older` or `copy`. A different destination library
creates a new item identity; `copy: true` does the same within one library. These live commands share the GUI handlers.
`app.library_copy` creates an independent copy of an open draft.

Headless or live MCP can call `library.prepare_publication` with the captured
`item` envelope, transported destination `documents` (including `library.json`),
optional `base_revision`, `allow_older` and `copy`. It returns an address, native document
text and `saved: false`, including with `inspect: false`. It accepts no filesystem
path and neither writes nor binds an editor. Destination resolution, durable
publication and catalog refresh remain host responsibilities.

`inspect_editor.library_destinations` reports writable host source IDs, and
`library_editors.publication` reports in-flight or unconfirmed save ownership
without filesystem paths. Later live edits remain unsaved when a captured
publication finishes. Save As changes the edited identity and invalidates
property drafts or gestures captured for the original, without discarding them.

## Generic and orderable part documents

In **Parts Library**, choose **New → Generic part** to open a part document in
the Items region. The Browser and Preview remain available, and the named document
tabs switch between Browse and retained part documents. A published symbol
also offers **Create part from this symbol** in its editor. **Edit** reopens an
exact published generic or orderable part; **Edit copy** gives the derivative a
new identity while preserving its licence and source provenance.

Use **Browse** beside Symbol, Footprint, Category or Generic part to open the
shared searchable chooser. Narrow results by library, inspect the native preview,
and confirm with Choose. Arrow keys move the selection and Enter confirms it;
Cancel preserves the current association. Results default to the latest revision
of each item, with **Include older revisions** for an explicit historical choice.
Opening an already selected older revision retains that exact selection. Results
are paged in sets of 50, and matching metadata is cached until the search, filter
or loaded sources change. The form itself shows only its chosen association.

Symbol and footprint associations are optional. Use **Clear** to remove a symbol;
its pin maps clear in the same undoable edit. **Add footprint…** opens the chooser
and creates a package only when Choose confirms a footprint. Cancel leaves the
part unchanged. A footprint-only part can publish without a dummy symbol. Choose
an optional category and its primary quantity field. Fixed parameters use the category's typed input parser; Apply value commits
an entered value, Reset value restores it, and applying an empty field removes
that fixed requirement. Each package selects an exact footprint revision. When a symbol is present,
explicitly assign every physical pad to a symbol terminal or to Unconnected.
Several pads can belong to one terminal. Selecting a mapping row highlights its
pad and corresponding terminal, including a terminal on another symbol unit.
Units are read-only here and remain editable in the Symbol Editor. The model
slot is reserved for future package-bound 3D support.

The part document also carries **Description**, **Keywords** and **Links**
fields: a one-line summary, a list of search tags and a list of label/URL
pairs. Leaving a field blank, or removing its last row, leaves that key out of
the native record entirely — no `null`, no empty array. The
[descriptive metadata contract](part-schema.md#descriptive-metadata-and-search)
defines the exact character/count bounds, case-insensitive duplicate rules,
HTTP(S) URL requirement and the rule that a loaded record never fetches a link.
Draft edits check those bounds before they are accepted; publication checks the
declared envelope and resolves the exact dependencies.

The first description, keyword or link selects the `pcbcraft-part/5` envelope
for the draft. Removing the last metadata field downgrades a rated draft to
`pcbcraft-part/4` and an unrated one to `pcbcraft-part/2`; adding metadata to a
retained format-1 draft upgrades it. Publication reselects and validates the
envelope. Because the full definition is pinned with the part, editing its
metadata changes the dependency digest and invalidates a placement captured
against the earlier content. Descriptive metadata adds no electrical
requirement: schematic placement still needs an exact symbol.

The fixed footer has the primary **Publish…** action and publication/destination
state. It opens the destination form; choose a writable library, or use **New
library…** when none exists, then confirm Publish. The new-library form asks for
a name and folder before returning to the captured publication.

Publish validates each declared symbol, category, typed value and package
map, then uses the same native-folder destination, no-clobber write and recovery
path as drawing publication. Only resolved published records enter placement.
An incomplete map stays in the document and cannot replace a valid published
revision. Part documents have their own revision-checked Undo/Redo and are kept
only in the current application session; closing changed work requires an
explicit discard decision. They have no persistent draft-file format.

Once the generic part is published with a package, **Add orderable part** opens
its manufacturer/MPN document, with package, optional datasheet and typed
specifications. Datasheet links are saved as text and are never fetched on load.
The exact owning generic part must already resolve before its orderable record
can publish. Both records use the ordinary Parts Catalog placement and offline
project-pinning path; plain symbol placement remains available.

Drawing editors show the loaded generic parts that use their published revision.
Publishing a later drawing revision offers those parts for review. Each selected
part opens with the exact reference advanced and its existing maps preserved;
publishing that part is a separate explicit action. Mapping problems must be
corrected before it can publish. This flow changes reusable source definitions;
reviewed adoption by already placed project instances belongs to #312.

Live MCP exposes `app.part_new`, `app.part_orderable`, `app.part_edit`,
`app.part_history`, `app.part_reference` and `app.part_focus`, using explicit
document IDs and captured revisions from `inspect_editor.library_editors.part_documents`.
`app.library_publish`, `app.library_catalog_edit` and `app.library_close` also
accept these documents. `library.prepare_part_publication` prepares validated
native part/manufacturer documents in a headless session, using already loaded
exact dependencies and transported destination documents. It accepts no host
filesystem paths and never confirms a save.

`app.part_reference` chooses a typed association field at the captured document
revision. A `new_package` field includes a new stable package ID and creates it
only when an exact footprint is confirmed. A footprint field includes its stable package ID; changing it clears
that package's old map before explicit remapping. Category and generic-part
changes clear only their dependent values. Re-selecting the same exact address
is a no-op. `library.search` accepts `include_older: true` to browse the same
available, unambiguous exact revisions through MCP; default search stays latest-only.

Categories are optional typed specification definitions. The bundled starter
collections do not contain categories; an empty chooser reports that none are
loaded. The Native Parts Demo includes resistor, connector and IC categories and
can be loaded through Add library when testing typed fields.

## Personal discovery preferences

Preferences → Libraries has separate **Basic components** and **Power** switches.
They control available built-in sources for new choices; project-pinned symbols
and footprints remain available offline and retain their geometry. External
registrations are managed through the existing library source manager.

The same page configures optional **Parts Catalog placement filters**. Require a
manufacturer/MPN, a resolved footprint with a valid pin map, or select a category
and require typed parameters to be present or match Exact, At least, At most,
Range or One of comparisons. The existing category schema defines field identity,
type and units. Missing specifications fail a required condition; choosing an
orderable part uses that record's specifications rather than generic assumptions.
Schematic-only power symbols remain available. These personal settings filter the
placement chooser only, leaving Parts Library authoring and existing project
edits unchanged; they are not a project validation policy.

The expanded catalog starts at about 70% of the window and remembers resizing
through its lower-right corner. Its result cache tracks source and filter changes,
while the existing list renderer draws only visible rows. Search covers all
loaded libraries, part names/IDs and manufacturer/MPN metadata. Native source
bounds allow 32 registrations including built-ins, 8,192 files per source and
32 MiB per source, with the existing 1 MiB per-file limit.

Expanded Parts Catalog results use a dense table with Part, Default value and
Library source columns, with the selected symbol and package details on the
right. The four tile-size buttons remain confined to the compact panel. The
preview divider is draggable and its width is saved independently of the dialog
size; the initial preview is narrower so additional results fit beside it.

Live MCP uses `app.set_preferences` for the same discovery settings:
`builtin_basic_components`, `builtin_power` and an optional complete
`placement_filter` object. Missing top-level settings stay unchanged, while the
filter object replaces its prior criteria. Inspection returns
`library_preferences` including the exact saved category and typed comparisons.
Invalid criteria reject the complete settings update before changing preferences,
source availability or project state.

The right pane shows separate symbol and selected-package footprint canvases,
using the Library Manager's independent Fit/zoom controls, wheel zoom and drag
pan. Changing the selected package resets only its physical preview. Symbol-only
records and unassigned packages show their missing-footprint state without
inventing a physical association; package choice remains explicit.

Repeat placement is in the expanded catalog's bottom action bar beside Cancel
and Place. Browser folder and bundle preflight receive their bounds from the
same engine constants as native loading, including the 64 MiB escaped bundle
transport bound.
