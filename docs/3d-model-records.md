# 3D model records, bindings and frames

**Status: proposed contract.** Nothing on this page is implemented yet. No
command, panel, codec or saved project uses a model record. This page settles the
shared contract for [#86](https://github.com/tangency-labs/pcbcraft/issues/86)
so that later slices can build on it: the standalone 3D view, STEP export, the
board solid and `board/1` for the Craft bridge
([#70](https://github.com/tangency-labs/pcbcraft/issues/70),
[RFC #444, section 6.7](https://github.com/tangency-labs/pcbcraft/pull/444)).
It was agreed in [the #86 discussion](https://github.com/tangency-labs/pcbcraft/issues/86#issuecomment-7015453468).

It builds on the [3D model storage](library-sources.md#3d-model-storage) design,
the [library record rules](part-schema.md#exact-references), the
[footprint coordinates](footprint-schema.md#coordinates-and-generated-reference)
and the [geometry kernel](geometry-kernel.md). The machine-readable record schema
is [model-v1.schema.json](schemas/model-v1.schema.json).

## Scope

| In this contract | Not in this contract |
| --- | --- |
| The model record: identity, geometry by hash, source frame, licence | Rendering, the 3D view and its controls |
| The binding from a package variant to a model, or to no body | STEP export and `board/1` payload fields beyond the frames |
| The model frame, the board frame and the component pose | The board solid (built by [#465](https://github.com/tangency-labs/pcbcraft/pull/465)) |
| Link and embed storage, and the states of a missing model | Lead maps, materials and appearance; formats other than STEP |

## The model record

A model record is a library record of its own kind, `pcbcraft-model/1`. It
describes one geometry file and nothing about where that geometry is placed.

| Field | Meaning |
| --- | --- |
| `format`, `library`, `id`, `revision`, `name`, `license` | Common identity fields with the [existing rules](part-schema.md#exact-references) |
| `source` | Required provenance (`kind`, `reference`, `locator`); descriptive, never fetched on load |
| `redistribution` | `allowed`, `not_allowed` or `unknown`; decides which operations may copy the bytes into a new file ([Redistribution](#redistribution)) |
| `geometry` | `kind` (`step`), `sha256` of the exact file bytes as 64 lowercase hex digits, and `size_bytes` |
| `source_frame` | Optional correction from the file's own frame to the model frame; omitted when it is the identity |

- **Identity.** The record is addressed by `(library, id, revision)`, never by a
  file name, path or display name. Its geometry is addressed by its hash.
- **Revisions.** Any change to stored content, including new geometry bytes, a
  new licence or a new source frame, makes a new revision. Conflicting content
  under one identity and revision is quarantined as for other records.
- **Size.** `size_bytes` is 1 to 67,108,864 (64 MiB).
- **No derived data.** Kernel bodies, meshes, bounding boxes and tessellations
  are rebuilt from the bytes and are never stored, so a kernel change cannot
  break a saved file.
- **STEP only at first.** `geometry.kind` has one value, `step`, read through
  [`pcbcraft-geometry`](geometry-kernel.md). Other formats need a new kind and a
  qualified reader.

### Source frame

The source frame describes the file, so it is the same for every package that
uses the model. It brings the file's geometry into the model frame:

```json
"source_frame": {
  "up_axis": "y",
  "origin_offset": { "x_nm": 0, "y_nm": 0, "z_nm": -350000 }
}
```

| Field | Values | Meaning |
| --- | --- | --- |
| `up_axis` | `z` (default) or `y` | `y` turns the file's +Y up into +Z: `U = Rx(+90°)`, so a file point `(x, y, z)` becomes `(x, −z, y)` |
| `origin_offset` | Nanometre point, each axis within ±100,000,000 | Added after `U`; moves the file's origin to the model origin |

The correction is `F = T(origin_offset) · U`. An omitted field is the identity;
the canonical writer omits `up_axis: "z"` and a zero offset, and omits
`source_frame` when both are absent. The schema describes this canonical form,
so it accepts only `y` for `up_axis` and only a nonzero offset.

There is no unit and no scale factor. A STEP file declares its own length unit,
and the reader converts it to nanometres. A file whose declared unit is wrong is
imported again as a new revision rather than scaled.

### Redistribution

An operation that copies a model's bytes into a new file copies them only when
the record says `allowed`. This covers **Embed used models**, `board/1`
publication for the Craft bridge and STEP export.

- **`not_allowed` and `unknown`** models are left out, even when their bytes are
  available from an installed library or the cache. The operation still
  completes and reports each model it left out, with
  `redistribution: not_allowed` or `redistribution: unknown` as the reason.
- **Embed** leaves such a model linked. Its record is still pinned, so the
  project keeps its identity and alignment.
- **`board/1` and STEP export** list such a component with its pose and the
  reason, and give it no body.
- **No override.** Someone with the right to redistribute an `unknown` model
  makes a new record revision that says `allowed`, so the decision is recorded
  with the model's provenance.

Viewing and checks read the bytes in place and are not affected.

## Binding to a package variant

A model binds to a **package variant** of a part definition, not to the part as a
whole and not to a footprint: one footprint serves many parts with different
bodies, and one part can have several packages. The binding is an optional
`model` field on each `packages.<variant>` record, which needs a new part format,
`pcbcraft-part/5`. The part schema and codec arrive with the implementation
slice.

Format 5 extends [format 4](part-schema.md#formats-and-implementation): every
package may carry a `model`, and `terminal_ratings` becomes optional, still
nonempty when present. A writer selects the lowest format that holds the
content:

| Content | Format written |
| --- | --- |
| No binding and no supply range | 2, or 1 for a retained format-1 record |
| Supply ranges and no binding | 4 |
| A binding on any package, `no_body` included, with or without supply ranges | 5 |

- **Removing the last binding** writes format 4 if supply ranges remain and
  format 2 otherwise. Removing the last supply range from a part that still has
  a binding keeps format 5.
- **Readers.** Formats 1, 2 and 4 stay readable. Earlier readers refuse format 5
  rather than open the part without its bindings.
- **Format 3** is never written and stays refused.

```json
"model": {
  "kind": "library",
  "definition": { "library": "basic-components", "id": "sot-23", "revision": 1 },
  "alignment": {
    "offset": { "x_nm": 0, "y_nm": 0, "z_nm": 0 },
    "rotation_mdeg": { "x": 0, "y": 0, "z": 90000 }
  }
}
```

```json
"model": { "kind": "no_body" }
```

- **`library`** names an exact model revision and its alignment.
- **`no_body`** says the package has no body on purpose, such as a fiducial, a
  test pad or a solder jumper. It is complete, not a coverage gap.
- **No `model` field** leaves the package unbound, which is reported as a gap.
- **Alignment** belongs to the binding, because one body can need a different
  placement on a different footprint. `offset` is in nanometres, each axis
  within ±100,000,000. `rotation_mdeg` holds whole millidegrees from 0 to
  359,999 on each axis. A zero `offset`, a zero `rotation_mdeg` and an
  all-zero `alignment` are omitted on canonical save.
- **Changes.** Adding, changing or clearing a binding makes a new part revision.
  Projects adopt it explicitly, as for any part revision.
- **Part form.** The binding fills the model slot that
  [#309](https://github.com/tangency-labs/pcbcraft/issues/309) reserves on each
  package row.

A per-manufacturer-part override (the same footprint with a different body for
each MPN) and models for symbol-only instances with a directly assigned footprint
are later work. Until then such an instance is reported as unbound.

## Frames

All frames are right-handed. Stored lengths are whole nanometres and stored
angles whole millidegrees. Millimetres and floating point appear only at the
boundaries: user input, STEP export and `board/1`.

### Board frame

The `board/1` frame used by the board solid and the Craft bridge:

- the origin is the board's coordinate zero, not the outline's corner or the
  placement origin;
- +X is board X;
- +Y is board −Y, because board Y points down in PCBcraft;
- +Z points away from the board on the top side;
- Z = 0 is the finished top outer surface, so the board spans −t to 0 for
  finished thickness t.

### Model frame

- The origin is the footprint origin on the mounting surface.
- +X is footprint +X.
- +Y is footprint −Y, because footprint Y points down.
- +Z points away from the board on the component's side.
- Z = 0 is the seating plane.

### Rotation

A rotation `(a, b, c)` in millidegrees is intrinsic X, then Y, then Z:

```text
R(a, b, c) = Rx(a) · Ry(b) · Rz(c)        (column vectors, p' = R · p)
```

This is the same rotation as Z, then Y, then X about fixed axes. It is not X,
then Y, then Z about fixed axes.

### Component pose

A placed footprint at board position `(x, y)`, with rotation `r` whole degrees
counterclockwise as seen from the top, places its model in the board frame with:

```text
P = T(x, −y, 0) · Rz(r) · S · A · F
A = T(offset) · R(rotation_mdeg)
S = I                         on the top side
S = T(0, 0, −t) · Ry(180°)    on the bottom side
```

- `F` is the record's source frame correction and `A` the binding's alignment.
- **Bottom side.** PCBcraft's 2D bottom placement mirrors local X before the
  rotation, so in 3D the body is given a half turn about Y,
  `(x, y, z) → (−x, y, −z)`, and seated on the bottom surface. It is turned
  over, never mirrored with a negative scale. A half turn about X would differ
  by 180° about Z and put an asymmetric part the wrong way round on its pads.
- **Nothing absolute is stored.** The pose is computed from the ordinary
  placement data, so a model follows its footprint when the footprint moves,
  turns or changes side, whatever caused the change.
- **Thickness.** `t` is the finished board thickness from the stackup. The
  binding never depends on it. A consumer that needs `t` when none is declared
  says so; it does not stand in a default for a mechanical result.
- **`board/1`** carries each composed pose as a 4×4 matrix in millimetres plus
  the inputs it came from, as RFC #444 section 6.7 describes. Receivers place
  with the matrix.

## Storage

The [3D model storage](library-sources.md#3d-model-storage) design chooses
between linking and embedding. This contract fixes where each piece lives.

In a native library folder:

```text
models/<id>.r<revision>.json      # the model record
models/assets/<sha256>.step       # the exact bytes
```

In a project:

```text
library/models/<library>/<id>.r<revision>.json   # always pinned
assets/<sha256>.step                             # Embed only
```

- **Pinning.** A project pins every model record that a pinned part's packages
  bind, as it pins every package footprint today. The record is small and always
  saved, in both modes.
- **Link** (the default) saves only the record. The bytes come from an installed
  library or a local cache, found by hash.
- **Embed** stores the bytes once per hash under `assets/`, and the `.pc`
  manifest's reserved `assets` list names each one with its path, size and
  SHA-256.
- **Mode.** A project-level default with an override for each model. Switching
  mode keeps the record identity and the alignment. Caching a linked model never
  changes the mode or marks the project changed.
- **Integrity.** Bytes whose SHA-256 or size differ from the record are rejected,
  never substituted.
- **Git.** An unchanged save changes no bytes. Asset files are immutable; they
  only appear or disappear.

### Packed projects

Today a [`.pc` manifest](native-project-storage.md#packed-projects-and-manifests)
must have an empty `assets` list, and any missing file or digest mismatch fails
the open. The storage slice adds asset support in a new container format,
`pcbcraft-container/4`, and keeps the two kinds of entry apart:

- **Documents stay strict.** Every `parts` entry, model records included, keeps
  the current rules, and an unlisted file still fails the open.
- **Model assets are optional.** A listed asset whose file is missing gives its
  components `geometry_unavailable`. One whose size or SHA-256 differs from the
  manifest or the record gives `hash_mismatch`, and its bytes are not used. The
  project opens and stays editable, and the open notice names each failed asset.
- **Saving.** Bytes that failed are never written back. The record and the
  binding stay, the asset and its manifest entry are left out, and the save
  notice names them, so the model stays `geometry_unavailable` until it is
  embedded again.

How embedded assets count against the packed-project size limits is settled by
the storage slice.

## Model states

A missing or unusable model never blocks opening, electrical editing, checks or
saving. Each placed footprint reports exactly one state, through GUI and MCP.
The states are checked in this order and the first that applies is reported, so
they are mutually exclusive:

| State | Meaning |
| --- | --- |
| `no_body` | The package has no body on purpose |
| `unbound` | No binding, including symbol-only instances |
| `record_missing` | The bound record is not pinned or cannot be read |
| `geometry_unavailable` | The record is readable but its bytes are missing: in Link mode from every installed library and the cache, in Embed mode from the project's `assets/` |
| `hash_mismatch` | Bytes are present but their SHA-256 or size differs |
| `kernel_unavailable` | The bytes match, but this build has no geometry kernel to read them, as in the browser |
| `unreadable` | The bytes match, but the STEP reader refused the file or at least one solid in it; the report names each refused entity |
| `bound` | The bytes match and the reader accepted every solid: the geometry is usable |

A partial read is never used: if any solid is refused, the model is
`unreadable` and contributes no body.

| Consumer | Behaviour |
| --- | --- |
| 3D view | Draws `bound` bodies only. Every other component is listed in the coverage report with its state and drawn without a body |
| STEP export, `board/1` | Write `bound` bodies, subject to [redistribution](#redistribution). Every other component is listed with its pose and state and gets no body. A `hash_mismatch` refuses the whole output, as RFC #444 section 6.7 describes for `board/1`, and leaves any previous file in place |
| Without a kernel | STEP export and `board/1` refuse the whole output with `kernel_unavailable` rather than write a file without bodies |

An embedded project reopens offline with all its models. A linked project reopens
offline with reduced 3D coverage and everything else intact. No invented box or
height stands in for a missing model in a mechanical output.

## Acceptance for the implementation slices

- **Frames.** A bottom-side SOT-23 places pin 1's lead on pad 1. An asymmetric
  part at 90° and 30° matches its pads on both sides. Quarter turns compose
  exactly.
- **Rotation order.** `(90°, 0, 90°)` gives `Rx(90°) · Rz(90°)`, distinct from the
  fixed-axes reading.
- **Source frame.** A Y-up fixture with an origin offset seats on Z = 0 at the
  footprint origin.
- **Part formats.** A part with supply ranges and a bound model round-trips as
  format 5. Removing its binding writes format 4, then removing its ranges
  writes format 2. A format-4 reader refuses the format-5 file.
- **Storage.** Link and embed round trips reopen offline; switching mode keeps
  identity and alignment; an unchanged save is byte-identical; tampered bytes
  report `hash_mismatch`.
- **Packed projects.** A `.pc` with a listed model asset missing opens editable
  and reports `geometry_unavailable`. One with an asset whose bytes differ opens
  editable and reports `hash_mismatch`. In both cases the notice names the
  asset, and the next save leaves it out.
- **States.** Each state above is produced by a command-built fixture, and none
  blocks electrical editing. A file with one refused solid beside a valid one
  reports `unreadable`.
- **Redistribution.** Embed and `board/1` copy an `allowed` model, and leave out
  a `not_allowed` and an `unknown` model with the reason, even when their bytes
  are cached.

## Delivery order

1. This contract and the model record schema.
2. Engine codecs for `pcbcraft-model/1` and `pcbcraft-part/5`, and native library
   loading of `models/`.
3. Project pinning, link and embed, `pcbcraft-container/4` manifest assets and hash checks.
4. Model import through a shared command and the part form slot.

The 3D view, STEP export and `board/1` follow from these.
