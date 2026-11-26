# Projects and navigation capabilities

## Projects

### Available

- Compact Home cards start a New Schematic or New PCB in the chosen workspace.
- New Project opens Untitled at once and a desktop first save names it after its folder/archive.
- rename, open, save/save-as folders or `.pc`.
- Desktop `pcbcraft <path>` opens a project folder, `project.json` or `.pc` at startup and a failed
  open keeps Home usable.
- Desktop Home remembers 12 recent projects with compact rendered schematic cards with previews
  sized to the card width and reopen/remove/clear actions across restarts.
- Save/Don't Save/Cancel before closing, New or Open replaces unsaved work.
- Desktop [recovery recording and snapshot
  requests](../formats/native-project-storage.md#recovery-snapshots-and-timed-saves) in the recovery store, off
  by default or at 1, 5 or 10 minute intervals, with bounded retention and documented I/O failure
  limits.
- Opt-in timed Save for a dirty project with a destination (off by default).
- Shared undo/redo.
- Supported split document schemas remain readable under the [current project
  layout](../formats/native-project-storage.md#compatibility-and-fixtures).
- Explicit CLI conversion copies the previous split layout to a new folder or package without
  changing the source.
- [stable single-use sheet occurrences](../formats/sheet-occurrence-storage.md) survive hierarchy edits,
  first-board creation, undo and save/reopen.

### Remaining scope

- Local recovery of unsaved work: [#69](https://github.com/tangency-labs/pcbcraft/issues/69).

## Navigation

### Available

- Dockable Layers, Properties and Selection filters with header dragging, edge stacking, per-view
  local arrangements and live MCP positioning, and a Floating/Compact panel style that joins the
  ribbon, Browser and docked cards to the window edge in Compact.
- Per-view selection filters for symbols (including power/ground), wires, labels/sheets, footprints
  and board graphics, with All items/Reset, collapse and disabled unsupported categories.
- Zoom (one shared manual range up to 1024× across all four editors), middle-button drag pan or drag
  up/down zoom, optional right-button drag pan, Fit, floating Browser/Net Inspector cards, Parts
  Catalog with cross-library/MPN search, a dense expanded list, independent symbol/package zoom
  previews, remembered window/divider sizing and a compact dock/float presentation, once/repeat part
  and power placement, theme/scale/navigation preferences.
- Per-view schematic and PCB grid spacing with separate remembered preferences and presets, and
  shared opacity.
- PCB grid hidden by default, with Off/Dots/Lines in Preferences.
- Mil/mm display units with a pointer coordinate readout and unit-aware spacing entry.

### Remaining scope

- Remaining footprint/wire geometry property fields with the inspector
  [#50](https://github.com/tangency-labs/pcbcraft/issues/50) under
  [#75](https://github.com/tangency-labs/pcbcraft/issues/75).

## Cross-probing and views

### Available

- Saved layouts under File → Layouts, per-layout activation and management, naming dialogs,
  confirmation before deletion and user-local save/reopen restoration, one Browser tree for
  Sheets/Boards and their symbols, nets and footprints, navigation and selection within the focused
  pane.
- Ordinary Browser clicks retain the current workspace, with explicit context actions for
  cross-workspace opening.
- Shared tree-row metrics and disclosure controls in Symbol/Footprint editors, and a closable,
  draggable header per split or detached pane, with no document tab strip in the main single view.
- Drag-out creates a window in the same running instance, with same-window pane reordering that
  retains dividers/cameras/cached documents, source shading, animated destination previews, a
  visible Dock button on detached headers and window drop targets anchored beside the grabbed header
  on every platform.
- Drag-in docks beside a destination pane, with direct cross-window drops when positions are
  available.
- Schematic and PCB inspection targets in per-window split layouts.
- Multiple native editor windows with the integrated title bar, window dragging, maximize and
  restore, and edge and corner resizing, including while project dialogs block editing.
- Splitting inside them, tab placement/close actions, independent cameras and selections, shared
  edits/undo/save, bidirectional component counterpart highlights and [optional directed following
  across schematic sheets](../design/board-workspace-plan.md#correspondence-and-cross-probing) for PCB
  footprint selections. The workspace picker opens a target here, beside the current view or
  directly in another window. Any window can close while others remain.
- a survivor takes over project controls. Last-window Close or explicit Quit prompts once. Live MCP
  drives the same view, workspace, arrangement, window and panel actions by ID, including footprint
  selection.
- Headless/live MCP can look up the supported pin/pad mapping.

### Remaining scope

- Editable-board acceptance [#54](https://github.com/tangency-labs/pcbcraft/issues/54),
  missing-mapping feedback [#65](https://github.com/tangency-labs/pcbcraft/issues/65), multiboard
  storage/assignment [#122](https://github.com/tangency-labs/pcbcraft/issues/122), portable
  arrangement export, full multi-monitor restoration, outside-client drag previews, provider view
  payloads.

## Interchange

### Available

- [KiCad schematic import](../schematic/kicad-import.md) of one self-contained KiCad 6–10
  `.kicad_sch` through `project.import_kicad_schematic` in headless MCP and
  `pcbcraft-cli import-kicad`: symbols, units, pins, references, values, fields, power symbols,
  wires, labels and no-connects.
- The import is exact or refused with reasons (including KiCad 10 jumper pin groups), checked
  against the nets the file describes.

### Remaining scope

- Sub-sheets, buses, board import and a desktop menu entry remain in
  [#60](https://github.com/tangency-labs/pcbcraft/issues/60).
- IPC-2581/ODB++/EDIF in [#119](https://github.com/tangency-labs/pcbcraft/issues/119).

## Sharing

### Available

- Native folder/archive storage.
- a routed ATtiny402/TMP102 sensor-node fixture and a secondary transistor flasher, with
  command-built generators and offline dependencies.
- Source builds for desktop/web.

### Remaining scope

- Polished demo and preview artifacts [#91](https://github.com/tangency-labs/pcbcraft/issues/91).
- User/contributor onboarding [#90](https://github.com/tangency-labs/pcbcraft/issues/90).

## Implementation notes

Desktop saves write to the chosen folder/archive. App version 0.1.0 uses
`project.json` entry points and container-v3 `.pc` ZIP packages, intentionally
breaking older project layouts (see [release notes](../../CHANGELOG.md)).
Browser uploads use current archives; downloads produce `.pc` and do not clear the dirty state
because the browser cannot confirm a durable write, so the page's leave warning
stays on until the project is replaced. The desktop Assistant and
local MCP listener are not available in the browser build.

The board walkthrough, polished demo and fresh-user onboarding acceptance in [#90](https://github.com/tangency-labs/pcbcraft/issues/90)
remain pending their dependencies. Documentation command checks and automated UI
tests do not substitute for a new person's end-to-end acceptance run.

Schematic, PCB layout, symbol and footprint editors share manual zoom up to
102400%, with pointer-anchored wheel/pinch navigation and content-specific Fit.
