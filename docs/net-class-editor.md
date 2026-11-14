# Net classes in the editors

**Analyze → Rules** opens the same class and assignment form in schematic and PCB
views. Classes belong to the project, so schematic-only authoring does not need
a physical board, footprint, stackup or material catalog. The form uses the
[existing catalog](net-classes.md), [assignments](net-class-assignments.md) and
[dimensional resolver](net-class-consumers.md); it adds no saved vocabulary.

On **Classes**, choose an existing definition or **New class**, enter its name,
and check the dimensions the class should contribute. Hard limits and routing
preferences have separate sections. An unchecked dimension displays the current
board default, or the built-in default when there is no board. It remains absent
from the class. Inputs accept the shared length syntax, including explicit `mm`,
`mil` and `in` suffixes. Untouched fields retain their exact nanometres even when
the selected display unit rounds the text. **Save class** creates or updates one
stable class identity in one undoable transaction.

Select a schematic wire or net label, or PCB tracks/vias, and use **Net rules…**
in Properties to open **Selected net**. Alternatively, search the chooser for a
current supported net. Display names help browsing; typed component, occurrence
and net identities determine the target. Select the complete set of classes and
choose **Assign classes**. Clearing all boxes removes the assignment. A previously
derived net is promoted only when a nonempty assignment is saved.

The page previews effective hard values and preferences and reports conflicts.
An unresolved preference displays **Unresolved**, rather than picking one class.
**Contributions and identities** shows every contributing built-in, board or class
source, including class UUID and definition revision; **Assignment origins** shows
the net identities retained through topology reconciliation. These are the same
sources available through `nets.inspect_classes`, `net_classes.resolve` and the
PCB contact query `pcb.route_rules`.

Bare pins, free unnamed wire/copper groups and selections spanning distinct nets
cannot provide one assignment target. Conflicting physical copper and accepted
memberships spanning a pending logical split also refuse selection-based
assignment. A current net can still be chosen explicitly. Repeated-instance and
board-transfer qualification belongs to #307; advanced electrical, relationship
and pour/thermal intent belongs to #304–#306.

**Board defaults…** opens the existing board-scoped form when a board exists.
Class definitions and assignment drafts remain disposable until saved; Cancel or
closing Rules changes nothing. Changed drafts prevent navigation that would
discard their edits. A project revision or originating view-target change disables
saving, and closing that view expires the form. Successful class/assignment edits
refresh the form from the same source view. Live MCP mutations are refused while
a project modal owns input; after dismissal, GUI and MCP share revision checks,
Undo/Redo and native save/reopen.
