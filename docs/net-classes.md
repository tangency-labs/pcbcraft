# Net-class catalog and dimensional resolution

Issue [#300](https://github.com/tangency-labs/pcbcraft/issues/300) supplies a
project-wide catalog and a resolver for explicitly supplied class sets. Classes
can be authored in a schematic-only project without creating a board or a
stackup. This slice does not assign classes to nets, expose an editor form or
apply class rules to routing, clearance queries or DRC. Those consumers continue
using [board defaults](board-rules.md) until #301–#303 implement their boundaries.

## Authored definitions

The catalog has `format: "pcbcraft-net-classes/1"` and a `definitions` array.
Each definition has a non-nil UUID `id`, positive `revision`, editable `name` and
sparse `rules` object. A class ID also identifies its dimensional rule set;
individual contributions are qualified by that ID, definition revision and
property. Names are 1–200 trimmed characters without controls. Duplicate names
are allowed because names do not select or prioritize rules. The catalog holds
at most 256 definitions with distinct IDs. Unknown properties, duplicate stored
property keys and unsupported versions are refused.

Every dimension is an integer number of nanometres. Omitted properties mean no
class opinion; a supported zero is explicit intent. There are no unit-bearing
strings, layer selectors, expressions, manufacturing profiles or electrical
calculations in this vocabulary. Empty rule sets are valid classifications.

| Property | Valid target / meaning | Combination |
| --- | --- | --- |
| `clearance_nm` | Copper belonging to different intended nets; pairwise minimum spacing | Largest applicable minimum |
| `min_track_width_nm` | Routed tracks; zero disables this minimum | Largest applicable minimum |
| `edge_clearance_nm` | Copper versus the addressed board outline | Largest applicable minimum |
| `min_annular_ring_nm` | Radial copper around a routed via drill | Largest applicable minimum |
| `min_via_diameter_nm` | Routed via outer diameter lower bound | Largest applicable minimum |
| `max_via_diameter_nm` | Routed via outer diameter upper bound | Smallest applicable maximum |
| `min_via_drill_nm` | Routed via drill lower bound | Largest applicable minimum |
| `max_via_drill_nm` | Routed via drill upper bound | Smallest applicable maximum |
| `preferred_track_width_nm` | Upcoming routed track size, not existing copper | Class preference, otherwise addressed board preference, otherwise built-in |
| `preferred_via_diameter_nm` | Upcoming routed via outer diameter | Same preference precedence |
| `preferred_via_drill_nm` | Upcoming routed via drill diameter | Same preference precedence |

Per-property ranges match `board::rules::Values` and command discovery: track
widths cannot exceed 10 mm, via outer diameters range from 0.050001 to 10 mm,
and via drills range from 0.05 to 9.999999 mm. Preferred track width has the
existing supported geometry minimum, 0.01 mm. Clearance, edge clearance and
annular ring permit zero through the supported coordinate range. Limits describe
supported representation, not fabrication qualification. Class validation refuses
internally reversed bounds or incompatible authored preferences, including via
drill/diameter/ring relationships. It does not populate omitted fields with
policy defaults.

## Resolution contract

`Project::resolve_net_classes(classes, board)` accepts an explicit set of class
UUIDs and an optional typed `BoardTarget`. Without a board, it uses built-ins
even if the project has a physical board. With a board, that exact project's
board defaults replace built-ins as in #242. A foreign or missing board and an
unknown class are errors; no first-board or display-name fallback is used.

Applicable hard limits intersect. Classes cannot weaken the selected baseline,
and board defaults cannot weaken a class's stronger limit. For preferences, an
explicit class opinion takes precedence over board/built-in preferences. Equal
class preferences agree; differing class preferences have equal precedence and
produce a conflict with no winner. Class UUID order, file order and names never
break ties. An inherited or authored preference outside the resulting hard
bounds is also a conflict, with the affected preference returned as null. The
resolver does not silently clamp a value, invent a preference or relax a bound.
For example, raising minimum track width above the default preference requires
an explicit compatible preference to obtain a conflict-free result.

The result includes the sorted class set, addressed board, per-property `values`,
all applicable `contributions`, and structured `conflicts`. Contributions retain
the baseline plus each relevant class, including overridden preferences and
weaker limits, so the decision can be explained. Each class source names its
rule UUID and definition revision; board sources name the saved board-rule UUID,
and built-in sources are explicit. A command response also carries the captured
session revision. Consumers must retain snapshot/revision authority and reject
stale results through their normal publication boundary.

Contradictory hard bounds and impossible drill/ring/diameter combinations remain
visible in the result with conflicts. Such a result must not be consumed as a
valid rule configuration or treated as a successful design check. Resolution
does not change copper, connectivity, assignments, history or saved bytes.

`Project::resolve_class_clearance(left, right, board)` returns a `PairResolution`
with the larger of the two independently resolved clearances, canonical
clearance contributions, and two object reports sorted by class set. Swapping
sides gives an identical result, including provenance and conflicts. Different
route preferences on different objects are valid; conflicting preferences within
one object's class set remain in that object's report and the pair's deduplicated
conflict list. Each object retains its own dimensions rather than acquiring the
other object's routing preferences. This slice supports board-default overrides
only, not pair-specific exceptions or request-local weakening overrides.

## Shared commands

| Command | Parameters and result |
| --- | --- |
| `net_classes.create` | `expected_revision`, `name`, sparse `rules`; allocates one stable ID at definition revision 1 |
| `net_classes.update` | `expected_revision`, `class`, `name`, complete replacement sparse `rules`; retains ID, increments definition revision on actual changes |
| `net_classes.remove` | `expected_revision`, `class`; removes the definition in one undoable transaction |
| `net_classes.inspect` | No parameters; returns `net_classes` and unchanged session `revision` without preparing connectivity |
| `net_classes.resolve` | Required `classes` UUID array, optional `board` UUID and optional `other_classes`; returns `resolution` and unchanged session `revision` |

Both queries return their data through MCP `execute` even with `inspect: false`.
Normal project inspection includes a nonempty `net_classes` catalog. Commands
accept unique class IDs within the 256-class limit; the native resolver also
deduplicates repeated IDs. Edit failures, stale revisions, exhausted definition
revisions and no-op updates preserve the current snapshot/history. Undo/Redo
restore exact IDs and definition revisions. Read-only commands need no revision
guard. All commands are available through headless and live MCP, with the live
editor's ordinary modal/file-operation refusal policy.

For example, after inspecting the current revision:

```json
{"command":"net_classes.create","params":{"expected_revision":1,"name":"Power","rules":{"min_track_width_nm":400000,"preferred_track_width_nm":500000}}}
```

Inspect the returned catalog to obtain its class UUID, then pass that UUID in
`net_classes.resolve`'s `classes` array. Use fresh inspected revisions for later
edits rather than reusing the example's number.

## Storage and follow-on boundaries

Only projects with classes use `pcbcraft-project/20`. Its required nonempty
`net_classes` record is stored in `project.json`, with definitions sorted by UUID
and one definition per line. It extends the existing folder/occurrence vocabulary
and permits the supported optional no-connect and persistent-net records
together. Board, sheet, instance, archive and library formats are unchanged.
Older project headers reject class records; format 20 rejects missing, null,
empty or invalid catalogs. The historical single-document encoder refuses to
discard classes.

Inspection does not migrate a legacy snapshot. A successful authored class edit
adopts current folder/occurrence metadata atomically when needed; Undo restores
the earlier snapshot. Untouched projects without classes retain their existing
encoding. Removing the last class omits the catalog and selects the format needed
by the remaining data. Canonical folder and `.pc` archive reopen/save preserve
IDs, revisions and bytes.

#301 should bind stable net identities to sets of class IDs and define removal
protection/reconciliation; this slice has no saved bindings to orphan. #302
should reuse this resolver for routing/query/checking consumers, including
snapshot validity and explicit conflicts. #303 owns the shared editor workflow.
No consumer should infer bindings from class names or treat the present catalog
as though it already changes DRC or routing behavior.
