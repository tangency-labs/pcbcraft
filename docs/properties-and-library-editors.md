# Properties and library editors

The properties inspector and the symbol/footprint editors share Craft controls,
but edit different things. Properties changes a selected project's component or
placement. A library editor authors a definition with its own identity, draft,
validation, undo and save lifecycle. Opening a definition must not silently change
its placed copies.

The user requested compact and expanded properties following PCBcraft Next, plus
symbol and footprint editor workspaces on November 7, 2026. The presentation
reference is Next's `src/desktop/inspector.rs` at
`568cd495397a91563e563e7163efb226d84fa83a`, MIT OR Apache-2.0, and the user's
read-only `02-schematic-properties.png` FigCraft export. Neither design files nor
the prototype are modified by this work.

## Product notes from published manuals

These observations describe documented behavior, not independent usability tests.
Only publisher user documentation was consulted. No KiCad/Altium implementation,
icons, artwork, library assets or example project files informed the implementation.

| Reference | Documented feature | PCBcraft direction |
| --- | --- | --- |
| [KiCad 10 Schematic Editor](https://docs.kicad.org/10.0/en/eeschema/eeschema.html), Editing symbol properties / Properties Manager | A selection inspector complements a detailed properties dialog; compatible properties can be edited across a selection. | Keep the compact inspector and expanded form on the same command path. Add bulk editing only for explicitly supported common properties. |
| Same manual, Creating and editing symbols | The symbol editor separates graphics, terminals and fields; library edits and schematic-copy edits have different update scopes. Pin tables support larger devices. | Preserve stable terminal identities. Make source versus project-copy editing explicit, and review updates to existing components. Multi-unit authoring is available; pin-table import remains later bounded work. |
| [KiCad 10 PCB Editor](https://docs.kicad.org/10.0/en/pcbnew/pcbnew.html), Footprint editor / Footprint pads | Footprint authoring has its own canvas; pads have explicit numbers, geometry and technology. Repeated placement, pad tables, arrays and defaults assist larger packages. | Start with original SMD and plated-hole geometry supported by the native schema. Keep pad numbers distinct from positions and preserve mappings. Provide arrays with automatic unused numbers; leave bulk pad tables for a subsequent slice. |
| Same manual, Checking footprints | Checks cover courtyard problems, holes/annuli, incompatible layers and copper or silkscreen collisions. | Report the checks actually implemented. A definition passing schema validation must not be presented as manufacturing-qualified or as passing full DRC. |
| [Altium: Creating a Schematic Symbol](https://www.altium.com/documentation/altium-designer/components-libraries/creating-schematic-symbol) | Graphics and electrically active pin ends are separate; symbol-level and primitive-level properties are contextual. | Keep the selected primitive's inspector separate from definition metadata. Show connection points clearly while editing. |
| [Altium: Creating a PCB Footprint](https://www.altium.com/documentation/altium-designer/components-libraries/creating-pcb-footprint) | Origin/grid setup, numbered pads, overlays and mechanical graphics support footprint construction; dimensions should be checked against component specifications. | Make the placement origin visible, support exact unit-aware dimensions and separate copper, silkscreen, fabrication and courtyard purposes. Mask/paste behavior and 3D need explicit subsequent contracts. |
| [Altium: Elements of the Environment](https://www.altium.com/documentation/altium-designer/design-environment-elements) | Properties changes context with the active document/selection and can use collapsible regions; document editors and host panels have separate roles. | Retain PCBcraft's ribbon, Browser, floating cards, themes and view ownership. Use contextual sections rather than copying Altium's visual chrome. |

The manuals were consulted on November 7, 2026; the Altium symbol and footprint
pages identify October 15 and October 14, 2026 updates respectively. These links
are behavioral references, not permission to inspect implementation pages or to
reuse separately licensed content.

## Properties behavior

The compact inspector belongs to the focused editor's selection. Counterpart
highlights do not grant edit authority. Reference, value and orientation are
applied atomically; typing in a draft does not change the project. Expanding the
form does not create another draft or undo history. Invalid input stays editable,
Cancel leaves the document unchanged and stale view/project revisions are refused.

Footprint selection includes a physical preview, explicit terminal-to-pad map
and an expandable source/license section. Assignment pins a definition without
placing copper. Initial board placement stays a separate command. A placed
package cannot be silently exchanged by this chooser.

## Workspace and window ownership

Symbol Editor and Footprint Editor appear alongside Schematic and PCB layout
in the shared workspace picker. Each has Draw, Library and View ribbon tabs,
a canvas, Browser and compact/expanded Properties. The picker can open them
here, beside the current pane or in a new native window. Their View ribbon
also provides Split, Detach/Dock and New window, using the same host/layout
controller as project views. Browser builds support splits within one tab.

Up to four definition documents can remain open, each with one session and undo
history shared by its views. Selection, camera, panel visibility, ribbon tab and
pending fields belong to individual panes. Closing a pane or non-last window
retains the document and its unapplied fields; Review edits can recover fields
from a closed pane. The last window and explicit Quit protect unsaved definitions
before the ordinary project-close decision. Closing the original native window
preserves the remaining window's identity and services.

Definition pane placements currently last for the application session. While
library panes are present, the last saved project arrangement is retained rather
than writing runtime definition IDs into project arrangement storage. Arrangement
creation, selection, renaming and removal are refused while library panes remain
in the layout. Opening a project binds arrangement bookkeeping immediately while
preserving those panes; returning to a project workspace does not trigger a
delayed saved-layout restore.

Each definition document owns its history and dirty state. A canvas owns only
selection, camera and tool drafts. Closing or moving a view must not publish a
definition, discard unsaved work without a decision or transfer project ownership.
The existing project remains the authority for applying a reviewed definition
change to a placed component. Project-local variants leave source libraries alone.

These notes do not close #50, #78 or #79 by themselves. Electrical-rule coverage,
further source integration, manufacturing layers, arbitrary shapes
and reviewed library-to-project updates need their own supported contracts and
acceptance evidence.

## Authoring basics

Choose **Symbol Editor** or **Footprint Editor** from the workspace picker.
The picker remains accessible in compact headers, and each editor's **Library → New**
creates another definition of its kind. **Library → Open** loads a saved item.
Switching through the picker retains document-local selection, camera, ribbon,
panel visibility and unfinished property fields. It cancels an unfinished pointer
gesture without committing it.

The project **Libraries** ribbon groups source management, saved-item Open,
Resume and **Edit copy**. Edit copy forks the focused view's selected symbol or
physical footprint under a new identity, entering its matching editor workspace
while leaving the source and placed instances unchanged. File has no workspace
launchers; inside an editor it holds document operations.
The browser tree selects definition metadata or a stable primitive ID.
Compact and expanded properties share one revision-bound draft, including invalid
typed lengths; Apply commits once, and Reset discards only those field edits.

Symbols have integrated pin leads with length, four directions, name/number visibility,
label sizes and line, inverted, clock or inverted-clock styles. Electrical type is
independent of the graphical marker. Newly created pins have a visible lead;
older saved items without appearance keep their separate drawing intact. An explicit
Edit copy integrates a legacy lead only when its axis-aligned stroke is unambiguous;
a legacy pin can also gain a lead through Properties → Add pin lead → Apply.

Both editors draw lines, rectangles, circles, parameterized arcs, polylines and
stroke text. A two-click arc starts as a semicircle; Properties sets its exact
start/sweep angles. Polyline clicks add vertices; Enter/double-click finishes an
open path and clicking the first vertex closes it. Symbol bodies support no fill,
solid fill or canvas-background fill (filled polygons must be convex). Footprint
artwork carries fabrication, silkscreen or courtyard purpose and physical stroke
width; footprint fills are not supported. Text retains editable content, size,
alignment and rotation rather than becoming anonymous line segments.

Footprint **Definition** properties can enable a generated silkscreen reference
and set its default X/Y offset. The editor shows `REF` at that position, and the
native definition retains the offset when exported or forked.

Footprints support SMD/plated through-hole rectangular, rounded, circular and oval
pads. Pad numbers accept the same 1–8 alphanumeric/`+`/`-`/`_` spelling as terminal
identifiers. Through-hole selection supplies a drill default and validates a
positive annulus; circles share one diameter. Slot/pad-stack/mask/paste editing
is still outside this slice.

Modifier-click and directional selection boxes select groups; the Browser supports
modifier selection too. Draw/Modify/Copy ribbon groups expose move, quarter-turn
rotation, mirrors, delete, duplicate, clipboard and rectangular arrays. An array
includes its original cell, accepts unit-aware row/column pitches and creates unused
electrical numbers in row order. The whole operation is one validated undo step.
Duplicate and paste have a cancellable placement preview. The internal clipboard
works between documents of the same kind. Pin/pad repeat placement inherits the
selected prototype's dimensions, type and orientation while allocating an unused
number. Selected pin names remain unchanged so users can name related signals.

Fields accept explicit mm, mil or inch lengths and store integer nanometres.
Drawing and movement snap to the editor grid; symbol attachment points retain the
native 1.27 mm grid and ±12.7 mm limit. Non-electrical symbol drawing may extend to
±100 mm. Set origin moves the entire definition relative to one selected anchor,
subject to the same validation. The origin remains visible on the canvas.
M starts a move, R/Shift+R rotates, P places a pin/pad, L draws a line, T places
text, E/double-click opens expanded properties and Escape cancels a gesture.
Ctrl/Cmd+A/C/V/D select all, copy, paste and duplicate in the focused canvas.
Schema validation checks geometry, unique numbers and valid drills/annuli; it does
not perform manufacturing DRC. Drafts can temporarily have no terminals or pads
and still save/reopen. Native export requires a terminal for a symbol, or a pad
or circular non-plated hole for a footprint.

The editable JSON envelope uses `pcbcraft-library-item/1` for single-unit symbols
and footprints without non-plated holes, `pcbcraft-library-item/2` for multi-unit
symbols, and `pcbcraft-library-item/3` for footprints with non-plated holes. It records definition identity,
revision, provenance (including a forked footprint’s original source record)
and stable primitive IDs separately from project storage.
Its object order is deterministic, with a 1 MiB encoded size limit, up to 512
primitives per symbol or 1,536 primitives per footprint (covering up to 1,024
pads plus artwork, mechanical holes and legacy graphics), and 64 undo snapshots
per document. The footprint schema itself allows 1,024 pads, 256
parameterized artwork objects, 64 legacy technical rectangles and 64 legacy
silkscreen paths.
Object/content allocation counters survive
Undo and save/reopen, so deleted IDs and observed content revisions are not reused.
Undo can therefore leave allocation bookkeeping unsaved even when geometry returns
to its saved state. Runtime document IDs and edit revisions are not saved. Save Library Copy publishes a new `.pcitem.json` file atomically on
desktop and refuses existing destinations, including source files. An exact
captured snapshot becomes saved only after publication and sync succeed; newer
edits remain unsaved. Open validates an editor file into a new document and
refuses replacing an already open identity. A cancelled picker or failed write
leaves documents intact. Browser downloads and native-definition exports do not
confirm a durable editor save, so they retain the unsaved state.

Export Native Definition writes the existing standalone symbol/footprint schema
to a new file. Keep the editor item as the provenance-bearing authoring master;
the existing standalone symbol format does not carry its license or stable
editor primitive IDs. Unknown symbol provenance requires recording its license
before saving or exporting. Native source registration/search is available through
the **Library** workspace (also reached through **Libraries → Sources**), as described in the [source guide](native-library-sources.md).
Deliberate application of an authored definition to a project remains follow-up
work; desktop **Library → Publish…** adds validated native revisions to a writable
registered folder and refreshes its catalog. **Save Library Copy** retains its
separate draft-file meaning. See [native publication](native-library-sources.md#edit-and-publish-native-items)
for identity, older-base warnings and write/recovery behavior.

Live MCP switches the five available workspace types through `app.set_workspace`
and discovers `app.library_new`, `app.library_fork_selected`,
`app.library_edit`, `app.library_view`, `app.library_focus`, `app.library_open`, `app.library_save`
and `app.library_close`. Forking addresses an explicit source view ID.
`inspect_editor.workspace` identifies the visible workspace, while
`inspect_editor.library_editors` reports document IDs,
primitive records, dirty fields and each document's revision. `app.library_view`
accepts an explicit pane ID and controls its ribbon, Browser, Properties and
navigation. `app.set_workspace` accepts source pane and placement; ordinary
`view.split`, `view.open_beside`, `view.detach`, `view.dock`, `view.open_window` and
`view.close` use the same layout path for definition and project views. Edits require
that library revision and use the same engine authoring transactions as the GUI.
Headless project sessions do not yet host these editor documents. Bulk tables, arbitrary pad shapes, mask/paste, 3D, browser/remote publishing
and reviewed updates to placed components remain follow-up scope under #78/#79.

## Review pin renumbering

In a Symbol Editor, **Library → Remap pins…** opens a private number table.
Change both numbers for a swap, then choose **Preview current nets**. The review
shows before/after numbers, the unchanged draft name, and separate pinned-before-number
name, current net and native package-pad evidence. The pinned evidence is looked up
by the draft's before number on the explicitly associated published copy, not by
primitive lineage across earlier draft edits. For example, after swapping draft IN
and OUT, draft IN at number 2 is shown separately from pinned number 2's OUT name
and net. A known placed terminal with no net says **Unconnected**; actual net names
are prefixed with **Net:** (for example, **Net: Unconnected**). An unassociated
revision or missing pinned terminal stays **Unknown**. With no open project or no matching
placed copy, pinned names, nets and pads are unavailable, not known unconnected evidence.
Names, electrical types, geometry and package mappings are not
edited by this operation. Shared appearances in other units and an explicit
power-role terminal follow the same logical correspondence.

The preview associates placed copies only with the same library/item identity
and the editor's published base revision. **Edit copy** starts another identity;
its resemblance to a placed symbol does not establish an association. Different
pinned revisions, missing terminals and unplaced terminals are reported rather
than matched by position or display text. The nets and pads describe the current
project, not a proposed reconnection after adopting the edited definition.

**Apply remap** commits an effective change in one library undo step. An explicit
unchanged map submitted through the command is a no-op: applying it consumes the
review without changing the authoring revision, history, encoded bytes or unrelated
canvas tool/selection/error state. The
form submits only changed rows. Apply does not publish, edit placed instances or
change project history. **Cancel** consumes the addressed review without changing
either document. Editing
a number withdraws its review. Applying refuses a changed authoring revision,
unapplied property fields, a changed/reopened project or a review from another
document. An edit followed by Undo still requires a new preview. All existing
symbol bounds and saved formats remain unchanged. Preparation is limited to
4096 affected rows (matching part instances × requested logical pins), checked
before connectivity/report allocation. Larger matching projects must review fewer
pins per operation; a swap must still include every colliding terminal together.
If it cannot fit, no preview or partial edit is produced. This is a transient
review-operation budget, not a lower saved-project limit. Every route also limits the
encoded report to 4 MiB, counting borrowed evidence before copying each row. Oversize
reports refuse without truncation. Before borrowing or building whole-project
connectivity, a 1,048,576-unit model charges pin appearances, anchors, direct marker
fanout, resolved route segment bounds and endpoint lookups, occurrence searches,
name collision candidates, and current and retired numbering records. Unrelated
sheets count too. A separate 64 MiB model charges escaped occurrence-path and name
input bytes, including a path allowance for each potential endpoint. The estimates
can refuse conservatively; warm and cold requests follow the same rules. These are
not bounds on ordered-map comparisons, total instructions, time or peak heap. Cold
graph preparation still visits the admitted project and can retain multiple copies
of path text. The report borrows the shared graph and display names, not a second
connectivity store.
The report can later gain bounded paging without changing explicit correspondence or apply authority.

Headless `library.preview_pin_remap` accepts `item` (an authoring envelope), `pins`
and optional `base_revision` (the published revision for project association).
Each of 1–128 `pins` entries is an object (not a positional array) containing the owning `unit`, positive `object` primitive
ID as a canonical decimal string and destination `number`. Numbers use the existing 1–8 ASCII letters/digits,
`+`, `-`, `_` alphabet. Primitive IDs belong to the authoring document and are
reconstructed when opening a published revision, not preserved across publication.
The response contains a validated candidate `item`, `changes`, and `instances`
with current nets/pads and association status; it writes nothing, including when
no project is open. No paths or filesystem access are accepted.

Live `app.library_pin_remap` requires `document`, `expected_revision` and `phase`.
`preview` requires `pins` and forbids `token`; `apply` and `cancel` require `token`
and forbid replacement `pins`. Obtain the captured token from
`inspect_editor.library_editors.pin_remap`. A token binds the exact review to both
the authoring revision, current project snapshot/runtime revision, and the
published symbol's relevant catalog evidence. Matching project pins can still
supply offline evidence, but a different published body or a quarantined
revision cannot. If that evidence changes, Apply requires a fresh preview;
Cancel still works. GUI and live commands use these same preview/apply/cancel handlers. A live preview refuses
while any pin remap form is open, preserving typed numbers and the captured
review, even for the same document. Apply or cancel that form before starting
another live preview; the form's own **Preview current nets** can refresh its
review. A failed refresh retains the form's previous token and unchanged captured
map, so Cancel or dismissal of that owning form still consumes it; editing any number
withdraws the old review before another preview can authorize Apply. Project-close or
new/open decisions that displace the form retire its token, even if the close prompt is
cancelled. Addressed apply/cancel cannot dismiss a different unreviewed form. Native preview also checks
the complete prospective editor-inspection success reply against the live client's
existing 16 MiB JSON-frame limit before publishing the new review/token. A reply
that cannot fit refuses without replacing the prior capture or opening a new
modal; required review data is not truncated. The check counts encoded bytes,
including the success wrapper but excluding the transport newline. It does not
replace the common report/preparation admission limits or measure peak heap. The
prospective value uses the normal inspection emitter with the proposed review and modal
state; the remap command preserves pane bindings rather than normalizing them afterward. Later unrelated
editor-state changes can affect inspection size; the visible form can still cancel
its capture. With normal inspection, live execution returns editor inspection;
`inspect:false` returns only an acknowledgement/project revision, not the token
or report. Headless `library.preview_pin_remap` instead preserves its focused
query report even with `inspect:false`.

Preparation is synchronous. A started operation can outlast its live request deadline;
a lost or timed-out reply is not proof of rollback. Inspect the current document and
`library_editors.pin_remap` before retrying. If a capture committed, use its visible
Cancel or the addressed cancel phase; if Apply committed, inspect its revision/history
rather than repeating it. Errors are returned to a receiving peer, not guaranteed delivered
after disconnection. Inspection remains available; separate `ui.*` automation has its own
input lifecycle rather than the document/presentation modal gate.

The new `remap_pins` action is refused by generic `app.library_edit`; existing primitive
replacement actions keep their prior behavior. Project adoption, terminal/gate
swapping on placed instances and native package-map editing are separate work.

These two commands use canonical decimal **strings**, not JSON numbers, for their
remap identities and revisions. A string contains only ASCII digits, with no sign,
whitespace, decimal point, exponent or leading zero except the value `"0"` itself.
The scalar domains are:

| Input | Inclusive range | Presence |
| --- | --- | --- |
| `pins[].object` | `"1"`–`"18446744073709551615"` (u64) | Required in each correspondence |
| Headless `base_revision` | `"1"`–`"4294967295"` (u32) | Optional; omission means unassociated, null is refused |
| Live `document`, `expected_revision` | `"0"`–`"18446744073709551615"` (u64) | Required in every phase |
| Live `token` | `"1"`–`"18446744073709551615"` (u64) | Required for apply/cancel only |

The corresponding report fields also use strings: `changes[].object`,
`instances[].pins[].object`, `instances[].pinned_revision`, and the
association value `source_revision` (null if unassociated). The live review's
`document`, `expected_revision`, `project_revision` and `token` are strings too.
The embedded native authoring `item` and native part-definition reference retain
their existing codecs; other editor commands and general library inspection IDs
are unchanged. Convert an exactly retained inspection ID to the new string form,
never a value already rounded by a numeric client. A well-spelled scalar can still
name an unknown primitive or document, or fail to address the captured review.
Apply also requires the current library revision and project snapshot. Cancel can
retire an old capture after either changes, provided the request still matches the
captured document, revision, token and owning form. Unknown request/correspondence fields
are refused. The structural array schema also forbids exact repeated rows.
Distinct rows reusing `(unit, object)` or a destination number can still be
structurally valid: both checked readers reject them through shared row validation.
The headless schema treats `item` as an opaque object; its native envelope is
validated by the reader. Resolved shared-terminal duplicates, unknown draft
primitives and collisions with unchanged terminals need authored transaction validation.
Pinned terminal existence is report lookup: an absent counterpart yields `missing_terminal`,
not a transaction refusal. These are not claims made by the scalar or array schema.

### Identity and external update boundary

The durable library identity is the native `(library, id)` and its immutable
published revision. The preview uses that identity and the explicit base revision
to associate current placed copies; an authoring primitive ID is only local to
that open draft, and live document IDs, revisions and review tokens are runtime
authority, not saved exchange addresses. Reopening or changing the relevant
project/draft requires fresh inspection and review before applying, rather than
replaying a token.

A future SolidCraft/Fieldcraft simulation or parametric adapter can resolve the
native library/part identities and supply explicit correspondences through this
same revision-checked review path. It must re-resolve draft-local primitive IDs,
preserve native pin-to-pad ownership and obtain fresh review authority after any
relevant edit. This adds no second mapping store or independent update authority.
A SolidCraft-origin board-outline edit belongs to the existing board identity,
project revision checks and shared geometry/native-storage path, not this symbol
command; an effective project edit invalidates a captured remap review. Exchange
transport, cross-application identity resolution and update/conflict policy remain
future design work; no simulation or outline-exchange integration is implemented
or tested by this operation.

## Additional manual review and local verification

The November 2026 usability expansion consulted the same publisher manuals above,
with attention to the KiCad symbol-pin and footprint construction sections and
Altium pin properties, duplication and footprint origin/grid workflows. These
informed the feature list; the geometry, forms, commands and glyphs are original
PCBcraft work using existing Craft chrome and the qualified Technical stroke font.
No vendor component definitions or implementation sources were used.

Engine regressions build an eight-pin symbol and an eight-pad plated footprint,
round-trip their editable/native forms, preserve source definitions, swap numbers
atomically and reject colliding, oversized, stale and partially allocated arrays.
GUI regressions exercise ribbon arrays, compact/expanded pin fields, cancelled
duplication, live group selection and stale array refusal. Native lifecycle checks
remain separate from these widget-driven tests.

A separate shared-state failure review followed the changed command callers,
per-pane draft binding and existing file-worker completion path. Tests exercise
partial array failure without identity allocation, stale batches from another
pane, cancelled placement, save completion with newer edits and disconnected
file workers. Reading the callers confirmed that transforms/clipboard/arrays
perform no new blocking I/O, validate temporary candidates before publication
and leave stale field drafts with their original revision. Native tests exercise
close cancellation and continued definition editing after the original host closes.

Multi-unit symbols use the unit selector above symbol properties. **Add unit** creates
an empty drawing under a new stable unit ID; selecting a unit retains the other
drawings and their primitive IDs. Shared terminal names and electrical types
remain part-wide. Existing single-unit documents retain version 1 of the editable
envelope. See the
[symbol contract](symbol-schema.md#multiple-schematic-units) for placement and
physical mapping behavior.
