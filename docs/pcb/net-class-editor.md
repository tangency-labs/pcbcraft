# Net classes in the editors

**Analyze → Rules** opens the same class and assignment form in schematic and PCB
views. Classes belong to the project, so schematic-only authoring does not need
a physical board, footprint, stackup or material catalog. The form uses the
[existing catalog](../formats/net-classes.md), [assignments](../formats/net-class-assignments.md) and
[dimensional resolver](../formats/net-class-consumers.md); it adds no saved vocabulary.

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
current supported net. [Shared readable names](../schematic/net-labels.md#generated-names-and-copper-labels) help browsing; typed component, occurrence
and net identities determine the target. Select the complete set of classes and
choose **Assign classes**. Clearing all boxes removes the assignment. A previously
derived net is promoted only when a nonempty assignment is saved.

In the standalone Rules window, **Net color…** edits the board's personal
color for a selected net with an active persistent ID. The color picker
returns to **Effective net rules** and retains any pending class choices.

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

**Working voltage** declares one [working-voltage requirement](../formats/electrical-intent.md)
on a saved net or class. Choose the owner under **Declared on**; a selected net
with a saved identity is chosen already. Check **Declare working voltage**, then
enter the value in volts, a **DC** or **Peak AC** basis and an **Earth** or net
reference, and choose **Save working voltage**. Clearing the check removes the
requirement. **Status** reports where the value was entered and that nothing has
verified it. When the net or its reference was removed, split or merged, or when
resolving a split retired its provisional identity, a **Needs review** notice
names the reason. The original value and reference stay selected until you change
them. After a split that kept the identities, choose **Save working voltage** to
record the review. An ambiguous or retired net that still carries a declaration
is listed under **Declared on** as **Former net**; its declaration can be reviewed
and cleared, but not changed. If resolving a split restored the original net,
declare the voltage on that net or select it as a reference if it still applies.
Only named nets are offered as references. Each save is
one Undo step and uses
`electrical_intent.set`, and the page reads the same report as
`electrical_intent.inspect`.

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
