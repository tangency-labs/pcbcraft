# Net classes in the editors

**Analyze → Rules** opens the same class and assignment form in schematic and PCB
views. Classes belong to the project, so schematic-only authoring does not need
a physical board, footprint, stackup or material catalog. The form uses the
[existing catalog](net-classes.md), [assignments](net-class-assignments.md) and
[dimensional resolver](net-class-consumers.md); it adds no saved vocabulary.

On **Class definitions**, choose an existing definition or **New class**, enter its
name, and check the dimensions the class should contribute. Hard limits and routing
preferences have separate sections. An unchecked dimension reads **Not specified**
and contributes no class rule.
Enabling it starts the field from the board or built-in baseline; only saving the
class authors that value. Inputs accept the shared length syntax, including explicit `mm`,
`mil` and `in` suffixes. Untouched fields retain their exact nanometres even when
the selected display unit rounds the text. **Save class** creates or updates one
stable class identity in one undoable transaction.

Select a schematic wire or net label, or PCB tracks/vias, and use **Net rules…**
in Properties to open **Effective net rules**. Alternatively, search the chooser for a
current supported net. [Shared readable names](net-labels.md#generated-names-and-copper-labels) help browsing; typed component, occurrence
and net identities determine the target. Select the complete set of classes and
choose **Assign classes**. Clearing all boxes removes the assignment. A previously
derived net is promoted only when a nonempty assignment is saved.

The page shows **Effective value** and **Active source**, with conflicts above the
dimension rows. Changed checkboxes show an **Unsaved assignment preview** with
preview column headings until **Assign classes** succeeds.
An unresolved preference displays **Unresolved**, rather than picking one class.
**Contributions and identities** distinguishes **Active**, **Overridden**,
**Less restrictive** and **Conflicting** sources, retaining original values, class
UUIDs and definition revisions. Equal hard limits are labelled **Matches built-in minimum/maximum**,
**Matches board minimum/maximum** or **Shared class minimum/maximum**, with the
individual source names in the tooltip and Details. They define one bound,
rather than adding their values. An explicit
class preference replaces the baseline even when their values match.
**Saved assignment origins** shows the net identities retained through topology
reconciliation. These are the same
sources available through `nets.inspect_classes`, `net_classes.resolve` and the
PCB contact query `pcb.route_rules`.

Bare pins, free unnamed wire/copper groups and selections spanning distinct nets
cannot provide one assignment target. Conflicting physical copper and accepted
memberships spanning a pending logical split also refuse selection-based
assignment. A current net can still be chosen explicitly. Repeated-instance and
board-transfer qualification belongs to #307; advanced electrical, relationship
and pour/thermal intent belongs to #304–#306.

The Geometry column marks the measured span beside each dimension: copper-to-copper
and copper-to-edge gaps, track width, radial annular ring, via outside diameter and
drill diameter. These are illustrative drawings, independent of the entered value;
the hover and accessible image description identify the measured boundaries.

**Board Setup → Design rules** embeds **Board defaults** and **Net classes** as
pages in the same window, sharing these editor components and commands. Switching
pages retains pending fields, including invalid input. **Apply board defaults**,
**Save class** and **Assign classes** save their own page and keep Setup open;
other pending layer, stackup or rule drafts remain intact. **Discard class changes**,
**Revert assignment changes** and **Revert changes** reset only their own editor. **Apply board setup**
saves the physical setup and retains the window if another rule page still has
unsaved fields. Cancel/close discards remaining drafts; it does not undo edits
already explicitly saved. Opening rule pages does not apply a suggested initial
stackup. **Analyze → Rules** remains available from either workspace, including
schematic-only designs. These dialogs stay in their originating editor window.

**Board defaults…** in Rules opens the board-scoped form when a board exists, using the same
row layout and illustrations. Built-in values are read-only until **Use built-in
defaults** is cleared; labels and diagrams remain legible in either state.
Class definitions and assignment drafts remain disposable until saved; Cancel or
closing Rules changes nothing. Changed drafts prevent navigation that would
discard their edits. A project revision or originating view-target change disables
saving, and closing that view expires the form. Successful class/assignment edits
refresh the form from the same source view. Live MCP mutations are refused while
a project modal owns input; after dismissal, GUI and MCP share revision checks,
Undo/Redo and native save/reopen.

## Differential pairs

Open **Analyze → Rules → Differential pairs…** in the schematic or PCB editor.
For a new pair, enter a name and choose the positive and negative nets explicitly.
The net pickers list active nets that are not already in another pair; an existing
pair's own endpoints stay selectable even when retired. If a derived net has no
persistent ID yet, promote it before pairing. To edit or remove an existing pair,
select it from the pair picker. A retained member without a current name is shown
by UUID. A pair that needs review shows its reason. Renaming keeps it; saving with
a changed member, or **Mark reviewed** (enabled only when both members are live),
clears it. **New pair** explains when fewer than two active unpaired nets are
available.

**Save pair** and **Remove pair** each apply one edit and close the form. Removal
leaves both nets intact. Cancel discards the draft, and Undo restores the exact
previous relationship. The [MCP commands](mcp.md#differential-pairs) use the same
validation and history. This records intent only: no pair classes, suffix
inference, split/merge repair or differential routing are applied.
