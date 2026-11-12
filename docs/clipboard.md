# Copy and paste

The schematic canvas uses the system text clipboard, so a selection can move
between split panes, detached windows, projects and separate running instances.
Copies carry their pinned native symbol definitions and package assignments;
the receiving project does not need the source library or source project open.
The clipboard contains only the selected design fragment, with no source-folder
paths or unrelated project objects.

Select symbols, whole wire routes or net labels, then use **Ctrl+C**, **Ctrl+X**
or **Ctrl+V** (**⌘** on macOS). These actions also appear in the canvas context menu and Capture → Modify.
**Ctrl+D** duplicates the selection through the same placement path without
changing the system clipboard. In text fields, these keys retain ordinary text
editing behavior.

Paste starts a cursor preview. **R** rotates a flat fragment about its anchor;
hierarchical objects retain their model-supported orientation. A canvas click
places it, and **Esc** or a right-click cancels it. Pan/zoom remain available. The
active view owns the preview, including its camera and explicit sheet target;
switching focus, replacing the project or making another edit cancels stale
placement authority. A completed paste is one shared undo/redo step. Copy and
cancel leave the electrical revision, saved bytes and undo history unchanged.

Copied objects receive new component, symbol, wire, junction and label IDs.
References are renumbered by default; values, component metadata, field
appearance and exact pinned definitions are preserved. Explicit wire attachments
inside the copied group stay attached to the copies. Endpoints whose owners are
not selected become free endpoints at the same coordinates, while shared
endpoints remain shared. Wire copies include the complete selected route.
Pasting does not auto-heal copied geometry into existing wires. Net labels and
native power-symbol names retain their ordinary named-net/contact semantics.

Cut removes the selected objects and leaves unselected incident wire routes
behind with dangling endpoints. It does not delete an unselected wire just
because a selected symbol or label was attached to it. As with other component
removals, accepted board footprints may remain pending a reviewed PCB update.
The host publishes clipboard text after a successful cut; the app cannot
acknowledge an operating-system clipboard write failure, so Undo remains the
recovery path if a platform clipboard service fails.

## Paste Options

**Ctrl+Shift+V** or Capture → Clipboard → **Paste Options…** offers original
reference preservation, a net-name suffix, and placement at the original model
coordinates. Keeping references rejects the entire paste if any reference
already exists. A suffix changes copied local/global label names and power
symbol values, allowing repeated circuits to keep separate named nets. Without
a suffix, matching global names can join the destination's existing nets.
Original-position paste still creates new identities and one undo step.

Package assignments are copied as intent, without creating PCB placements.
Hierarchical bodies and interface labels use the extended contract below.
Ordinary text can be converted explicitly as described below. Images, SVG and
other EDA applications' native clipboard formats are not accepted as native
design fragments; unsupported input does not change the project. The separate
import/source boundary remains in force.

Browser keyboard copy/paste uses browser clipboard events. Browser menu paste
arms the destination, then asks for **Ctrl+V** (or **⌘V**) because the browser
host does not implement the desktop RequestPaste operation. Native menu paste
requests the operating-system clipboard directly. Empty or unsupported text
reads clear the pending request. Paste Options stays in its initiating window,
and delayed canvas reads cannot insert native payloads into a newly opened text
field. Clipboard formats remain versioned independently of saved projects.

## Portable contract and commands

`pcbcraft-clipboard/1` is a strict JSON envelope with `kind: "schematic"`, an
`anchor` point in integer nanometers, and `documents`, a map from native relative
document paths to UTF-8 text. The documents describe one flat, nonempty,
schematic-only project fragment. Native document decoders validate all records,
paths, library revisions, geometry and cross-references. Unsupported versions,
editor kinds, conflicting pinned definitions, resource limits and invalid
placements reject the entire operation. The envelope is bounded to 8 MiB before
JSON parsing; native project budgets still apply after insertion. Clipboard text
never grants authority to write a file or execute a command.

