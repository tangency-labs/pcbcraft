# Hierarchical sheets

Place a sheet with **S** or **Capture → Sheets → Subsheet**, then click its top-left
corner on the canvas. The form can create a named child or place an existing
unparented sheet. Creation and placement are one undo step. The Browser displays
children beneath their parents, while every document keeps its UUID and filename.
Each child has one parent; repeated instances and cycles are refused.

Double-click inside the sheet symbol to enter its child. **Alt+Backspace** returns
to the parent in the same pane. Browser navigation works in both schematic and
PCB workspaces, and other panes/windows keep their own document and camera.

Inside the child, use **H** or **Capture → Sheets → Hierarchical label**, click an anchor position
and enter a name and direction. The label exposes a matching pin on the parent's
sheet symbol automatically. Draw a wire to the pointed label anchor or parent
pin with **W**; an ordinary click also starts a wire at an unconnected anchor.
Parent wires and child wires then belong to one project electrical net. Repeated
names on other sheets do not connect them. The Net Inspector continues to list
the active sheet's local groups; complete cross-sheet groups are available through
project inspection. Local/global labels and power symbols remain issue #42.

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
connectivity in folders and packed projects; see the [storage contract](native-project-storage.md#hierarchical-sheet-records).
