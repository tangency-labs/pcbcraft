# Pin and pad inspection selection

Pins in schematic views and Pads in PCB views can be selected for inspection
through **Selection filters**. These two filters start off to preserve the
existing click-to-wire and footprint-selection defaults. **All items** and
**Reset** enable them with the other supported categories. With Pins enabled,
clicking a pin endpoint or drawn lead selects that exact placed appearance;
explicit W wiring still uses the existing wire tool. With Pads enabled, a click
selects the exact visible physical land using its copper contour. Hidden copper
cannot be picked; legacy center markers use their displayed pick radius.

A plain click replaces selection, Shift adds, and Ctrl/Command toggles. The
Browser exposes Pins and Pads branches under their owners when the relevant
filter is enabled. Selected pins receive an endpoint ring; selected pads highlight
their actual copper shape. Both show a read-only Properties label. Inspection
alone does not select an owner for moving, rotating,
flipping, editing or deleting; drawing tools remain available. A modifier pick
can coexist with an explicitly selected owner, whose normal edit authority is
unchanged. Ordinary selection replacement or Escape clears inspection.

Inspection belongs to one runtime view and is not saved in project documents or
arrangements. Splitting creates a fresh selection; moving a pane between hosts
retains its own selection. Switching documents/workspaces, project replacement,
an authored revision change (including Undo/Redo), or disabling its filter clears
inspection. Hiding all of a selected pad's copper layers removes that pad from
inspection. Selections never reappear from an offline reopen. Shared symbol units
retain distinct placed terminal addresses; complete cross-view highlighting and
Reveal remain the separate #398 presentation slice.

## Live commands

`view.select_inspection` uses the same handler as canvas and Browser picks. It is
an additive window command; existing `view.select` and `view.select_footprints`
payloads keep their meanings. From `inspect_editor`, use the project `revision`,
the addressed view's `history_root_revision` and its numeric view ID. `targets`
is a list of up to 256 distinct qualified addresses using the
[correspondence source vocabulary](../formats/pin-pad-correspondence.md). Empty targets in
replacement mode clear inspection and ordinary selection. Optional `mode` is
`replace` (default), `add` or `toggle`.

```json
{
  "view": 0,
  "expected_revision": 17,
  "history_root_revision": 1,
  "targets": [{
    "kind": "terminal",
    "project": "<project UUID>",
    "sheet": "<sheet UUID>",
    "symbol": "symbol-2",
    "terminal": "1"
  }]
}
```

A pad target uses `kind: "pad"`, `project`, `board`, `footprint` and `pad`.
Ownership, existence, filter and physical visibility validation completes before
selection changes. Foreign, missing, incompatible, closed-view and obsolete
revision/opening requests are refused atomically. `inspect_editor.views` reports
the qualified addresses in `inspection_selection`, separately from editable
owner selections. No native format, project revision, dirty state or document
Undo/Redo entry changes during inspection or filter updates.