GUI and MCP use `schematic.copy`, `schematic.cut` and `schematic.paste`.
Copy/Cut take `items` and `expected_revision`; `Session::execute` returns
`clipboard` text and the resulting revision, including when MCP suppresses
project inspection with `inspect:false`. Paste takes `text`, `x_nm`, `y_nm`
and `expected_revision`, plus optional `rotation` (0/90/180/270), `references`
(`renumber` or `preserve`) and `net_name_suffix`. All three take an explicit
`sheet` UUID, required when several sheets exist. These commands operate on
portable text; MCP does not read or write the desktop's system clipboard. The
selected MCP transport's request/reply budgets also apply to encoded payloads.

## Hierarchical clipboard

Select a sheet body to copy/cut/duplicate it with its complete child subtree,
including nested sheets, symbols, explicit interface connections and pinned
package intent. Interface labels can be copied or duplicated while hovered on
the canvas or from their context menu; floating panels exclude hover targets.
Copy/Cut on a selected sheet body also work from the
normal canvas/ribbon menus. Root wire routes enter the payload only when selected;
unselected parent routes stay outside the copied tree.

Duplication creates independent child document UUIDs and new component, symbol,
wire, junction, interface and block IDs. Child coordinates remain local to each
new sheet; cursor placement moves only the root geometry. Free source sheet names
are retained, while collisions receive bounded copy names. References renumber
by default, and a suffix can distinguish copied labels, interfaces and native
power values. This creates independent circuit content, not repeated instances
of a shared definition. Existing single-parent/cycle rules still apply.

Cut detaches sheet bodies while retaining their child documents/content. It
preserves unselected parent routes as dangling endpoints at their old coordinates.
Cutting an interface similarly preserves unselected routes on both its child and
parent sides. An eligible same-project Cut opens Paste Options with independent
duplication selected. Explicitly enable **Reuse original unparented sheets** to
reparent the current child content without duplicating components or renaming
its nets. Clipboard metadata never enables this choice automatically. Occupied,
missing, foreign or cyclic child ownership is refused without stealing a parent
or publishing part of the operation. Cancel leaves only the preceding Cut;
ordinary Undo recovers it.

`pcbcraft-clipboard/2`, `kind: "schematic"`, extends the native document map to
one selected root and its complete reachable child tree, addressed by an explicit
`root_sheet` UUID independently of native display order, with optional `cut_sheets`
metadata. Orphan payload sheets/components, board owners, invalid
records and oversized input are rejected. Flat schematic and PCB exports retain
version 1, so existing flat interchange stays compatible; older clients reject
version 2. Saved project formats are unchanged. Clipboard metadata suggests
relocation but never bypasses live ownership, cycle, revision or geometry checks.

Copy/Cut additionally accept optional `objects`, a unique array of root
`sheet-symbol-N` and `label-N` IDs. `items` still supplies symbol/wire arrays,
which may both be empty for a hierarchy-only selection. Paste accepts optional
`reuse_sheets: true`; its command default remains independent duplication.
Hierarchical sheet symbols and interfaces retain their orientation. Their
previews cache document creation and remapping once, with stable transient IDs;
final paste always uses the original portable text and validates again.

The opt-in native regressions exchange clipboard text between two real processes
on Linux or Windows. Run them one at a time on an isolated interactive desktop
(a separate Wayland display on Linux); they open two windows and temporarily
replace that desktop's clipboard. The windows must be allowed to receive focus.
These commands also work in PowerShell on Windows:

```sh
cargo test --locked -p pcbcraft-app clipboard::native_tests::native_clipboard_between_processes -- --ignored --exact --nocapture
cargo test --locked -p pcbcraft-app clipboard::native_tests::native_board_clipboard_between_processes -- --ignored --exact --nocapture
cargo test --locked -p pcbcraft-app clipboard::native_tests::native_hierarchy_clipboard_between_processes -- --ignored --exact --nocapture
```

