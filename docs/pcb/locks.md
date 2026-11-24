# Lock PCB placements and copper

## Placed-footprint locks

Placed footprints retain an optional `locked` flag in native board format 34,
35 when the board also contains arcs, or 37 when it contains straight-via
teardrops. Inspection and typed board views report a boolean, defaulting to
false for older files. Fresh canonical encoding omits
false and preserves true; Native Save can reuse an unchanged board document
verbatim, including explicit false. Without active locks the encoder chooses
the format required by the remaining board data. Reports are optional in all
three formats.

A locked footprint remains selectable and inspectable. The shared move, rotate,
flip, align/distribute and delete commands protect existing locks. A selection
containing a locked footprint is refused as a whole, naming each locked placement,
even if it would serve as a stationary alignment anchor. A single-footprint
transform that explicitly leaves the pose unchanged remains a no-op. Applying a
reviewed schematic-to-board update that would remove a locked footprint also
refuses the whole update and names the locked placement. Removing a component’s
last schematic unit, including through group Delete, Cut, or sheet deletion,
is refused if it would immediately remove a locked footprint. An accepted
placement stays on the board pending review. Refusals make no board
or Undo/Redo changes. These locks do not govern package changes that retain the
placement, reference-text edits or routing obstacles. There is no lock/unlock
command or GUI toggle for footprints yet.

Clipboard Copy, Cut, Duplicate and Paste still refuse active footprint locks. Diagnostic
single-file storage also refuses the flag when true. Canonical clipboard
fragments without active footprint locks keep their previous encoding. See
[clipboard](../schematic/clipboard.md#pcb-canvas) and
[native storage](../formats/native-project-storage.md#placed-footprint-lock-metadata).

## Fixed tracks and vias

Select tracks and vias and set **Lock** in Properties to **Locked** or
**Unlocked**. A mixed selection displays **Mixed**. Locked copper remains
selectable and inspectable, while its size controls are read-only. The shared
`pcb.set_copper_locked` command requires an explicit board UUID, expected revision,
`locked` boolean and at least one track/via UUID. One atomic edit changes only
lock flags; no-op requests leave revision/history unchanged.

An edit fails as a whole if it would change or delete any existing locked track
or via. This includes positions, track ends/layers/widths, via diameters/drills,
segment drags that stretch a locked neighbor, footprint moves/rotations/alignment
that carry locked track ends, and mixed-selection Delete or Cut. An unchanged
locked object does not block independent unlocked edits or genuine no-ops.
Lock flags preserve authored copper records; they do not freeze the board's
setup, schematic net intent or footprint contacts when no copper record changes.
In particular, an existing flip that releases a stationary track end keeps that
track's fixed geometry. Locking/unlocking alone changes no copper connectivity.

Copy, duplicate and paste preserve lock flags while assigning new identities
and the requested new positions to copies. Existing source objects remain
protected. Cut refuses a selection containing locked copper. Native folder and
archive saves preserve copper locks; board format 18 (or a later extension
required by other content) and board-kind clipboard format 3 (or format 4 with
explicit via assignments) carry them. Older readers refuse these extensions.
Canonical boards and fragments without copper locks use the formats required by
their other content. Clearing the last copper lock leaves those requirements in
force. Inspection exposes
an explicit `locked` boolean for every track and via, and typed board snapshots
carry it for later walkaround/shove consumers. No routing solver is added here.
