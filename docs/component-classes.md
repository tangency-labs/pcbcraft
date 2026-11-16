# Component classes

A component class is a named group of components. Classes are project-owned, so
the schematic and the board share one catalog and one set of memberships, and a
class can be authored before any board exists. A component can belong to several
classes. This page describes the catalog, explicit membership and their storage;
[net classes](net-classes.md) follow the same identity rules for nets.

## Identity

Each class has a stable UUID, a definition revision from 1 to 4294967295 and an
editable name. A rename that would take the revision past that range is refused. The UUID is the only lookup authority: names may repeat and never select
membership. Names are 1–200 trimmed characters without control characters. A
project holds at most 256 component classes.

Membership is stored by component identity, never by designator, so annotating
or renaming a reference keeps every membership. Commands address a component by
the ID project inspection reports (`component-N`); the saved record stores the
number `N`, the same number the component record and its placed symbols carry.
One accessor pair converts between the two, so a later change of component
identity rewrites membership in one place.

## Shared commands

| Command | Parameters and result |
| --- | --- |
| `component_classes.create` | `expected_revision`, `name`; allocates one stable ID at definition revision 1 |
| `component_classes.update` | `expected_revision`, `class`, `name`; retains ID and membership, increments the definition revision on an actual change |
| `component_classes.remove` | `expected_revision`, `class`; refuses a class that any component belongs to and lists those components |
| `component_classes.inspect` | No parameters; returns `component_classes` and the unchanged session `revision` |
| `components.set_classes` | `expected_revision`, `component`, complete `classes` UUID set; replaces that component's memberships in one undo step, and an empty set removes them |
| `components.inspect_classes` | Optional `component` (omit it rather than passing null); returns `component_memberships` for that component, or for every component that belongs to a class, and the unchanged `revision` |

Both queries return their data through MCP `execute` even with `inspect: false`.
Normal project inspection includes a nonempty `component_classes` catalog. Class
sets must be unique non-nil UUIDs naming existing classes. Unknown components or
classes, stale revisions and invalid names refuse the whole request; no-op
updates and unchanged sets keep the snapshot, history and revision. Undo/Redo
restore exact IDs, definition revisions and memberships. Deleting a component
removes its memberships in the same undo step. All commands are available through
headless and live MCP with the live editor's ordinary refusal policy.

For example, after inspecting the current revision:

```json
{"command":"component_classes.create","params":{"expected_revision":1,"name":"Connectors"}}
{"command":"components.set_classes","params":{"expected_revision":2,"component":"component-3","classes":["<class UUID from inspection>"]}}
```

Use fresh inspected revisions for later edits rather than reusing the example's
numbers.

## Storage

A nonempty catalog selects `pcbcraft-project/24` and is stored in `project.json`
directly after any net-class catalog, with definitions sorted by UUID and one per
line. Readers accept the definitions in any order and sort them by UUID, as for
net classes:

```json
"component_classes": {
  "format": "pcbcraft-component-classes/1",
  "definitions": [
    {"id":"5d1c…","revision":1,"name":"Connectors"}
  ]
},
```

Memberships live in the project's instance home beside the sheet occurrences and
any net registry: `schematic_instances` in `project.json` when there is no board,
or the board folder's `instances.json`, which then uses
`pcbcraft-instances/5`. Records are sorted by component and then class, unique,
and one per line:

```json
"component_class_bindings": [
  {"component":3,"class":"5d1c…"}
]
```

Project/24 is a superset of project/23: generated net numbers, net classes and
their assignments, project parameters and no-connect intent are all optional
with their existing rules. Both fields are omitted when empty; explicitly empty
lists, null, unknown fields, unsorted or repeated records, and records naming a
missing component or class are refused. Older project and instance headers
refuse these records, and instances/5 requires memberships. A catalog without
members leaves the board instance header unchanged. The diagnostic single-file
encoding refuses component classes.

Inspection does not migrate a project. The first class edit adopts current
folder and occurrence metadata in the same undo step. Projects without component
classes keep their existing format and bytes, and removing the last class
returns the entry point to the version its other content needs.

## Not yet covered

Membership is explicit only. Pattern membership by part parameter or sheet,
removal with rebinding of members to another class, bulk assignment and rule
scopes that consume component classes are later work on the same catalog and
commands.
