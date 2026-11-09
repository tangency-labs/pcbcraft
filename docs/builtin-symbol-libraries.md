# Bundled symbol libraries

Default and Power are native libraries distributed with the app. Embedded
loading is selected by `libraries/bundled.txt` and supplies the same `library.json` and symbol documents as a folder or
portable bundle. Both pass through `NativeSource::load` and the normal catalog's
validation, revisions, conflict checks and content digests.

The Default library keeps the stable identity `starter` for existing project
links; its display name is Default. Bundled source registrations are
`bundled-starter` and `bundled-power`. These are session source IDs, not
component identities, and removing a registration leaves pinned instances intact.
Source limits include bundled registrations.

Catalog click, double-click, drag/drop and the Ground ribbon action use ordinary
symbol placement. Each gesture captures the exact definition revision/digest
and an editable initial Value. Power placements have normal components, unit
placements and terminal endpoints. Fields, attributes, properties, G/M/R,
library-copy authoring, deletion and undo follow the component paths.

## Definition format

`pcbcraft-symbol/2` adds two optional fields:

- `behavior`: `{"type":"power","terminal":"1"}` names the specified terminal
  globally using the component's Value. The terminal must exist; drawing,
  library and item names never imply electrical behavior.
- `defaults`: `metadata` and `field_layout`, using the existing component-field
  and unit-appearance schemas. Any definition may provide them. Placed instances
  can override them through normal properties.

Power definitions hide Reference, position Value and initially exclude the
component from BOM, board and position outputs through those defaults. Their
pins and drawings are ordinary native data. Copy authoring and native export
retain the behavior and defaults. Renaming Value changes the global name;
local/global text labels remain independent annotations.

Version 1 remains readable and retains its original encoding when saved. Version
2 is pinned with the project, so it reopens offline and older readers reject the
new semantics instead of silently discarding them. Explicit selections from any
library require a revision and digest. Supplying a revision alone also requires
its digest. Commands omitting library, revision and digest still default to
`starter` and may reuse its existing pinned copy. Provenance-free
older definitions can match an otherwise identical current definition without
weakening conflict checks between complete source definitions.

## Removed marker representation

Power symbols no longer have net-label styles or separate marker/text tools.
The text-label API accepts only `local` and `global`. Former `power`, `power_bar`
and `ground` annotation records are unsupported and rejected by the strict
native decoder; this branch does not migrate them or silently drop their data.
All power definitions and placed instances use the ordinary native library,
component, unit, terminal, field and history paths.

## Validation

Command-built regression circuits cover native folder/bundle/embedded delivery,
custom power definitions with unrelated IDs, Value naming, field defaults,
copy/export, offline pinning, connected/disconnected movement, cancellation,
source conflicts and history. Existing project round-trip tests remain in place.
UI tests cover both catalog presentations, repeated and rapid placement,
drag/drop, the Ground ribbon and ordinary component properties.
