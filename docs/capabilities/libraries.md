# Libraries and part definitions capabilities

## Libraries

### Available

- Bundled Basic components and Power native libraries with ordinary component instances, explicit
  global-terminal roles and editable instance defaults.
- 30 Basic components and 11 Power symbol types.
- 18 original SMD and plated-header footprint definitions.
- Saved built-in library switches and configurable typed placement filters.
- Native generic part/package/MPN selection, typed exact/min/max/range/one-of requirements with
  canonical units and compatibility findings, and complete pinned dependencies survive offline
  reopen.
- up to 32 independently placed schematic units per part with shared terminal identity, fields and
  package assignment across sheets.
- Independent symbol/footprint authoring workspaces available alongside schematic/PCB in the shared
  workspace picker, with Draw/Library/View ribbons, mixed splits, detach/dock/native windows,
  per-pane navigation/selection and primitive properties, integrated pin leads/labels/styles, oval
  and alphanumeric pads, editable curves/text and technical layers, generated footprint references
  with editable default offsets, group transforms, numbered duplication/arrays, internal clipboard,
  native validation, separate undo, draft-save/open/export, desktop native-folder publication with
  stable item identities and immutable revisions.
- Generic/orderable part documents inside Parts Library with exact symbol/package references,
  explicit complete pad maps, footprints of up to 1,024 pads and 256 artwork objects and symbols of
  up to 1,024 terminals, typed fixed values/specifications, authored part description, keywords and
  labeled HTTP(S) links, separate Undo/Redo, session-only drafts, publication
  and dependent-part reference review.
- Live MCP.

### Remaining scope

