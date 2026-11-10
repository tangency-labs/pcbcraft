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

Paste starts a cursor preview. **R** rotates the entire fragment about its
anchor, a canvas click places it, and **Esc** or a right-click cancels it. The
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

**Ctrl+Shift+V** or Capture → Modify → **Paste Options…** offers original
reference preservation, a net-name suffix, and placement at the original model
coordinates. Keeping references rejects the entire paste if any reference
already exists. A suffix changes copied local/global label names and power
symbol values, allowing repeated circuits to keep separate named nets. Without
a suffix, matching global names can join the destination's existing nets.
Original-position paste still creates new identities and one undo step.

Package assignments are copied as intent, without creating PCB placements.
Hierarchical sheet blocks and hierarchical interface labels are not included in
this flat schematic fragment contract. Plain text, images and other EDA
applications' native clipboard formats are not accepted as design fragments in
this slice; unsupported input is reported without changing the project. The
separate import/source boundary remains in force.

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

An opt-in Linux native regression exchanges clipboard text between two real
processes. Run it on a separate Wayland display because it temporarily owns that
display's clipboard:

```sh
cargo test --locked -p pcbcraft-app \
  clipboard::native_tests::native_clipboard_between_processes -- --ignored --exact --nocapture
```

## PCB command contract

The engine and MCP also support `pcb.copy`, `pcb.cut`, `pcb.paste` and
`pcb.delete_items`. Canvas controls are added separately. Copy/Cut/Delete take
an explicit `board`, `expected_revision`, and `items`, whose optional arrays are
`footprints` (canonical placement IDs), `drawings`, `tracks` and `vias` (UUIDs).
Copy/Cut return portable `clipboard` text even with `inspect:false`; MCP does not
operate the system clipboard. Mixed cuts and deletes create one undo step.
Removing physical objects retains their schematic counterparts.

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
