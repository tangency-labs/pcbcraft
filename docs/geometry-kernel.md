# Geometry kernel and STEP library

`crates/geometry` (`pcbcraft-geometry`) is the one place PCBcraft builds solid
bodies, reads and writes STEP (ISO 10303-21) files and measures them. It is the
shared base for 3D board inspection and STEP assembly export
([#86](https://github.com/tangency-labs/pcbcraft/issues/86)). This page records
which kernel it uses, why, how the dependency was qualified and what is still
open. No command, panel or saved format uses the crate yet.

## Choice

The kernel is SolidCraft's `solidcore`, a Rust port of the SMLib and SM_API parts
of NVIDIA's USD BRep. PCBcraft uses four crates from it:

| Crate | Job |
| --- | --- |
| `solidcore` | B-rep solids (already in `Cargo.lock` through the Craft UI crates) |
| `solidcore-ops` | Modelling calls on kernel bodies: primitives, copies and transforms, record conversion, bounding boxes, assembly placement matrices, the session's panic guard |
| `solidcore-record` | The product structure of a STEP file: parts, assemblies, occurrences and their placements, in millimetres |
| `solidcore-step` | STEP read (AP203, AP214 and AP242 geometry and topology, units) and AP242 write with products, names and colours |

They come from `https://github.com/tangency-labs/solidcraft.git` at
`73bdb6cdd39efc3899fb54e7217289a630a89efe`, the revision the workspace already
pins for the Craft UI crates. `solidcore` is developed in that repository's
`solidcore/` folder; `tangency-labs/solidcore` is a read-only mirror of it.
Taking the crates from the mirror would put a second copy of the kernel in the
build with types that do not match, and would need another `allow-git` entry in
`deny.toml`.

The pin moves only with the workspace's SolidCraft revision. A bump is its own
change that re-runs the qualification below and lists the kernel changes since
the previous pin. When the kernel crates are published on crates.io, they move
to exact crates.io versions.

Alternatives considered:

- `truck-stepio` 0.3.0 and `truck-modeling` 0.6.0 (Apache-2.0, crates.io) are the
  fallback kernel. They read geometry and topology only: no products, assembly
  links, names, colours or units, which PCBcraft would then have to write itself.
- `parry3d` (Apache-2.0) is the candidate for distance and overlap checks on
  meshes where an exact B-rep distance is not available. It is not a dependency
  yet. `solidcore-ops` also has boundary distance between bodies, which is not
  qualified yet.
- Open CASCADE and its bindings are LGPL and are excluded by the licence rules in
  [AGENTS.md](../AGENTS.md#license-and-sources-the-rule-that-cant-be-undone).

## Boundary

`pcbcraft-geometry` is the only crate that depends on `solidcore-ops` or
`solidcore-step`. `pcbcraft-engine` has no kernel dependency, so engine builds,
engine tests and the browser editor do not pay for the kernel. The same API
could later move into an engine module if that proves simpler.

- **Kernel-neutral API.** No kernel type appears in the public API. Callers use
  `Body`, `BoundingBox`, `Transform`, `Placement`, `StepRead`, `RefusedItem` and
  `GeometryError`, so a
  kernel swap touches only this crate.
- **Units.** Lengths are whole nanometres (`i64`), within
  `pcbcraft_units::MAX_LENGTH_NM` of zero. The kernel's millimetre floats are
  converted at the boundary, each coordinate rounded once to the nearest
  nanometre.
- **Panic guard.** Every kernel operation the crate calls (make, measure, read,
  write) runs under a guard that turns a panic into
  the error `kernel_internal`, adapted from SolidCraft's kernel boundary. The
  reader refuses only the solid whose conversion panicked and keeps the others.
  `kernel_crash_test()` lets a diagnostics command prove the guard works.
  Dropping a body frees its kernel data outside the guard.
- **Lints.** The crate forbids `unsafe` and denies `unwrap`, `expect` and `panic`
  outside tests.
- **Native only.** The kernel crates are target dependencies for
  non-`wasm32` builds. On `wasm32` the crate compiles without them, no body can
  be made, and every operation that reaches the kernel returns
  `kernel_unavailable`; arguments refused before the kernel keep their codes. CI
  runs these tests on `wasm32-wasip1`.
- **No kernel data in projects.** A stored 3D model is its original STEP bytes by
  content hash, as the [3D model storage](library-sources.md#3d-model-storage)
  design proposes, or the dimensions a simple body was built from. Bodies, boxes
  and meshes are derived and rebuilt on load, so a kernel change never breaks a
  saved file.

Current operations:

| Function | Result |
| --- | --- |
| `Body::cuboid(name, origin_nm, size_nm)` | An axis-aligned block |
| `Body::bounding_box()` | The exact box of the body's curves and surfaces, in nanometres |
| `read_step(bytes)` | Every solid and surface model of a file in its stated units, and each solid item refused with its entity number and reason |
| `write_step(part_name, bodies)` | One AP242 part; the same bodies always give the same bytes (no time stamp) |
| `Body::transformed(transform)` | A moved copy; the body itself is unchanged |
| `StepRead::placements` | Every use of a body in the file's world frame: one per assembly occurrence (a part used twice is one body placed twice), and each body outside any product placed once where it is |
| `StepRead::placed_bodies()`, `StepRead::bounding_box()` | The placed bodies, and their box in the world frame, each body measured after it is moved |
| `kernel_crash_test()` | `kernel_internal` from a deliberate kernel panic |

Error codes are stable snake_case names that MCP and CLI replies can carry:

| Code | Meaning |
| --- | --- |
| `invalid_params` | An argument was refused before the kernel saw it, including a name with an ASCII control character, which a STEP string cannot carry |
| `model_unreadable` | Not UTF-8 text, not an ISO 10303-21 exchange structure, a malformed solid, a malformed product structure (a cycle, a broken reference, nesting deeper than 32 levels, or an expansion past 100,000 placed bodies or 1,000,000 steps), or a placement that is not finite or moves a body more than 1 km from the origin |
| `model_unsupported_entity` | An entity or construct the reader does not support yet |
| `operation_failed` | The kernel could not complete an operation on valid input |
| `kernel_unavailable` | This build has no kernel (the browser) |
| `kernel_internal` | The kernel panicked; the panic was caught |

## Qualification

At SolidCraft revision `73bdb6c`:

| Check | Result |
| --- | --- |
| Licences, from each crate's manifest | `solidcore`, `solidcore-ops`, `solidcore-step`, `solidcore-convert` and `solidcore-record` are Apache-2.0 (the `solidcore/` workspace licence). Their other dependencies (serde, serde_json, thiserror, log and their proc-macro crates) are MIT OR Apache-2.0, with `memchr` Unlicense OR MIT, `zmij` MIT and `unicode-ident` (MIT OR Apache-2.0) AND Unicode-3.0. |
| `cargo deny --locked check licenses sources bans` (cargo-deny 0.20.2, `deny.toml` unchanged) | bans ok, licenses ok, sources ok |
| New `Cargo.lock` entries | `pcbcraft-geometry`, `solidcore-ops`, `solidcore-step`; no new crates.io package |
| Dependency closure of `pcbcraft-geometry` | 20 packages |
| Dependency closure of `pcbcraft-engine` | no `solidcore` crate |
| Round trip of a synthetic block written by the kernel as AP242 and read back | bounding box within 1 µm, name kept |
| Two bodies in one part | both read back by name, in the order written |
| A surface model listing its closed shells out of entity order, beside an open shell | the shells read back in ascending entity-number order |
| Body names with an apostrophe, a backslash, non-ASCII characters and leading or trailing spaces | read back unchanged |
| An empty body name; part names `board.STEP`, `board.stp.STEP`, `board.step.stp`, `.step`, blank | the body reads back with its part's name; the part name loses its trailing `.step` suffixes and then its trailing `.stp` suffixes (any case), so `board.step.stp` becomes `board.step`; a part name left empty or blank is written as `Part` |
| 1 mm blocks touching each end of the coordinate range (±`MAX_LENGTH_NM`) | accepted, box read back exactly; one nanometre past either end refused |
| Writing the same bodies twice | identical bytes |
| The crate on `wasm32-wasip1` (no kernel) | making, reading and the crash test give `kernel_unavailable`; refused arguments keep `invalid_params` and `model_unreadable` |
| Deliberate kernel panic, and a panic outside the kernel's own guard | `kernel_internal`; the kernel keeps working |
| A kernel failure while making a body's STEP record (injected: no valid body fails there) | a caught kernel panic stays `kernel_internal`; a record the kernel refuses is `operation_failed` |
| Non-UTF-8 bytes, text that is not STEP, an empty file | `model_unreadable` |
| A solid whose surfaces use an unsupported entity, beside a valid solid | that solid refused with `model_unsupported_entity` and its entity number; the other read and placed |
| Non-positive sizes, coordinates out of range, names with control characters, nothing to write | `invalid_params` |
| A transform that flattens, an axis that is not finite, a move out of range | `invalid_params` |
| A file placing a part exactly 1 km out, and 1 nm further | read; `model_unreadable` |
| Ten synthetic STEP fixtures, read, written as one part of placed bodies and read again | every bounding box within 1 µm of the box worked out from the generator's dimensions |
| Inch and metre units | converted to nanometres within 1 µm |
| A part used twice, turned a quarter the second time | one body, two placements |
| Two placed surface models sharing one closed shell, each beside its own open shell | three bodies, four placements: the shell placed by each model, nothing left at the origin |
| A few hundred bytes of mapped items that would place a block 2^20 times; exactly 100,000 placements; 100,001 | `model_unreadable` before anything is expanded; read; `model_unreadable` |
| An expansion of exactly 1,000,000 steps, and of one more | read; `model_unreadable` |
| 2^16 mapped placements, with no other relationship and with 10,000 unrelated relationships; 64 placements and 7,870 relationships at exactly 1,000,000 steps, and one more | read; `model_unreadable` before anything is expanded; read; `model_unreadable` |
| A board of 500 components of 16 pins, each component its own product (8,500 bodies, 21.1 MB) | read without error or refused item in 406 s, peak resident memory 944 MiB (debug build, Linux) |

The automated checks are the unit tests in `crates/geometry/src/tests.rs` and
the integration tests in `crates/geometry/tests/`.

### STEP fixtures

`crates/geometry/tests/fixtures/step` holds the qualification set, about 90 KB
of STEP text written by `scripts/generate-step-fixtures.py` from the geometry and
topology structures of ISO 10303-42 and the product and assembly structures of
ISO 10303-41, -43 and -44, with no CAD system involved. Running the script again
gives the same files.

| File | What it tries |
| --- | --- |
| `box.step` | A block away from the origin (AP214) |
| `cylinder.step` | A cylinder whose side is one face closed by a seam edge |
| `cylinder_x_axis_ap203.step` | A cylinder along x whose side is bounded by its two circles alone, no seam (AP203) |
| `hole_plate.step` | A 1.6 mm plate with four 3.2 mm holes and one 10 mm hole |
| `assembly.step` | A board and a part, each its own product, placed by an assembly |
| `assembly_shared_part_turned.step` | One part placed twice, the second time turned a quarter about z |
| `box_inch.step` | A block in inches |
| `assembly_inch.step` | The board-and-part assembly in inches |
| `y_up.step` | A board placed with its thickness along y, as y-up systems write it (AP242) |
| `box_metre.step` | A block in metres |

`expected.json` beside them records, for each file, its unit, how many bodies
and placements it holds and the box of the placed bodies in nanometres. The
boxes are worked out by the generator from the dimensions it writes, not
measured by a kernel, so the same file gates any later kernel bump or swap. The
tolerance, 1 µm, is stored in the same file.

### Board-sized read

`crates/geometry/tests/step_board.rs`, adapted from SolidCraft's board fixture,
builds a board whose components are each their own product with a package and
its own pins, the worst case for an import, and reads it back. By default it
builds 2 components of 16 pins so a debug test run takes a few seconds. The
figures above are for the full size:

```sh
PCBCRAFT_STEP_BOARD_COMPONENTS=500 PCBCRAFT_STEP_BOARD_PINS=16 \
    cargo test -p pcbcraft-geometry --test step_board
```

In a debug build nearly all of the read time is the kernel building bodies from
their records (about 50 ms a body); parsing the file and walking its assembly
take well under a second at that size, and the time grows in proportion to the
number of bodies. A release build is much faster; its figures are not recorded
yet.

The kernel crates are Apache-2.0 only, unlike the rest of SolidCraft (MIT OR
Apache-2.0). Apache-2.0 is on the `deny.toml` allow list, so no exception is
needed. `solidcore`'s `NOTICE` is kept unchanged in
[`crates/geometry/SOLIDCORE-NOTICE`](../crates/geometry/SOLIDCORE-NOTICE).
Apache-2.0 requires it to accompany distributed builds, so the release notices
file listed under [Dependencies](../ATTRIBUTION.md#dependencies) must include it.

## Known limits and open work

- **Experimental API.** The `solidcore` README calls its API experimental. The
  kernel stays behind this crate, and each pin bump re-runs the qualification.
- **Not on crates.io.** `solidcore-step`, `solidcore-ops` and `solidcore-convert`
  are marked `publish = false`, and `solidcore-record` is unpublished, so the git
  pin is the only source today.
- **Reader coverage.** Faces whose edges have no closed-form parameter curve on
  their surface are refused with their solid; the kernel can make those curves
  but is not asked to yet. Names of assembly occurrences and colours are not
  read yet, and the writer writes no colours.
- **Assembly size.** A STEP file whose assembly places more than 100,000 bodies,
  takes more than 1,000,000 steps to expand, or nests deeper than 32 levels, is
  refused as a whole. A step is each part visited, each representation it
  reaches, each representation walked and each of its items, and each
  representation relationship in the file once for each mapped item walked and
  once for each representation a part reaches (the expansion scans every
  relationship then). The reader counts the expansion from the product graph
  before it expands anything, so a small file that would place a body millions
  of times through mapped items, or make the expansion scan thousands of
  relationships for each of them, is refused without being expanded. A large
  board exported with one body for each pin can come near the 100,000-body
  figure. Memory grows with
  the bodies: the measured board needed about 110 KiB for each placed body
  (944 MiB for 8,500), so a file near the limit can need around 11 GB.
- **Writing assemblies.** `write_step` writes one part. A placed read is written
  as one body per placement, so a part used twice is written twice; writing the
  product structure back is left to STEP assembly export.
- **Qualification set.** The fixtures cover planes, cylinders, holes, assemblies
  and three units. Cones, spheres, tori, B-spline surfaces, mapped items and
  bodies with voids are read by the kernel but not in the set yet.
- **Browser.** `solidcore` itself checks for `wasm32-unknown-unknown`, but
  `solidcore-step` has not been built for it, and the kernel README asks for a
  wasm stack larger than the default 1 MB. Until that is qualified, every operation
  of the crate on `wasm32` that reaches the kernel returns `kernel_unavailable`,
  and a browser 3D view would use meshes.
- **Input size.** `read_step` takes the whole STEP text in memory and has no size
  limit; its bodies and refusals grow with the file. The caller (the later STEP
  import command) bounds the input.
- **Size.** A probe outside this workspace measured about 9.5 MB added to a
  native release binary for a full kernel round trip. The browser size is not
  measured; the kernel should load there only when a 3D view or STEP action
  needs it.
- **Distance, meshes and assemblies.** Tessellation for the 3D view, body
  distance and assembly export are not in the crate yet. They
  are added as new functions over the same `Body` type without changing the
  error codes or units.
