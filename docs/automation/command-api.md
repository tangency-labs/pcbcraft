# MCP command API

## Command API version

`list_commands` keeps returning the command array as its first text part. A
second text part, also given as the result's `structuredContent`, reads
`{"command_api_version": 1, "catalog_revision": "<64 hex digits>", "capabilities": ["command_api", "error_codes", "version_check", "deprecation_warnings"]}`.
These are three separate things:

- `command_api_version` is one number for the whole public command contract,
  `COMMAND_API_VERSION` in `crates/engine/src/command_api.rs`. It rises only
  when a published change breaks existing callers, by an explicit decision, and
  never falls. There is no version per command.
- `catalog_revision` is the SHA-256 of every command this session describes,
  sorted by id, with object keys sorted and titles, summaries and descriptions
  included. It changes whenever a command is added or removed or a description
  changes, so a client can drop cached discovery data; a new revision alone
  does not mean existing callers are incompatible. It covers the whole catalog
  whatever `filter` keeps. A live editor (project, window and `ui.` commands)
  and a headless session (project commands) describe different catalogs, and
  builds differ, so each reports its own revision; it is computed when asked,
  not stored.
- `capabilities` names the optional protocol features this build offers; a
  feature added later is appended and none is removed. Command schemas from
  `list_commands` and `describe_command` say which commands and parameters the
  running build supports.

