# Schematic editing capabilities

## Schematic editing

### Available

- Up to 64 flat or hierarchical sheets with create/rename/delete and A4–A0 landscape/portrait paper
  (sheet Properties, Browser **Paper** menu or `sheet.set_page`), drawn on a uniform theme-following
  canvas by default with a frame (sheet Properties, Browser **Frame** menu or `sheet.set_template`):
  the Default border, zone markers and parameter-filled title block, or a plain border.
- Appearance preferences offer optional paper shading and per-theme sheet/canvas colours, while the
  canvas around it stays editable and keeps its grid unless **Show grid outside paper** is off.
- Fit and Home cards frame the paper, and older sheets get paper around their drawing on open.
- Project and sheet parameters (`@project.Revision`, `@sheet.Title`) edited in Properties with
  nothing selected or through `parameter.set`/`parameter.list`/`parameter.resolve`, with sheet
  values defaulting to the project's, and read-only built-ins (`@instance.Name`, `@instance.Page`,
  `@instance.Pages`), S placement, body/title movement and four-corner resizing with attached-wire
  previews, double-click entry/Alt+Backspace return, synchronized child labels/parent pins and
  explicit inter-sheet connectivity, explicit command targets, independent pane navigation and
  shared undo/save/reopen.
- Compact/expanded selection properties with atomic reference/value/orientation and unit-aware X/Y
  edits for selected symbols, attached-wire movement and read-only terminal/library details.
- place, rotate, mirror horizontally or vertically about the symbol center with attached wires, edit
  standard/custom component fields and population/exclusion attributes, move/delete symbols.
- Independently drag reference/value/custom text with per-unit visibility, name display, physical
  size/alignment/rotation, offsets and automatic-layout reset.
- Local/global net labels with automatic orientation away from snapped pins and upright default
  global text, plus global power/ground anchors, named inspection, shared GUI/MCP commands and
  undo/save/reopen.
- M/Disconnect and Move picks a selected or hovered symbol and preserves old wiring for replacement.
- M moves a selected whole wire or mixed group while disconnecting contacts to unselected objects
  and preserving internal connections/no-connect markers.
- G and group dragging retain external connections.
- Drawing-bound overlap warnings with compact amber counts, reference tooltips and a right-click
  chooser for hidden symbols.
- Directional boxes, modifier selection, Ctrl/Cmd+A Select All on the current sheet respecting
  Selection filters, and group move/delete for components, wires and named-net anchors.
- System clipboard copy/cut/paste and duplicate across panes/windows/instances, pinned native
  definitions and package intent, rotation, reference/net-name options and one-step undo.
- Explicit plain-text conversion to local/global net-label groups.
- Complete hierarchical subtree/interface clipboard, independent duplication or explicitly selected
  cut relocation, stable cached previews and pan/zoom.

### Remaining scope

- Repeated child instances remain deferred.
- Remaining group G-drag: [#44](https://github.com/tangency-labs/pcbcraft/issues/44).
- Template editing and logos: [#46](https://github.com/tangency-labs/pcbcraft/issues/46).

## Wiring

### Available

- Orthogonal routes with a held-Ctrl grid override.
- Direct starts at unconnected pins/free ends, W at the pointer and cached magnetic targets.
- Segment drag/G and Delete.
- Corner/free-end/shared-junction handles.
- Pin/endpoint/segment attachment.
- Exact part/wire drop contacts and two-pin insertion into straight runs.
- Overlap/return healing and new-route loop replacement, enabled by default with a saved Auto-heal
  wires toggle in Preferences → Schematic and the Modify menu.
- Slice, Heal and Delete in the ribbon.
- Contextual Break releases one or both attached ends, with free movement of detached straight
  pieces, guarded departure and reconnection on return.
- Shared undo and headless MCP commands.

### Remaining scope

- Obstacle-aware routing [#30](https://github.com/tangency-labs/pcbcraft/issues/30).
- Ambiguous contacts and edits beyond healing's complexity limits are refused.
- Loops with intermediate pins or branches are preserved.

## Designator locking

### Available

- Part-instance **Lock designator** in compact/expanded Properties.
- Shared GUI/live/headless command with non-mutating document-change preview, explicit boolean
  inspection, lock-preserving clipboard and folder/archive reopen, one-step undo.
- Manual reference edits remain available.

### Remaining scope

- There is no separate unit-letter lock.

## Package completion

### Available

- [Explicit sheet or project scope](../libraries/symbol-schema.md#complete-loose-packages), dry run, exact pin
  correspondence, locked-owner skipping and one undo step through headless/live MCP.

### Remaining scope

- Manual unit swapping is not included.

## Schematic annotation

### Available

- [`annotate.preview` and `annotate.apply`](../schematic/annotation.md) in the engine and live/headless MCP, with
  sheet or project scope, lock-aware gap filling for placeholders, duplicate diagnostics, stale-token refusal and
  one-step undo.

### Remaining scope

- Duplicate repair, full renumbering and a graphical review control remain in
  [#45](https://github.com/tangency-labs/pcbcraft/issues/45).

## Implementation notes

The engine can produce a [vector schematic PDF](../manufacturing/schematic-pdf.md) of one sheet
or all project sheets in hierarchy order, with searchable text at each sheet's
saved paper size and project-export bookmarks. There is no export action in
the UI or MCP yet.

[Intentional no-connect markers](../schematic/no-connect-markers.md) attach to logical part
terminals and open wire ends through Q, Capture and context actions. They follow
placed units and endpoints, support ordinary click/marquee selection and
Delete/Backspace without deleting their owners. A wire-end X can stretch its
attached wire, and Q/Capture clears previous selection/highlights. Marks survive duplication/clipboard and native save/reopen, and have a focused,
read-only check with navigable missing, incompatible and connected-target
findings, including unmarked dangling wire ends. Marker edits preserve the actual
netlist and physical DRC findings. Marked targets block new wires and endpoint
attachments through both GUI and MCP until the mark is removed; automatic wire
hover/start is suppressed throughout their magnetic target area. Full type-conflict/power-driver ERC and incremental scheduling remain
with #47/#245/#243.

[Shared readable net names](../schematic/net-labels.md#generated-names-and-copper-labels)
cover schematic browsing, Rules, PCB inspection and fitted pad/via/track overlays.
Accepted boards retain their reviewed names until Update PCB; the overlay toggle
is view-local. Repeated-instance and multiboard naming remains within the current
single-use/single-board scope.

Symbol Grab/Move previews support R rotation, with movement and orientation committed
together as one undo step and discarded together on cancellation.
Power/ground G-drag retains touching wire contacts regardless of placement order,
with connected previews and shared undo; fixed pins stay in place. New placements
are ordinary native library instances; the old power-marker styles are unsupported.
Schematic part/power placement repeats by default for new settings, including catalog
drag/drop with custom power-net names, and completed wires leave the tool ready for another route. Canvas context menus provide the
supported drawing, selection and view actions, with spacing choices in a Grid submenu.
The [editor guide](../editor-guide.md) covers one-shot placement and cancellation.

The schematic ribbon's **Ground** action starts a GND placement directly. **P**
opens the built-in Power collection, which includes both arrow-shaped **VCC** and
flat-bar **VCC (bar)** choices with the same editable default net name.
