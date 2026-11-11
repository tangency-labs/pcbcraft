# Footprint definition and reference placement schema

This specification defines footprint records and reference placement within the
library model merged in [PR #221](https://github.com/tangency-labs/pcbcraft/pull/221).
It specifies the native footprint fields, generated reference defaults and board
placement overrides. The schemas describe canonical saved JSON. Implementations
must also perform the semantic checks below when accepting definitions and
transactions.

Board format 15 adds local reference position and print overrides. Earlier board
formats retain their existing encodings when these fields are absent. Footprint
library definitions continue to use their independently versioned format 1.

| Document | Format | Machine-readable schema |
| --- | --- | --- |
| Published footprint definition | `pcbcraft-footprint/1` | [Footprint schema](schemas/footprint-v1.schema.json) |
| Physical placement inside a board | Enclosing `pcbcraft-board/15` | [Placement schema](schemas/footprint-placement-v15.schema.json) |

The schemas use [JSON Schema Draft 2020-12](https://json-schema.org/draft/2020-12/json-schema-core).
Register both schemas locally by their `$id` URNs when validating placements.
Their references resolve from those registrations, with no network lookup.
Application validation belongs in the shared command and storage codecs. These
schema documents are also available to library tooling independently of the
application.

With Python 3.10 or later and `jsonschema` 4.18 or later installed, run the schema
check from the repository root. Version 4.18 introduced the registry API used for
offline reference resolution. It checks both schemas and the bundled footprint
definitions.
Additional arguments accept standalone native definitions, and `--placement`
accepts one physical placement record.

```sh
python3 scripts/check-footprint-schemas.py
```

Run the focused checker regressions with the following command. The cross-runtime
pattern check also uses Node.js when available.

```sh
python3 -B -m unittest discover -s scripts -p 'test_footprint_schemas.py'
```

## Ownership and identity

A footprint owns pads, physical artwork and a generated reference's default
position and font. A component instance owns its reference text, such as `R12`.
A board placement owns its origin, orientation, side, terminal-to-pad mapping
and optional reference displacement. Library previews substitute `REF` for the
component's text.

The immutable definition identity is `(library, footprint kind, id, revision)`.
The `revision` is a positive unsigned 32-bit content revision. An editor change
allocates a new content revision, while a fork allocates a new item identity.
Conflicting content with the same identity and revision is refused. Projects
pin exact definition copies under
`library/footprints/<library>/<id>.r<revision>.json` and reopen offline from them.

The format version identifies the readable field contract. An item's revision
identifies its content, and a library package release identifies a collection.
Changing a label position on one board advances that project's editing history.
Changing the library default advances the definition's content revision.

## Published definition fields

Objects are closed to unknown properties. Required arrays remain present even
when empty. Optional empty arrays and absent optional fields are omitted by the
canonical writer. JSON `null` is outside this canonical schema. Existing readers
also accept `null` for some optional Rust fields as a compatibility input.

| Field | Type and constraint | Meaning |
| --- | --- | --- |
| `format` | Exact string `pcbcraft-footprint/1` | Native footprint field contract |
| `library`, `id` | 1–64 lowercase ASCII letters, digits and internal hyphens | Stable library and item identity |
| `revision` | Integer 1…4,294,967,295 | Immutable content revision |
| `name`, `license` | Nonempty trimmed strings, at most 200 UTF-8 bytes each | Display name and license declaration |
| `source` | Required `kind`, `reference`, `locator` strings | Provenance, preserved when pinned |
| `pads` | 1…128 distinct numbered pads | Physical lands and optional plated drills |
| `graphics` | 0…64 rectangles | Fabrication, silkscreen or courtyard geometry |
| `artwork` | Optional 1…128 parameterized objects | Lines, rectangles, circles, arcs, polylines or stroke text |
| `silkscreen` | Optional 1…64 centerline paths | Physical paths clipped against pad clearance |
| `reference` | Optional `pcbcraft-stroke-1` or `pcbcraft-technical-1` | Enable generated reference text using this font |
| `reference_offset` | Optional nonzero local point | Displace the generated reference from its automatic anchor |

Source strings are nonempty, trimmed and free of control characters. Their UTF-8
byte limits are 64 for `kind`, 2,048 for `reference` and 1,024 for `locator`.
The locator is descriptive provenance. Loading a definition performs no fetch.

Pad numbers and mapping identifiers retain their spelling, including `01`.
They contain 1…8 ASCII letters, digits, `+`, `-` or `_`. The
[footprint guide](footprint-library.md#definition-and-placement-contract)
defines the existing land shapes, copper bindings and drill relationships.
Parameterized artwork uses the existing shared drawing geometry, with physical
stroke width and `fill` equal to `none`.

## Coordinates and generated reference

Every persisted length is an integer number of nanometers. Points contain
exactly `x_nm` and `y_nm`. Unit-aware controls convert millimeters or mils to this
representation before submitting a command. Geometry uses the footprint's local
origin and the existing native X/Y axes, with positive Y pointing down. Rotation
uses whole degrees counterclockwise, with 90 degrees mapping local `(x, y)` to
`(y, -x)`.

Local coordinates and dimensions are bounded by 100,000,000 nm. Each reference
offset axis is within ±100,000,000 nm. These are current implementation bounds.
The automatic anchor, library displacement and board displacement have distinct
meanings.

| Value | Omitted behavior | Saved meaning |
| --- | --- | --- |
| Definition `reference` | Physical generated reference disabled | Font used for the component's actual reference |
| Definition `reference_offset` | Zero displacement | Library default adjustment from the automatic anchor |
| Placement `reference_offset` | Zero displacement | Additional adjustment from that library default |

A definition offset requires an enabled `reference`. Zero displacements are
represented by omitting the field. Resetting a board reference submits a zero
offset through the command, which removes the stored override. Disabling the
library reference clears its default offset in the same authoring transaction.

The automatic anchor centers the complete reference lettering horizontally.
Format 1 estimates the upper pad extent as the minimum of
`center.y_nm - max(width_nm, height_nm) / 2` across pads. The path extent is the
minimum silkscreen point Y minus half the stroke width. The lower edge of the
lettering ink sits one requested clearance above the smaller of those extents.
The renderer then adds the library displacement and the placement displacement.
It clips the resulting strokes against pad clearance before transforming them
into board coordinates. Font fallback, physical text height, stroke width and
clearance use the existing
[silkscreen contract](footprint-library.md#silkscreen-settings).

For a local reference point `p`, library displacement `d`, placement displacement
`o` and footprint origin `t`, its board position is
`t + rotate(mirror_x_if_bottom(p + d + o))`. Bottom placement mirrors local X
before rotation. Quarter turns preserve exact integer coordinates. Other
whole-degree rotations round the transformed local coordinates to nanometers.
Moving, rotating or flipping a footprint carries its reference with it.

## Local printed text

The optional placement `reference_text` object contains sparse print overrides.
Its fields are optional and non-null, and at least one field must be present.
An absent field inherits its default. An explicit value remains an override even
when it currently matches that default. Reset removes the override.

| Field | Type and constraint | Inherited default |
| --- | --- | --- |
| `text` | Nonblank single-line string of at most 128 Unicode characters, without controls | The owning component's current reference |
| `font` | `pcbcraft-technical-1` or `pcbcraft-stroke-1` | The pinned footprint's reference font, with Technical available for explicitly enabled local ink |
| `size` | Required integer `height_nm` and `stroke_width_nm` within this optional object. Height is positive and at most 10 mm. Stroke is positive and at most half the height. | The board's silkscreen size and stroke settings |
| `rotation` | Whole degrees 0…359, counterclockwise around the local text anchor | Zero, before the footprint's own transform |
| `align` | `start`, `middle` or `end` | `middle` |
| `visible` | Boolean | Enabled when the pinned definition has a generated reference |

Height and stroke form one size override so an inherited stroke cannot become
invalid when a local height or global style changes. Pad clearance continues to
come from the board's silkscreen settings. Alignment and local rotation apply to
the lettering at its bottom-ink anchor, before the library and placement offsets,
pad clipping and footprint transform.

A local text literal is used for rendered and exported ink. The schematic,
component reference, BOM identity and terminal mappings continue to use the
owning component. For example, printing `Power input` on R2 leaves its component
reference R2. An explicit **Rename component** action uses the existing shared
reference edit with uniqueness validation, updating that component across its
schematic and board views. The component's stable ID and local literal survive
that rename. A literal `R2` remains literal even when the component becomes R12.
Removing the `text` override restores the current component reference.

The Properties card shows the component reference alongside local print controls.
**Use component reference** removes the literal override while retaining other
appearance settings. **Reset silkscreen** removes all local print overrides and
the placement offset. Reference selection has its own edit target and uses the
owner only for correspondence, layer-colored emphasis and existing unrouted connections.
Delete hides the selected reference's ink, and R rotates its local text. Selecting
the footprint body provides **Edit silkscreen text** for references whose ink is
hidden. Text selection never grants physical-footprint edit authority.

Board canvas rendering and Gerber silkscreen export use the same generated and
clipped paths. The editor-only fallback label on a retained marker footprint
continues to use its placement offset through the legacy rendering path.

## Physical placement fields

The placement schema validates one physical `footprints` array entry inside a
board document. It includes the exact pinned identity, numeric object and
component IDs, explicit terminal-to-pad map, board-space origin and optional
`rotation`, `bottom`, `reference_offset` and `reference_text`. A zero rotation and front-side
placement are omitted on canonical save. Origin coordinates currently range
from −987,300,000 to 987,300,000 nm on each axis.

The containing board supplies its stable board identity and format. Format 15
requires at least one nonzero `reference_offset` or nonempty `reference_text`
override among its footprints. Other placements may omit both. Format 15 extends
format 14 and retains every field
permitted by format 14, including optional tracks and vias inherited from
formats 12–13. Bottom-side placements are accepted in both formats 14 and 15.
Removing the last offset and print override restores the format required by the board's other
data. Earlier board formats reject both fields. The
[native storage contract](native-project-storage.md) defines the remaining board
envelope and its legacy encodings. The physical placement schema covers the
library-backed representation; retained marker placements have their existing
legacy identity and `pin_1_pad` rules in the engine codec.

`pcb.set_footprint_reference_offset` takes the board target, `footprint`,
`expected_revision` and an `offset` point. GUI and MCP use this transaction,
including reset. Unknown targets, stale revisions and out-of-range values are
rejected before mutation. Undo restores the previous override and saved format.

`pcb.set_footprint_reference_text` takes the board target, `footprint`,
`expected_revision`, an `appearance` object and optional `offset`. The appearance
uses the `reference_text` fields above. A null appearance resets all print
overrides. Omitting `offset` preserves position, and a zero offset removes its
override. The whole edit validates and commits as one transaction. Physical print
appearance requires a native footprint definition. The legacy marker's editor
annotation retains its earlier offset-only behavior.

## Semantic validation

JSON Schema checks object shapes, required fields, enumerations and numeric
bounds. Implementations must additionally check relationships that depend on
other fields or project state.

- Pad numbers are distinct. A rounded corner is at most half the shorter land
  dimension, a circular land has equal dimensions, and a plated drill is smaller
  than both land dimensions.
- Drawing extents after tessellation stay within definition bounds. Lines have
  distinct endpoints, rectangles have ordered corners, and polyline segments
  have distinct consecutive points. Text is nonblank and control-free.
- Metadata meets the UTF-8 byte limits and trimming rules. JSON Schema's string
  lengths count Unicode characters, so its bounds alone cannot establish this.
- Placement IDs and component references resolve in the owning board/project.
  The pinned library/item/revision exists, and its mapping is complete and
  bijective under the current codec. PR #221's terminal-to-multiple-pads mapping
  needs a later format and command change.
- The board format agrees with the fields present. Retained legacy forms and
  no-op save bytes retain their existing compatibility behavior.

Editable authoring documents use `pcbcraft-library-item/1`, with stable
primitive IDs, draft state and revision allocation. A draft may have no pads.
Export validates a complete published definition against the footprint contract.
The published schema therefore applies after export. Reopening the draft retains
the default reference offset and subsequent exports retain its new revision.

## Compatibility and versioning

The `reference_offset` extension follows the existing format-1 optional
field convention. Earlier binaries reject an unknown field. They cannot read
definitions carrying this extension. Definitions with no new data keep their
existing bytes on no-op save. The format-15 board extension provides an explicit
version for placed overrides.

The format-1 field set defined here is fixed. Future authored fields or changed
meanings require a new footprint format and an explicit compatibility design.
The board-local appearance fields above extend the first format-15 implementation
before its release. Later changes to the released board contract require their
own version and compatibility review.

Offsets are additive. Adopting a definition revision with a changed default
preserves the placement's displacement and recomputes its position. A definition
update must preview that label movement along with the physical geometry before
applying the update as one undoable transaction. Keeping the absolute board
position during such an update requires an explicit choice that recalculates the
stored displacement.
