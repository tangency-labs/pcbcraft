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

**Remove** forgets the registration/cache and leaves its files intact. **Delete**
is desktop-only for writable registered folders. Its confirmation names the
folder, counts native item revisions and explains that projects keep their
pinned copies. Pending editor changes from that library block deletion. The
host rechecks the source and folder before moving it to system trash; failure
retains the registration and reports an error. Linux/FreeBSD use the desktop
`gio trash` service, macOS uses Finder, and Windows uses the recycle-bin API
through PowerShell. An unavailable service has no permanent-delete fallback.
Built-in libraries cannot be refreshed, relocated, removed or deleted.

To try the original MIT OR Apache-2.0 examples, add `examples/native-library`
or `examples/native-parts`, or package one for transfer:

```sh
python3 scripts/package-native-library.py examples/native-library /tmp/workshop.pclib.json
```

The packager preserves existing output. The bundle's `documents` object maps
native relative paths to UTF-8 file contents, validated with the same engine
rules on both platforms. Press **A** in Schematic to use the existing Parts
Catalog placement picker; Library browsing itself never places an item.

The source loader accepts at most 18 registrations including the bundled libraries, 256 files per
source, 1 MiB per file and 8 MiB of decoded UTF-8 content per source. Bundles are
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

Use `execute` with normal inspection enabled to receive these results;
`inspect: false` intentionally returns only the unchanged project revision.
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
`relocate`, `remove`, `delete`, and desktop `new` (a metadata form and folder picker). Add/relocate open host pickers, and Delete retains
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