None of them is the MCP `protocolVersion` negotiated by `initialize`,
`serverInfo.version` (the build's version) or a native file-format version,
and none is saved in projects.

[`docs/schemas/command-contracts.json`](../schemas/command-contracts.json) is a
snapshot of every command's contract. It contains `command_api_version` and
two sorted command maps, `commands` for project commands and `window_commands`
for window and `ui.` commands. Each command is expanded across lines so changes
to different parameters can merge. Object keys are sorted. The snapshot omits
command IDs from the entry bodies and drops prose and deprecation annotations
from commands and schemas. Values of `const`, `default`, `enum` and `examples`
are kept whole. The engine and app tests compare the described commands with it
and label detected differences. If either parameter schema contains `$ref`,
`$dynamicRef` or `$recursiveRef`, every difference in it is conservatively
breaking: an unchanged reference may reach any target, including a property
or array item used under `not` elsewhere. The classifier does not resolve
reference targets or their polarity. The same conservative treatment applies
if either schema contains `unevaluatedProperties` or `unevaluatedItems`, even
nested: a locally looser alternative can stop evaluating a property or item,
so an unchanged outer restriction can refuse a formerly valid call. Both
scans also include data values, which can cause extra breaking labels. If
unequal schemas in these guarded contexts produce no child diagnostic (for
example, removing an empty map), the whole schema is reported as breaking.
Without references or unevaluated keywords, the rules are:

- compatible, when every call the old schema accepted is still accepted: a
  command added, an optional parameter added to an object that refuses
  unknown names (`additionalProperties: false` and no `patternProperties`,
  as every command's top-level parameters are), a required parameter made
  optional, enum values added, a type widened (`integer` to `number`
  included), a bound loosened, a `pattern`, `format`, `const`, `multipleOf`
  or `uniqueItems` dropped, `additionalProperties` or `items` loosened,
  `anyOf` alternatives appended, a `$defs` or `definitions` entry added,
  `examples` changed, or an equal-length `prefixItems` child loosened;
- breaking otherwise: a command or parameter removed, a parameter newly
  required, enum values removed, a type narrowed or added, a bound tightened or
  added, a `pattern`, `format`, `const` or `multipleOf` added or changed, a
  `default` added, changed or dropped (also by replacing a schema that
  carries one with one that accepts anything), a `$defs` or `definitions`
  entry removed or changed in any way (a definition may be used under `not`,
  where loosening it refuses calls), a property added to an object that
  accepted unknown names (a call may already pass that name with another
  value), a changed `prefixItems` length or a tightened child, any `oneOf`
  change (a value may then match two alternatives), any
  change under a keyword compared as a whole (`not`, `allOf`, `$ref`, `if`,
  `then`, `else`, `patternProperties`, `dependentRequired`,
  `dependentSchemas`, `dependencies`), and any change under a keyword without
  a rule.

The rules apply at every depth of a schema. Against an unchanged snapshot,
a detected breaking difference fails the tests until `COMMAND_API_VERSION`
is raised. After the raise, or for other detected differences, the failing
test prints the section to copy into the snapshot. Rewriting a snapshot in
the same commit instead needs comparison with a retained prior snapshot.
The snapshot drops a removed command; an id that comes back later is an
addition.

The tests see only the described parameter schemas. Changed units, a changed
result or meaning, and a stricter check the schema does not express need the
same explicit decision with no failing test to prompt it; and they cannot tell
whether a new default keeps the old behaviour. They also compare the code with
the snapshot in the same commit, so a breaking change committed together with
a rewritten snapshot passes them. When the pull request’s target commit has a snapshot, CI on
a pull request also compares that `command-contracts.json` with the pull
request's checked-out merge result as part of the engine tests. The same
classifier is available locally with `cargo run -p pcbcraft-engine --example
command_contracts_against_base -- BASE.json HEAD.json`. The comparison fails when the
classifier detects a breaking difference in either section without a raised
`command_api_version`, or when the version falls. If the target commit has no
snapshot, the workspace tests check the first snapshot against discovery
instead. The cross-revision check sees only recorded contracts, so a changed
meaning still needs review, and it cannot tell whether a raise was the right
decision; a review must still read
every change to `command-contracts.json`.

### Stating a version

Pass optional `version` to `execute` to pin the `command_api_version` from
`list_commands`. There is no per-command version. Omitting `version` runs
against the session’s current API. The MCP tool accepts a positive JSON
integer (for example `1`, not `1.0`, `"1"` or `null`). Check the serving
session’s `version_check` capability before pinning. Legacy MCP backends that
do not implement pinning refuse a pin without dispatching; unpinned calls
still work.

The MCP bridge and raw live clients that need a guarantee across editor versions
use `engine.execute_versioned` with a required non-null `params.version` beside
`params.command`. Use the `command_api_version` reported by the serving
editor's command discovery (`list_commands` over MCP, `engine.commands` on the
raw live endpoint). For example, if the editor reports version 1:

```json
{"method":"engine.execute_versioned","params":{"command":"edit.undo","version":1}}
```

A missing or null pin on that method is `invalid_request`; an older editor
rejects the method rather than executing the command. Use plain `engine.execute`
for unpinned requests. A current editor also checks a non-null `version` on
plain `engine.execute` (omitted or `null` means unpinned), but a legacy editor
ignores it and may execute the command. A malformed pin on a known command
that reaches the version check returns `invalid_params` on the live endpoint.

A known, routable project, window or `ui.` command with a different API version
returns `version_mismatch` without running. Refresh the catalog and adapt the
call. Unknown commands and a missing live editor keep their existing refusals.
Outer admission checks still run first, including assistant approval, ordinary
project/window editor refusal and deferred-job expiry, busy and editor refusal.
The `ui.` handler's own checks, such as busy or native focus, run only after a
matching or omitted pin.

### Deprecated commands and parameters

Mark a command or top-level parameter deprecated for at least one release
before removing it. Discovery annotates the command or its schema under
`params.properties` with `"deprecated": true` and
`"x-deprecation": {"since": "0.2.0", "replacement": "new_name"}`. The
replacement is a command id or parameter name, or `null` when there is none.
Marking a deprecation changes discovery and its catalog revision, not the
command contract snapshot or the API version.

A successful call that uses something deprecated still returns its ordinary
result as the first MCP text part. A second text part, also given as
`structuredContent`, carries `{"warnings": [...]}`. Each entry has `code`
(`deprecated`), `command`, `parameter` (`null` for a whole command), `since`,
`replacement` and a human-readable `message`. Passing a deprecated parameter
explicitly counts as use even when its value is `null` and the command accepts
it. The live endpoint puts `warnings` beside `result`; failures and successes
without warnings keep their previous execution reply shape. The
`deprecation_warnings` capability indicates support. No production command or
parameter is deprecated yet.

## Error codes

A tool call that fails after its arguments were accepted is still a result with
`isError: true` whose first text part is the message, as before. (Argument
faults caught by the MCP tool validator, such as a missing `command`, remain a
JSON-RPC `invalid params` error without a PCBCraft code.) A second text part, also given as the result's
`structuredContent`, reads
`{"error": {"code": "stale_revision", "message": "...", "details": {"current_revision": 12}}}`.
Branch on `code`, never on the message: messages may be reworded, codes are
not. `details` is an object, empty unless the table says otherwise; its keys are
only ever added. The live editor endpoint answers a failed request with
`{"error": message, "code": ..., "details": {...}}`; a reply without a code, from
an older editor, reads as `unavailable`. A well-formed code from a newer editor
that this list does not have yet is passed through unchanged; treat a code you
do not know like `unavailable`. Missing or non-object details in a live reply
read as an empty object. The table describes failures produced by this build;
a malformed reply from another editor may omit the facts it describes.

| Code | Meaning |
| --- | --- |
| `unknown_command` | No command has the requested id. |
| `invalid_params` | The parameters are a non-null scalar or array, name a parameter the command does not describe, leave out a required one, have the wrong type or range, or fail the command's own checks. |
| `unavailable` | The command cannot run in the current state: no project is open, nothing to undo, a dialog or file operation is open in the editor, and similar; also a headless-only command sent to an open editor. |
| `stale_revision` | The request was prepared against an earlier revision. `details.current_revision` gives the current one. For annotation, `details.part_instances` names affected parts; preview again instead of retrying a stale token. |
| `not_found` | An ID names nothing in the project, such as a deleted wire, symbol, net label, sheet or footprint. Some commands report an ID that is not of the expected form the same way, others as `invalid_params`. |
| `cap_exceeded` | The edit would pass a fixed limit, such as 256 net labels on a sheet, the sheet or wire limit, bends on a wire, or the project's object IDs, or a live `schematic.complete_packages` report would exceed the 16 MiB reply limit (nothing is edited). |
| `needs_live_editor` | A window or `ui.` command, or `inspect_editor`, in a session without an open editor. |
| `denied` | The person declined the request in the editor's assistant. |
| `cancelled` | The request was stopped, timed out or lost its connection before it finished; nothing changed. |
| `busy` | The editor is already running a request of the same kind, such as a design check, or has reached a worker or request capacity limit; retry later. |
| `no_reply` | No usable reply came back from the editor, so the command may have run; inspect before retrying an edit. |
| `connection_failed` | Connecting or sending the request failed. A failed send may still have delivered a complete request; inspect before retrying an edit. |
| `invalid_request` | The request to the live editor is malformed: an unknown method, a missing command id or a missing required version pin. |
| `version_mismatch` | The stated command API version is older or newer than the one this session serves; the command did not run. `details` gives `command`, `stated_version` and `current_version`. |
| `out_of_scope` | A known command is not allowed in the read-only session. `details.command` names it and `details.scope` is `read_only`. The check precedes revision and command parameter validation. |

A version pin is checked before the command’s parameters, including
`expected_revision`. Once that pin matches or is omitted, the read-only scope is checked.
After that, project commands that take `expected_revision` check it before
their other parameters. A stale
edit reports `stale_revision`; a malformed revision is `invalid_params`. The read-only [`pcb.plan_route`](../pcb/route-planning.md#command)
query keeps its existing `stale` plan outcome for otherwise valid requests. The
part and library document revisions that some window commands take are reported
as `unavailable` when stale, for now.
At direct command and live control dispatch, catalog validation treats omitted
or null parameters as an empty map; required keys still apply. Other non-object
parameter values reaching that validation are `invalid_params`. At the outer
MCP `execute` boundary, omit `arguments.params` to use the same empty-map
behavior; a supplied `arguments.params: null` is a JSON-RPC `invalid params`
error. Deferred work can be cancelled or refused by capacity or live host policy
before engine validation. Once admitted, deferred copper checks stale revision,
then catalog parameters, then board identity like headless execution.
After live host policy permits a window request, its catalog reports
`unknown_command` for an unknown id and `invalid_params` for unknown keys,
missing required keys or invalid value types. A host-policy refusal remains
`unavailable`, before that classification. UI commands use their separate
input lifecycle: their catalog checks ids, parameter names and required keys;
other handler refusals, including UI value checks, are `unavailable` for now.
The list is locked in
[`docs/schemas/error-codes-v1.json`](../schemas/error-codes-v1.json): codes are
only appended, never renamed, removed or reused; the engine tests fail when the
lock and the code disagree. A code that is no longer produced stays listed. A failure may move to a more
specific code in a later version: some lookups and limits still report
`invalid_params`, and some state checks `unavailable`, so treat those two as
the general cases they are. The capability
`error_codes` says a build reports them.
