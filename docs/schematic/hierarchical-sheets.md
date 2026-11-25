# Hierarchical sheets

Place a sheet with **S** or **Sheets → Subsheet**, then click its top-left
corner on the canvas. The form can create a named child or place an existing
unparented sheet. Creation and placement are one undo step. The Browser displays
children beneath their parents, while every document keeps its UUID and filename.
Each child has one parent; repeated instances and cycles are refused.

Double-click inside the sheet symbol to enter its child. **Alt+Backspace** returns
to the parent in the same pane. Browser navigation works in both schematic and
PCB workspaces, and other panes/windows keep their own document and camera.

Inside the child, use **H** or **Sheets → Hierarchical label**, click an anchor position
and enter a name and direction. The label exposes a matching pin on the parent's
sheet symbol automatically. Draw a wire to the pointed label anchor or parent
pin with **W**; an ordinary click also starts a wire at an unconnected anchor.
Parent wires and child wires then belong to one project electrical net. Repeated
names on other sheets do not connect them. The Net Inspector lists the active sheet's endpoints after resolving named connections across the project; complete
cross-sheet groups are available through project inspection. See
[named nets and power symbols](net-labels.md) for local/global names and supplies.

Names are case-sensitive and unique within a child. Label properties (**E** while
hovering, or **Properties…** in its context menu) update the parent pin's name
and direction without changing its identity. Input, bidirectional and passive
pins occupy the left edge; output and tri-state pins occupy the right. Direction
does not yet enforce electrical rules. Up to 64 labels fit on a sheet, with stable
slots so removing one does not move neighboring pins.

The sheet name sits above its outline. Drag either the body or name to move the
sheet, preserving the initial grab offset. Click to select it, then drag any of
its four corner handles to resize; the opposite corner stays fixed and the
minimum size leaves room for its pins. Attached wires follow both movement and
resizing. A gesture commits on release as one undo step; **Esc**, a secondary
click, focus loss or dropping outside the canvas cancels it.

**M** while hovering or **Move** in the context menu also picks up a label or
sheet symbol; click its new position or press **Esc** to cancel. Attached routes
retain their endpoint identities. A sheet symbol's **Rename Sheet…** action renames the
child without changing paths or references.

**Remove Sheet Symbol** detaches the child to the project root and removes parent
wires directly attached to its pins, preserving everything inside the child.
**Delete Hierarchical Label** removes that label, its parent pin and wires
directly attached to either. Deleting the child through the Browser also removes
its placements and associated board footprints; its own children become roots.
Each operation is atomic and undoable. Save/reopen preserves hierarchy and
connectivity in folders and packed projects; see the [storage contract](../formats/native-project-storage.md#hierarchical-sheet-records).

Copy, Cut and Duplicate are also available for sheet bodies and hovered interface
labels. A body copy carries its complete child subtree; duplication makes new
independent documents and components. Cut retains child documents for relocation
and preserves unselected incident routes on both sides of an interface. See the
[clipboard guide](clipboard.md#hierarchical-clipboard) for reuse, naming,
versioned exchange and cancellation. These operations keep the existing
single-parent model and do not introduce repeated sheet instances.

## Preview a split to a new sheet

`schematic.preview_split_sheet` reviews moving selected objects from one sheet
onto a new child sheet without editing anything. Give it the sheet, an `items`
selection in the shape Copy and Move take, and the new sheet's `name`, which
must be free without case. Symbols and net labels decide what moves; wires,
junctions and no-connect markers follow what they attach to and cannot be moved
on their own. The reply lists:

- the selected symbols with their part instances and designators, which a split
  would keep, and parts whose other units stay behind (units of one part may sit
  on different sheets, so this is reported, not refused);
- the wires and junctions that would travel, and the `cut_wires` with one end on
  each side. Wires follow their ends: a wire travels when both ends do, and a
  junction travels only when every wire meeting it leads inside. A wire with an
  end outside is cut even when it is selected, and a selected wire with neither
  end inside is listed in `staying_wires`; selected no-connect markers whose pin
  or wire end stays are listed in `staying_no_connects`;
- the `ports`: each net with members on both sides becomes one hierarchical
  label and sheet pin, named with the net's authored name (the first in
  alphabetical order when it has several), without the sheet path that may be
  shown when another sheet uses the same name. An unnamed net uses its generated
  `Net-NNNN` name and is marked `generated`. The proposed direction follows the
  pin types, using a part's selected alternate pin function where it has one,
  as the netlist does: driven only from inside is output (tri-state when every inside
  driver is tri-state), driven only from outside is input, driven from both
  sides or carrying a bidirectional pin is bidirectional, otherwise passive;
- the `global_nets` that need no port because the same global label or power
  symbol name sits on both sides. If a wire is cut or a net member stays on the
  parent sheet, a matching anchor must stay on the parent to reconnect that
  branch; a match on a different sheet alone is not enough. A power symbol left
  outside the selection does not carry its name inside, so that net is listed as
  a port.

More than 64 ports, two boundary nets with the same name, an empty or unknown
selection, a selection with no symbol or net label, a taken or blank name, a full project and a stale `expected_revision`
are refused. The revision, undo history and project files do not change, and
read-only sessions may run it.

## Move a selection to a new sheet

In the schematic, select the symbols and net labels to move (wires follow their
ends), then choose **Move to New Sheet…** from the canvas context menu or the **Sheets** ribbon
group's menu. The form suggests a free sheet name and shows the preview for the
name in the field: how many symbols, wires, junctions and net labels move, the
ports with their names and directions (generated names are marked), the nets
that stay connected by a global or power name, and any part whose units end up
on two sheets. Selected wires that stay behind are counted. Without a selected
symbol or net label the command is unavailable. A taken or empty name shows the reason and offers no move. Nothing
changes until **Move to Sheet**; Cancel leaves the project as it was.

`schematic.split_sheet` applies the same plan with the same parameters plus
`expected_revision`, and optionally `x_nm` and `y_nm` (given together) for the
new sheet symbol; by default it sits where the moved objects were. Symbols, part
instances, wires, junctions and net labels keep their IDs, references and
positions on the new child sheet. Each port becomes a hierarchical label in the
child, wired to the inside members, and a pin on the sheet symbol, wired to the
outside members; a cut wire's outside end becomes the label's place. Global and
power nets keep connecting by name. The split is one undo step, and it is
refused, leaving the project and revision unchanged, if the resulting netlist
would group any terminals differently or drop a net name. A net that had only
a generated number gains the port name, so a later board update reports it as
the same connections under a new name.
