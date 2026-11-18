# Pin and pad correspondence

The engine resolves every placed symbol appearance and physical pad belonging to
one logical terminal from a single immutable project snapshot. This is the
resolver foundation for [#396](https://github.com/tangency-labs/pcbcraft/issues/396).
Individual pin/pad selection and cross-view highlighting remain in #397 and #398.

`Project::resolve_counterparts` takes a typed `board::correspondence::Source`:
a `TerminalTarget` with an explicit project/sheet and terminal address, or a
`PadTarget` with an explicit project/board and pad address. Results include all
placed appearances of the logical terminal across supported symbol units and
all physical lands in its explicit mapping. Terminals are ordered by stable unit
ID; pads are ordered lexically by their exact string IDs. Alphanumeric numbers,
permuted mappings and one terminal mapped to several pads are preserved.

The resolver reads the actual placement and its exact project-pinned definition.
A newer desired footprint assignment awaiting Update PCB does not replace that
mapping. It neither fetches library updates nor derives links from reference
text, matching numbers, coordinates, proximity or electrical net membership.

`Session::resolve_counterparts` additionally checks the captured session revision.
A stale query returns empty counterpart arrays. Reopening, Undo and Redo advance
the session revision, so clients must inspect again. Querying does not change
authored bytes, dirty state, revision, snapshot identity or document history.
The query has no worker, network access or deferred completion. Its sets are
bounded by the existing project, symbol-unit and footprint-pad limits.

## MCP

The read-only `resolve_counterparts` tool is shared by headless and live dispatch.
Pass `expected_revision` from `inspect_project.revision`, the project UUID from
`inspect_project.project.id`, and one qualified `source`:

```json
{
  "expected_revision": 17,
  "source": {
    "kind": "terminal",
    "project": "<project UUID>",
    "sheet": "<sheet UUID>",
    "symbol": "symbol-2",
    "terminal": "V+"
  }
}
```

A pad source instead uses `kind: "pad"`, `project`, `board`, `footprint` and
`pad`. UUIDs and symbol/pad IDs come from inspection. Unknown fields and malformed
addresses are refused. The response has `format: "pcbcraft-counterparts/1"`,
`source`, `status`, `component` (null if unavailable), `terminals` and `pads`, plus
the current `revision` and runtime `opening`. Each endpoint in the arrays is
itself a qualified source address. Resolved results include the complete
terminal/land group in either query direction.

The existing `cross_probe` tool and `Project::pad_for_terminal` /
`terminal_for_pad` retain their representative, singular compatibility behavior.
No existing singular response becomes an array, and no native format changes.

## Unavailable results

| Status | Meaning |
| --- | --- |
| `resolved` | The complete supported terminal and pad sets are available |
| `no_project`, `no_board` | The requested project session or physical board is absent |
| `foreign_project`, `foreign_sheet`, `foreign_board` | The explicit owner does not resolve in this captured project |
| `stale` | The captured session revision is no longer current |
| `missing_target` | The source symbol/pin or footprint/pad no longer exists |
| `unassigned` | The logical component has no package assignment or placement |
| `unplaced_package` | A pinned package choice exists without a physical placement |
| `unplaced_symbol` | The accepted physical mapping names a logical terminal whose symbol unit is not placed |
| `unresolved_mapping` | Accepted data cannot resolve the link, including a removed logical counterpart |
| `unconnected_pad` | The physical pad is explicitly declared unconnected |

An unavailable result may retain validated source-side endpoints, such as an NC
pad or an unassigned terminal. It never invents a counterpart or substitutes an
owner selection. These results grant no edit authority. Full repeated-instance
and multiboard workflows remain outside this slice; existing explicit ownership
is validated without introducing another connectivity graph.