The native module remains unavailable on macOS. Compiling the Windows tests
alone does not verify an actual Windows clipboard exchange.

## Paste from other programs

Ordinary text opens **Paste Text** before any design objects are created. In a
schematic, one line becomes one local or global electrical net label, with an
explicit scope, vertical spacing and optional net-name suffix. These are named
net anchors, not drawing notes: matching names connect within their scope. Empty
rows, multi-column input, invalid names and more than 256 labels reject the whole
conversion. The form text is editable, and its text field retains normal system
copy/paste behavior.

On a PCB, a numeric CSV or tab-separated table becomes one Board Outline
polyline. Choose source **mm** or **mil**, optional closure and Y-axis inversion;
the displayed vertex count and dimensions use the same parser as the command.
**Keep source coordinates** places the validated geometry directly; otherwise
placement uses the native cursor preview with R rotation and cancellation.
An optional header is `x,y`, `x_mm,y_mm` or `x_mil,y_mil` (tabs also work).
Unit-qualified headers must match the chosen units. Cells may quote numeric
values and use decimal points or finite scientific notation; formulas and
locale decimal commas are not evaluated or guessed. Closed outlines normalize
one repeated final vertex. Limits are 64 KiB and 2–256 vertices, with at least
three for closure. A malformed row rejects the entire conversion.

For example, a table copied from a spreadsheet can contain:

```csv
x_mm,y_mm
0,0
25.4,0
25.4,12.7
0,12.7
```

The form stays in its initiating view/window, and a changed project opening or
revision invalidates Apply. Preparation creates no destination objects or undo
entry; successful placement is one native edit. Native PCBcraft payloads with
unsupported versions stay on the strict native parser and cannot become net
names accidentally. Structured EDA payloads and images are outside these text
conversions; they do not invoke an importer.

Headless clients use `schematic.prepare_text_paste` (`text`, `style`,
`spacing_nm`, optional `net_name_suffix`) or `pcb.prepare_coordinate_paste`
(`text`, `units`, `closed`, optional `invert_y`), both with `expected_revision`.
They return portable `clipboard` text even with `inspect:false`, without
requiring an open project or touching the OS clipboard. Submit the reviewed
result to the ordinary native paste command with explicit destination authority.

## PCB canvas

The PCB canvas uses the same system clipboard keys and **Layout → Clipboard**
menu. Its context menu also offers Copy, Cut, Paste, Paste Options, Duplicate and
Delete. A directional marquee selects visible footprints, tracks, through vias
and Board Outline graphics together; Shift adds and Ctrl/Command toggles.
**Ctrl/Command+A** selects all visible supported categories. Hidden layers and
selection filters constrain editing targets; correspondence highlights do not.
Mixed Delete and Cut are one undo step and retain schematic components.

Track/via lock flags survive Copy, Duplicate and Paste on the new object IDs.
Existing source copper remains protected, and Cut/Delete refuse the whole
selection if removing any locked object would be required. Lock-bearing board
fragments use `pcbcraft-clipboard/3`; older readers refuse that extension.

Pasting copied footprints opens **PCB Paste Options**, where each source circuit
has an explicit destination schematic sheet and independent schematic X/Y anchor.
Suggested anchors leave a gap below existing sheet content. Paste duplicates
linked schematic components and their physical packages together, retaining
internal wiring and related native power annotations. References renumber by
default; original references and net-name suffixes are available in the form.
The destination must have no unrelated pending PCB update before duplication.
If the selected components have an internal connection that their copied flat
circuits cannot preserve, such as a missing hierarchical boundary, Copy/Cut
refuses the entire selection with guidance to copy the complete schematic
subtree first. It does not silently export a disconnected circuit. Already
carried global connections remain supported.

