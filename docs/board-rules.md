# Board rules

**Analyze → Rules** edits the physical board's saved dimensional limits and
preferred routing sizes. The same `pcb.set_rules` transaction is available to
headless and live MCP. Values are integer nanometres in storage; the form accepts
the configured display units or an explicit suffix. Opening, inspecting or
cancelling the form changes no project data.

| Property | Built-in value | Meaning |
| --- | --- | --- |
| Copper clearance | 0.2 mm | Minimum spacing between different intended nets |
| Minimum track width | 0.15 mm | Hard minimum checked against existing tracks |
| Board edge clearance | 0.3 mm | Minimum copper-to-outline spacing |
| Minimum annular ring | 0.1 mm | Minimum radial via copper around the drill |
| Minimum / maximum via diameter | 0.050001 / 10 mm | Supported outer-diameter bounds |
| Minimum / maximum via drill | 0.05 / 9.999999 mm | Supported drill-diameter bounds |
| Preferred track width | 0.25 mm | Initial width for upcoming routes |
| Preferred via diameter / drill | 0.6 / 0.3 mm | Initial dimensions for upcoming vias |

The broad built-in via bounds preserve the existing supported geometry range;
they are not a fabrication qualification. Preferred dimensions must satisfy all
hard limits, including the annular ring. Reversed or impossible bounds, invalid
units and unsupported sizes are refused without partial changes. Changing rules
does not resize or unlock existing copper: DRC reports any resulting violations.
Explicit copper dimensions remain authored geometry, so they can be inspected
and checked even when they violate a saved limit. Clearance enforcement while
routing remains separate work.

## Resolution and commands

Each property resolves from an authored board default when present, otherwise
from its built-in value. Board inspection includes `rules.authored` and
`rules.effective`, whose `values` and per-property `sources` identify `builtin`,
`board` (with the stable rule UUID), or a diagnostic `request`. The addressed
board supplies the scope. DRC reports the values and sources it actually used.

`pcb.set_rules` requires `board`, `expected_revision` and `rules`. The latter is
a complete object containing the eight `LIMIT_FIELDS` and three
`PREFERENCE_FIELDS` listed by command discovery, or explicit `null` to remove
authored defaults. The form's **Use built-in defaults** performs that reset.
Changing values retains the saved rule UUID; identical values and repeated resets
do not create history steps. Changes are atomic and undoable.

`pcb.check_design` and `Session::clearance_query_with_rules` accept request-local
hard-limit overrides. On a board with authored defaults, minimums may only
increase and maximums may only decrease; contradictions are refused. Such an
override does not change the saved values or routing preferences. Without authored
defaults, diagnostic thresholds retain the earlier coordinate-range validation,
even when no legal routing dimensions could satisfy them. Saved defaults still
require feasible dimensions. Direct query overrides must be an object containing
recognized hard-limit fields; unknown keys are refused.
The older explicit-clearance query API follows the same resolver.

Omitting dimensions from `pcb.add_tracks` or `pcb.add_via` uses the corresponding
board preferences, including layer-change vias. The GUI initializes its sizes
from those same preferences. An explicit ribbon preset/custom size overrides the
upcoming geometry in that view until the board or its saved rules change;
ordinary edits do not reset that choice. Rule edits invalidate unfinished route
authority, prepared clearance results and previous DRC reports through the
existing session revision. They do not claim to implement background checking.

## Storage

An authored board-default record has `format: "pcbcraft-rules/1"`, an independent
UUID `id`, and the typed `values` object. It lives in `board.json` under board
format 19, which accepts the earlier physical fields including locks. Format 19
requires a valid rule record, and older board headers reject it. Null, missing
or contradictory stored records fail before adoption. Removing the record
returns to the format required by the board's remaining data.

A board without authored rules retains its existing byte encoding. Folder and
`.pc` archive saves preserve rule IDs and exact values; undo/redo restores the
same intent. The legacy diagnostic single-document encoder refuses to discard
rules. This slice does not change the project, board-folder or archive layout.

Net classes, persistent net assignments, relationship constraints and fabrication
profiles remain under #77 and its follow-on issues. This board-default resolver
provides a shared consumer boundary without assigning rules by display names or
claiming impedance, current-capacity or manufacturing verification.