- Further library/source workflows [#92](https://github.com/tangency-labs/pcbcraft/issues/92).
- Advanced authoring, project application and persistent definition layouts
  [#78](https://github.com/tangency-labs/pcbcraft/issues/78)/#79.
- Git/catalog sources [#93](https://github.com/tangency-labs/pcbcraft/issues/93)/#94.

## Native library sources

### Available

- Read-only Built-in Basic components/Power registrations.
- Parts Library workspace with Project/Built-in/Sources groups, exact project-pinned revisions,
  usage and source status, offline previews, native source previews and nested/flat orderable
  records.
- Desktop remembered folders, no-clobber bundle installation and confirmed system-trash deletion.
- Whole-library bundle export (desktop file, browser download).
- Browser folder/bundle reads with names retained for reopening.
- Symbol/footprint/part/category/manufacturer search with file/conflict diagnostics, exact part
  resolution and canonical quantity normalization.
- Native part placement and typed requirements.
- External footprint assignment.
- Complete project pinning and offline reopen after source relocation/removal.
- [source guide](../libraries/native-library-sources.md).

### Remaining scope

- Git sources [#93](https://github.com/tangency-labs/pcbcraft/issues/93), provider catalogs
  [#94](https://github.com/tangency-labs/pcbcraft/issues/94), browser/remote publishing, and adoption
  of geometric, pin-map or orderable-selection changes.

## Implementation notes

[Footprint definitions can include circular non-plated through holes](../libraries/footprint-library.md#definition-and-placement-contract),
with editor controls, placement transforms, offline storage, and mechanical
obstacles for routing and design checks. They have no copper or net, and
fabrication exports a [separate non-plated drill file](../manufacturing/drill-files.md#fabrication-drill-files)
when a board contains them.

[Plated footprint slots](../libraries/footprint-library.md#definition-and-placement-contract)
are authored and inspected independently from the copper land. The board shows
the finished opening on rotated and back-side placements and checks its full
annular ring. Fabrication export refuses slot-bearing boards until plated-slot
machining is supported; non-plated slots are not supported.

Typed quantities accept and display engineering prefixes from atto (`a`) through
exa (`E`), including femto (`f`) and peta (`P`), with exact canonical values and
round-trip editing. See the [quantity contract](../libraries/part-schema.md). Generic parts can author
[device supply ranges](../libraries/part-schema.md#device-supply-range) for their power inputs.
The pinned limits appear in schematic Pin functions and part inspection, and
carry through offline saves and native clipboard copies. Missing bounds are
unknown; electrical rule checking does not yet consume them.

The **Parts Library** workspace browses native libraries with search/Type, nested and
flat orderable parts, native previews, revisions and dependencies. Desktop source
actions include remembered folders, refresh/relocate, bundle installation into
unused folders, removal and confirmed system-trash deletion of whole libraries, plus individual
generic/orderable identities on Unix desktop with dependency and open-document
guards. Browser folder/bundle
reads remember names that need reopening after reload. See
[native library sources](../libraries/native-library-sources.md) for platform limits. Share ›
Export writes the selected library to one bundle file. Disabled planned controls
do not yet provide comparison or connections.
Desktop New library, New symbol/footprint, Edit and Edit copy use the existing
authoring workspaces; Publish writes immutable native revisions into writable
registered folders. Browser publishing remains unavailable.
The symbol editor can declare [alternate pin functions](../libraries/symbol-schema.md#alternate-pin-functions)
with an electrical type. A placed part's **Pin functions** tab and
`part_instances.set_pin_function` select one per terminal. Inspection, drawn names,
the netlist's terminal metadata and the focused no-connect check use that choice;
wire endpoints and net membership do not change.

Symbol **Library → Remap pins…** and live `app.library_pin_remap` preview explicit
pin renumbering with separate draft names and pinned-before-number names/nets/native
package-pad evidence before an effective
change becomes one revision-checked library undo step. An explicit unchanged map
is a no-op. Headless `library.preview_pin_remap` prepares the same read-only report.
The two remap commands use exact decimal strings for their routing identities and
revisions; existing native records and other commands keep their codecs. Different,
unassociated or known conflicting revisions do not supply current pin names, nets
or pads; placed copies, wires and native part maps stay unchanged. Preparation refuses more than
4096 affected rows before building a report. All routes charge report evidence
against 4 MiB, even with no matching copy. Only a lookup for an associated,
placed terminal triggers the modeled whole-project graph-input and path-text
checks, including unrelated sheets. Native
capture separately refuses an editor-inspection reply exceeding the existing live
JSON byte limit. These budgets are not limits on total algorithmic operations,
graph latency or peak heap. It does not support every
combination of the larger saved project/terminal limits. See
[reviewed pin renumbering](../libraries/properties-and-library-editors.md#review-pin-renumbering).

The shared `library.available_revisions` query lists existing instances with newer
loaded native revisions, including through live/headless MCP with `inspect: false`.
It preserves project pins and history, reports missing/conflicting/unresolved
sources explicitly, and makes no compatibility or adoption decision; see
[query available revisions](../libraries/native-library-sources.md#query-available-revisions).
Reviewed [adoption](../libraries/native-library-sources.md#adopt-non-geometric-revisions)
moves chosen instances to a newer symbol or generic-part revision whose changes
are limited to names and default values, through `library.preview_adoption` and
`library.apply_adoption` or the Parts Library review, as one undoable edit.
[Preview footprint pad changes](../libraries/native-library-sources.md#preview-footprint-pad-changes)
reports altered pads and lost contact for straight track ends and through vias
on one placed instance. It does not check standalone circular track ends, move
copper, resolve pad maps or adopt revisions.
The shared read-only `library.check` command reports repeated footprint pad
numbers and incomplete or inconsistent package maps in transported library
documents, a footprint draft or a part draft. Each finding names its rule, exact
subject, authored pad occurrences, measured count, limit and basis. It runs in
read-only sessions and through MCP with `inspect: false`, changes nothing, and
does not relax loading or publication; see
[check library items](../libraries/native-library-sources.md#check-library-items).

Parts Library Preview and `library.where_used` list actual instances of an exact
native item revision in the open project, separating footprint assignments from
physical placements and showing whether a newer loaded revision exists. See
[where-used](../libraries/native-library-sources.md#find-where-a-revision-is-used) for the
read-only result and its limits.

Generic-part authoring supports independently optional symbols and footprints in
`pcbcraft-part/2`, or `pcbcraft-part/5` when the definition carries a
description, keywords and labeled links, including footprint-only definitions
and definitions with no drawings. Retained
format-1 parts remain readable. Parts Library can publish, search and preview
these definitions and their manufacturer records. Schematic placement requires a
symbol; direct board-only part placement is not yet implemented.