Pasting a Cut fragment back onto its original project/board defaults to reusing
compatible, unplaced components. It retains their references and current logical
wiring. If reuse is no longer valid, the form retains the fragment and reports
why; turn off **Reuse original unplaced components** to duplicate instead.
Geometry-only paste needs no schematic destination. Ambiguous layer mappings
open the form rather than guessing; existing setup is retained.

Paste enters a physical cursor preview: **R** rotates the group, click places it,
and **Esc** or right-click cancels. Pan and zoom remain local to the receiving
pane. Focus, opening or revision changes, text entry and competing authored tools
invalidate the preview. The form stays in its initiating native window. Original
board coordinates can instead be applied directly from the form. A failed final
copper or bounds check retains the preview for correction without a partial edit.

## PCB command contract

The engine and MCP also support `pcb.copy`, `pcb.cut`, `pcb.paste` and
`pcb.delete_items`. Copy/Cut/Delete take
an explicit `board`, `expected_revision`, and `items`, whose optional arrays are
`footprints` (canonical placement IDs), `drawings`, `tracks` and `vias` (UUIDs).
Copy/Cut return portable `clipboard` text even with `inspect:false`; MCP does not
operate the system clipboard. Mixed cuts and deletes create one undo step.
Removing physical objects retains their schematic counterparts.

Board fragments with physical graphic strokes use `pcbcraft-clipboard/2` and
preserve each `stroke_nm` and layer role. Fragments without stroked graphics
retain version 1. Version 2 is distinguished by `kind`, independently of the
existing hierarchical schematic payload. Both board versions are read; older
clients reject the new payload rather than dropping its widths.

A `kind: "board"` envelope carries exact placed native packages and pad maps,
physical geometry, used layer identities/roles, and one native schematic
fragment per source sheet. It includes wiring internal to selected components,
connected named-net boundaries and related excluded native power annotations;
it does not flatten sheets or carry unrelated components. Source and destination
projects need no shared library installation. The same 8 MiB bound applies.

Ordinary PCB paste duplicates logical and physical objects together. It takes
`text`, `x_nm`, `y_nm`, `board`, `expected_revision`, and a `schematic_targets`
map keyed by every source sheet UUID. Each value explicitly supplies a destination
`sheet` UUID and `at: {x_nm, y_nm}` anchor. Schematic placement is independent of
board coordinates. `rotation`, `references` and `net_name_suffix` follow the
schematic policies, with rotation applying only to physical geometry. Copied
power annotations stay excluded from the board. A destination with an accepted
PCB baseline must have no unrelated pending update; paste accepts only its own
new component/package additions. Legacy two-pad markers require native package
assignment before duplication into an accepted board.

`reuse_components: true` instead places compatible, currently unplaced original
components on their original project and board, retaining their references and
current schematic wiring. This supports Cut followed by Paste without creating
extra symbols. It refuses occupied, retired or changed-package components,
foreign project/board identities, schematic target remapping and net suffixes.
A clipboard's Cut flag suggests relocation to a host; it does not grant mutation
authority or bypass validation.

Optional `layers` maps source layer UUIDs to destination layer UUIDs. Defaults
use compatible exact identities or a unique purpose/side match; ambiguous inner
layers require a choice. Explicit mappings preserve purpose and may deliberately
change copper side. Names never determine identity. Board setup is not copied,
and through vias require an existing compatible setup. Geometry-only paste
creates no logical counterparts or package acceptance changes.

All pasted physical objects get fresh placement/graphic/copper identities.
Copper shorts are checked using the existing track/pad contact model before
publication, including newly pasted pads touching existing copper. This is the
current routing approximation, not a clearance or fabrication sign-off. A failed
mapping, scope, package, bounds, reference or copper check leaves the destination
and history unchanged. Preview snapshots cache logical work but confer no commit
authority; the command validates the complete candidate again.

The engine/MCP scope and identity rules are specified in the
[hierarchical clipboard contract](hierarchical-clipboard-contract.md).
