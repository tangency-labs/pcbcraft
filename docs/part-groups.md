# Part groups

A part group is a named set of part instances. Groups are project-owned, so the
schematic and the board share one catalog and one set of memberships, and a
group can be authored before any board exists. A part instance can belong to
several groups. This page describes the catalog, explicit membership and their
storage; [net classes](net-classes.md) follow the same identity rules for nets.
Part instances and part definitions are described in [part-schema.md](part-schema.md).

## Identity

Each group has a stable UUID, a revision from 1 to 4294967295 and an editable
name. A rename that would take the revision past that range is refused. The UUID
is the only lookup authority: names may repeat and never select membership.
Names are 1–200 trimmed characters without control characters. A project holds
at most 256 part groups.

Membership is stored by part-instance identity, never by designator, so
annotating or renaming a reference keeps every membership. Commands address a
part instance by the ID project inspection reports; the saved record stores its
number, the same number the project's part-instance record and its placed
symbols carry. One accessor pair converts between the two, so a later change of
part-instance identity rewrites membership in one place.

## Legacy names

Two published names keep the older word for compatibility:

| Name | Meaning |
| --- | --- |
| `component-N` | The ID of part instance `N` in commands and reports. `part_instances.set_groups` and `part_instances.inspect_groups` take it in `part_instance`. |
| `components` in `project.json` | The project's part-instance records, keyed by the same number `N` that a membership's `part_instance` stores. |

Both stay as they are on every saved format; renaming either would need a new
project format with a reader for the old one.

## Shared commands

| Command | Parameters and result |
| --- | --- |
| `part_groups.create` | `expected_revision`, `name`; allocates one stable ID at revision 1 |
| `part_groups.update` | `expected_revision`, `group`, `name`; retains ID and membership, increments the revision on an actual change |
| `part_groups.remove` | `expected_revision`, `group`; refuses a group that any part instance belongs to and lists those part instances |
| `part_groups.inspect` | No parameters; returns `part_groups` and the unchanged session `revision` |
| `part_instances.set_groups` | `expected_revision`, `part_instance`, complete `groups` UUID set; replaces that part instance's memberships in one undo step, and an empty set removes them |
| `part_instances.inspect_groups` | Optional `part_instance` (omit it rather than passing null); returns `part_instance_memberships` for that part instance, or for every grouped part instance, and the unchanged `revision` |

Both queries return their data through MCP `execute` even with `inspect: false`.
Normal project inspection includes a nonempty `part_groups` catalog. Group sets
must be unique non-nil UUIDs naming existing groups. Unknown part instances or
groups, stale revisions and invalid names refuse the whole request; no-op
updates and unchanged sets keep the snapshot, history and revision. Undo/Redo
restore exact IDs, revisions and memberships. Deleting a part instance removes
its memberships in the same undo step. All commands are available through
headless and live MCP with the live editor's ordinary refusal policy.

For example, after inspecting the current revision:

```json
{"command":"part_groups.create","params":{"expected_revision":1,"name":"Connectors"}}
{"command":"part_instances.set_groups","params":{"expected_revision":2,"part_instance":"component-3","groups":["<group UUID from inspection>"]}}
```

Use fresh inspected revisions for later edits rather than reusing the example's
numbers.

## Storage

A nonempty catalog selects `pcbcraft-project/24` and is stored in `project.json`
directly after any net-class catalog, with groups sorted by UUID and one per
line. Readers accept the groups in any order and sort them by UUID, as for net
classes:

```json
"part_groups": {
  "format": "pcbcraft-part-groups/1",
  "groups": [
    {"id":"5d1c…","revision":1,"name":"Connectors"}
  ]
},
```

Memberships live in the project's instance home beside the sheet occurrences and
any net registry: `schematic_instances` in `project.json` when there is no board,
or the board folder's `instances.json`, which then uses
`pcbcraft-instances/5`. Records are sorted by part instance and then group,
unique, and one per line:

```json
"part_group_memberships": [
  {"part_instance":3,"group":"5d1c…"}
]
```

Project/24 is a superset of project/23: generated net numbers, net classes and
their assignments, project parameters and no-connect intent are all optional
with their existing rules. Both fields are omitted when empty; explicitly empty
lists, null, unknown fields, unsorted or repeated records, and records naming a
missing part instance or group are refused. Older project and instance headers
refuse these records, and instances/5 requires memberships. A catalog without
members leaves the board instance header unchanged. The diagnostic single-file
encoding refuses part groups.

Inspection does not migrate a project. The first group edit adopts current
folder and occurrence metadata in the same undo step. Projects without part
groups keep their existing format and bytes, and removing the last group
returns the entry point to the version its other content needs.

## Not yet covered

Membership is explicit only. Pattern membership by part parameter or sheet,
removal with rebinding of members to another group, bulk assignment and rule
scopes that consume part groups are later work on the same catalog and
commands. The shared parameter checks for UUID and UUID-set parameters still
word their refusals with "class"; wording them per catalog is a later change.
