# Geometry kernel and STEP library

`crates/geometry` (`pcbcraft-geometry`) is the one place PCBcraft builds solid
bodies, reads and writes STEP (ISO 10303-21) files and measures them. It is the
shared base for 3D board inspection and STEP assembly export
([#86](https://github.com/tangency-labs/pcbcraft/issues/86)). This page records
which kernel it uses, why, how the dependency was qualified and what is still
open. The board substrate adapter uses this crate; no command, panel or saved
format uses it yet.

## Choice

The kernel is SolidCraft's `solidcore`, a Rust port of the SMLib and SM_API parts
of NVIDIA's USD BRep. PCBcraft uses these crates from it:

| Crate | Job |
| --- | --- |
| `solidcore` | B-rep solids (already in `Cargo.lock` through the Craft UI crates) |
| `solidcore-ops` | Modelling calls on kernel bodies: primitives, copies and transforms, record conversion, bounding boxes, assembly placement matrices, the session's panic guard |
| `solidcore-record` | The product structure of a STEP file: parts, assemblies, occurrences and their placements, in millimetres |
| `solidcore-step` | STEP read (AP203, AP214 and AP242 geometry and topology, units) and AP242 write with products, names and colours |
| `solidcore-tess` | Tessellation of a derived substrate body; already pinned through the Craft toolkit |

They come from `https://github.com/tangency-labs/solidcraft.git` at
`73bdb6cdd39efc3899fb54e7217289a630a89efe`, the revision the workspace already
pins for the Craft UI crates. `solidcore` is developed in that repository's
`solidcore/` folder; `tangency-labs/solidcore` is a read-only mirror of it.
Taking the crates from the mirror would put a second copy of the kernel in the
build with types that do not match, and would need another `allow-git` entry in
`deny.toml`.

The workspace applies a narrow local `solidcore-ops` backport for STEP transform
composition, described in [its patch record](../vendor/solidcore-ops/PATCH.md).
The three matrix products follow the already-merged upstream correction while
the remaining kernel and toolkit sources stay at the qualified revision. Only
geometry's dependency closure uses this ops layer; it introduces no UI change or second
kernel copy. The non-conformant transformation-item detection from the upstream
PR is outside this backport.

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

`pcbcraft-geometry` is the only crate that directly depends on `solidcore-ops`,
`solidcore-record` or `solidcore-step`. `pcbcraft-engine` has no kernel dependency, so engine builds,
engine tests and the browser editor do not pay for the kernel. The same API
could later move into an engine module if that proves simpler.

- **Kernel-neutral API.** No kernel type appears in the public API. Callers use
  kernel-neutral types such as `Body`, `BoundingBox`, `Transform`, `Placement`,
  `StepRead`, `RefusedItem`, `ErrorCode` and `GeometryError`, so a
  kernel swap touches only this crate.
- **Units.** Lengths are whole nanometres (`i64`), within
  `pcbcraft_units::MAX_LENGTH_NM` of zero. The kernel's millimetre floats are
  converted at the boundary, each coordinate rounded once to the nearest
  nanometre.
- **Panic guard.** Every kernel operation the crate calls (make, measure, read,
  write) runs under a guard adapted from SolidCraft's kernel boundary. When
  panics unwind, the guard turns a panic into `kernel_internal`; aborting panics
  cannot be caught. With unwinding, the reader refuses only the solid whose
  conversion panicked and keeps the others (no test reaches this: no valid input
  makes that conversion panic). `kernel_crash_test()` exercises a caught panic
  when unwinding, returns `operation_failed` without panicking when the kernel
  is present and panics abort, and returns `kernel_unavailable` without a kernel.
  Dropping a body frees its kernel data outside the guard.
- **Lints.** The crate forbids `unsafe` and denies `unwrap`, `expect` and `panic`
  outside tests.
- **Kernel availability.** Native builds always have the kernel. On `wasm32`
  it is opt-in via `wasm-kernel`. Without it, no body can be made and every
  operation that reaches the kernel returns `kernel_unavailable`; arguments
  refused before the kernel keep their codes. CI runs these kernel-less tests on
  `wasm32-wasip1`. Its separate `wasm32-unknown-unknown` checks are compile-only,
  with and without the kernel.
- **No kernel data in projects.** A stored 3D model is its original STEP bytes by
  content hash, as the [3D model storage](library-sources.md#3d-model-storage)
  design proposes, or the dimensions a simple body was built from. Bodies, boxes
  and meshes are derived and rebuilt on load, so a kernel change never breaks a
  saved file.

### Board substrate

`pcbcraft-board-solid` builds a read-only solid from one addressed board, without
linking the kernel into `pcbcraft-engine`. It uses the centre lines of Board
Outline drawings, mapped into the board/1 frame described below. Open paths can join only at exactly
coincident endpoints. Nested contours alternate material and void regardless of
winding; crossings, touches and unfinished contours are refused. Round through
pad and via drills are removed at their resolved board centres, using drill
rather than copper-land diameters. Circular footprint non-plated holes are removed
at their resolved board centres using their finished diameters. This does not
infer slots, which the board document cannot yet represent.

The solid is in the shared board/1 frame: the origin at board zero, +X along
board X, +Y along board −Y, and Z = 0 at the finished top surface. The body spans
−t..0, where t is the stackup's declared nominal finished thickness, not its
minimum or the sum of stock plies. If there is no finished thickness
declaration, the result carries the explicit nominal fallback of 1.6 mm. The solid and its optional triangle mesh are derived memory objects;
no document, undo history, file format, viewport or STEP output changes. On a
build without the optional browser kernel, valid substrate inputs return
`kernel_unavailable`.

### Later integrations

This crate takes bytes, dimensions and transforms; it does not open an external
application, own a project revision or choose a transport. A later live
SolidCraft or FieldCraft connection can pass a model snapshot through the same
STEP boundary without making that connection a prerequisite for standalone
inspection or editing. Simulation results and their provenance belong to that
integration, not to a kernel body.

A parametric external board outline can likewise remain authored source outside
this crate, with a derived geometry snapshot and its transform passed here for
measurement. There is no outline editor or parametric evaluator here. Applying
an imported outline to a board must go through the board's shared command and
validation path; reading or measuring the snapshot does not author it. Neither
live integration, simulation nor parametric outline import is implemented by
this change.

Current operations:

| Function | Result |
| --- | --- |
| `Body::cuboid(name, origin_nm, size_nm)` | An axis-aligned block |
| `Body::bounding_box()` | The exact box of the body's curves and surfaces, in nanometres |
| `read_step(bytes)` | Every solid and surface model of a file in its stated units, and each solid item refused with its entity number and reason |
| `write_step(part_name, bodies)` | One AP242 part; the same bodies always give the same bytes (no time stamp) |
| `Body::transformed(transform)` | A moved copy; the body itself is unchanged |
| `StepRead::placements` | Every use of a body in the file's world frame: one entry for each body placed by an assembly occurrence (a single-body part used twice is one body placed twice), and each otherwise-unplaced body placed once where it is |
| `StepRead::placed_bodies()`, `StepRead::bounding_box()` | The placed bodies, and their box in the world frame, each body measured after it is moved |
| `kernel_crash_test()` | `kernel_internal` from a deliberate unwinding kernel panic; `operation_failed` without panicking when panics abort; `kernel_unavailable` without a kernel |

Error codes are stable snake_case names that MCP and CLI replies can carry:

| Code | Meaning |
| --- | --- |
| `invalid_params` | An argument is invalid or is refused by the kernel, including a name with an ASCII control character, which a STEP string cannot carry |
| `model_unreadable` | Not UTF-8 text, not an ISO 10303-21 exchange structure, a malformed solid, a malformed product structure (a cycle, a broken reference, nesting deeper than 32 combined occurrence/mapping levels, or an expansion past 100,000 placed bodies or its input-scaled work budget), or a placement that is not finite or moves a body more than 1 km from the origin |
| `model_unsupported_entity` | An entity or construct the reader does not support yet |
| `operation_failed` | The kernel could not complete an operation on valid input, or the crash test was refused because panics abort |
| `kernel_unavailable` | This build has no kernel (`wasm32` without `wasm-kernel`) |
| `kernel_internal` | The kernel panicked; the panic was caught |

## Qualification

At SolidCraft revision `73bdb6c`. The table describes the qualification checks
and the outcomes they require; measured samples retain their stated scope.
Unless a target is named, runtime checks use Linux, not native Windows, macOS
or a browser runtime.

| Check | Required outcome or recorded measurement |
| --- | --- |
| Licences, from each crate's manifest | `solidcore`, `solidcore-ops`, `solidcore-step`, `solidcore-convert` and `solidcore-record` are Apache-2.0 (the `solidcore/` workspace licence). Their other dependencies (serde, serde_json, thiserror, log and their proc-macro crates) are MIT OR Apache-2.0, with `memchr` Unlicense OR MIT, `zmij` MIT and `unicode-ident` (MIT OR Apache-2.0) AND Unicode-3.0. |
| `cargo deny --locked check licenses sources bans` (cargo-deny 0.20.2, `deny.toml` unchanged) | bans ok, licenses ok, sources ok |
| Packages introduced by the initial kernel qualification | `pcbcraft-geometry`, `solidcore-ops`, `solidcore-step`; no new crates.io package |
| Dependency closure of `pcbcraft-geometry` | 20 packages |
| Dependency closure of `pcbcraft-engine` | no `solidcore` crate |
| Round trip of a synthetic block written by the kernel as AP242 and read back | bounding box within 1 µm, name kept |
| Two bodies in one part | both read back by name, in the order written |
| A surface model listing its closed shells out of entity order, beside an open shell | the shells read back in ascending entity-number order |
| Body names with an apostrophe, a backslash, non-ASCII characters and leading or trailing spaces | read back unchanged |
| An empty body name; part names `board.STEP`, `board.stp.STEP`, `board.step.stp`, `.step`, blank | the body reads back with its part's name; the part name loses its trailing `.step` suffixes and then its trailing `.stp` suffixes (any case), so `board.step.stp` becomes `board.step`; a part name left empty or blank is written as `Part` |
| 1 mm blocks touching each end of the coordinate range (±`MAX_LENGTH_NM`) | accepted, box read back exactly; one nanometre past either end refused |
| Writing the same bodies twice | identical bytes, with an empty time stamp in the header |
| The crate on `wasm32-wasip1` (no kernel) | making, reading and the crash test give `kernel_unavailable`; refused arguments keep `invalid_params` and `model_unreadable` |
| Browser-target `cargo check` of `pcbcraft-geometry`, without default features and repeated with `--features wasm-kernel` | The conditional `web` job requires both checks to pass when it runs. This is a requirement, not a recorded result; neither check links or executes a browser module. |
| Deliberate kernel panic, and a panic outside the kernel's own guard, in an unwinding build | `kernel_internal`; the kernel keeps working |
| A kernel failure while making a body's STEP record (injected: no valid body fails there) | a caught kernel panic stays `kernel_internal`; a record the kernel refuses is `operation_failed` |
| Non-UTF-8 bytes, text that is not STEP, an empty file | `model_unreadable` |
| A solid whose surfaces use an unsupported entity, beside a valid solid | that solid refused with `model_unsupported_entity`, its own entity number and the surface's name; the other read and placed |
| Non-positive sizes, coordinates out of range, names with control characters, nothing to write | `invalid_params` |
| A transform that flattens, an axis that is not finite, a move out of range | `invalid_params` |
| A file placing a part exactly 1 km out, and 1 nm further | read; `model_unreadable` |
| Thirteen synthetic STEP fixtures, read, written as one part of placed bodies and read again | every bounding box within 1 µm of the box worked out from the generator's dimensions |
| Inch and metre units | converted to nanometres within 1 µm |
| A part used twice, turned a quarter the second time | one body, two placements |
| A translated child under a quarter-turned parent; mapped descendants under a rotated/translated parent | child-local transforms apply before inherited world transforms; placement matrices and placed-body boxes match analytic expectations |
| A mapped solid with origin at x=5 mm and a quarter-turned target | the map origin lands at target zero; its inverse applies before the target and the world box matches the independent fixture oracle |
| Two placed surface models sharing one closed shell, each beside its own open shell | three bodies, four placements: the shell placed by each model, nothing left at the origin |
| A small mapped-item tree that would place a block 2^20 times; exactly 100,000 placements; 100,001 | `model_unreadable` before anything is expanded; read; `model_unreadable` |
| An expansion at its input-scaled work budget (1,233,000 steps for the synthetic file), and of one more | read; `model_unreadable` |
| 2^16 mapped placements, with no other relationship and with 10,000 unrelated relationships; 1,024 placements and 1,000 relationships at their 2,211,000-step budget, and one more | read; `model_unreadable` before anything is expanded; read; `model_unreadable` |
| Repeated mapped items with duplicate plain links to their target's companion | the companion list is counted once per target; placements keep their multiplicity and full link-scan charges retain the exact 2,173,000-step work boundary and first refusal |
| 50,000 mapped uses and 50,000 related links, below the placement cap but over the work budget | `model_unreadable` with the work-budget reason; pre-count companion iterations do not multiply by the uses |
| Alternating mapped targets and plain relationship companions at 32 and 33 levels; a 2,048-level acyclic route | read; `model_unreadable` from pre-count before dependency expansion; `model_unreadable` |
| A shared mixed tail cached shallowly then reached deeper; combined occurrence/mapping paths | accepted at combined depth 32; `model_unreadable` at 33 |
| 1,000 distinct products with the writer's two representations and two relationships per part, sharing one synthetic B-rep | one body in 1,000 placements; a small reader regression, not the body-heavy board measurement |
| Fixture construction and reader payloads with LF and CRLF endings | the intended records are inserted; identical counts and world boxes |
| A surface model of 20,000 empty closed shells beside the block, both placed 32,768 times through mapped items | read in under 30 s: one body, 32,768 placements, the model refused |
| Original qualification sample: 500 part instances of 16 pins, each part instance its own product (8,500 bodies, 21.1 MB) | read without error or refused item in 406 s, peak resident memory 944 MiB (debug build, Linux); historical measurement, not a timing bound |

The automated checks are the unit tests in `crates/geometry/src/` and
the integration tests in `crates/geometry/tests/`.
The [placement-order review evidence](../tools/performance/baselines/issue387-review/README.md)
retains the three before-fix failures, dependency/source audit and bounded board smoke.

### STEP fixtures

`crates/geometry/tests/fixtures/step` holds the qualification set, 113,540 bytes including `expected.json`
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
| `assembly_nested_rotated.step` | A translated child inside a quarter-turned parent |
| `mapped_offset_origin.step` | A non-zero representation-map origin placed at a rotated target |
| `assembly_mapped_rotated.step` | A mapped translation inside a rotated and translated parent |
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
builds and reads a board whose part instances are each their own product with a
package and its own pins: a body-heavy import case with no shared parts. By default it
builds 2 part instances of 16 pins so a debug test run takes a few seconds. The
figures above are the original Linux debug measurement at the full size, not a
claim that every read has the same time or memory cost:

```sh
PCBCRAFT_STEP_BOARD_PART_INSTANCES=500 PCBCRAFT_STEP_BOARD_PINS=16 \
    cargo test -p pcbcraft-geometry --test step_board
```

For this measured debug-board workload, nearly all read time was the kernel
building bodies from their records (about 50 ms a body); parsing the file and
walking its assembly took well under a second. Release-build read times have
not been measured.

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
  exceeds its work budget, or nests deeper than 32 combined occurrence/mapping
  levels, is refused as a whole. The work budget is 1,000,000 steps plus 1,000
  steps per parsed STEP entity, independent of line endings or repeated item
  references in one entity. The input allowance admits ordinary flat products
  while bounding work amplification linearly in the input entity count; the
  placement limit remains independent. A step is each part visited, each representation it
  reaches, each representation walked and each of its items, and each
  representation relationship in the file once for each mapped item walked and
  once for each representation a part reaches (the expansion scans every
  relationship then). All those scans count, including unrelated links; the
  high placement ceiling does not promise acceptance of every assembly below it.
  The reader counts the expansion from the product graph and checks the actual
  mixed-edge depth, including cached tails reached deeper, before it expands anything, so a small file that would place a body millions
  of times through mapped items, or make the expansion scan thousands of
  relationships for each of them, is refused without being expanded. Which
  bodies an item gives (a surface model can give one per closed shell) is
  worked out once for each item, not again at each of its placements. A large
  board exported with one body for each pin can come near the 100,000-body
  figure. The original measured board needed about 110 KiB for each placed body
  (944 MiB for 8,500); extrapolating that same construction to 100,000 bodies
  gives around 11 GB. This is not a measured limit or a scaling law for other
  files; shared geometry and body complexity change the cost.
- **Writing assemblies.** `write_step` writes one part. A placed read is written
  as one body per placement, so a part used twice is written twice; writing the
  product structure back is left to STEP assembly export.
- **Qualification set.** The fixtures cover planes, cylinders, holes, assemblies
  and three units. Cones, spheres, tori, B-spline surfaces, mapped items and
  bodies with voids are read by the kernel but not in the set yet.
- **Browser.** The `wasm32-unknown-unknown` checks do not link a browser module
  or run it. `apps/pcbcraft-web` leaves `wasm-kernel` off. Before enabling it:
  - Account for panics that abort. This is the default on `wasm32`; the guard
    cannot turn an abort into a `GeometryError`. A panic in a wasm call traps
    instead. With the kernel enabled and panics aborting, `kernel_crash_test()`
    returns `operation_failed` without inducing a panic.
  - Qualify the stack size. SolidCraft's web app at `73bdb6c` links with
    `-zstack-size=8388608` (8 MiB) in `apps/solidcraft-web/build.rs`. This change
    does not set that flag in PCBcraft.
  - Run a kernel-backed STEP round trip in a wasm runtime and measure the size
    the kernel adds to the linked web build. Neither is covered by the compile
    checks or by the kernel-less WASI tests.
- **Input size.** `read_step` takes the whole STEP text in memory and has no size
  limit; its bodies and refusals grow with the file. The caller (the later STEP
  import command) bounds the input.
- **Size.** A probe outside this workspace measured about 9.5 MB added to a
  native release binary for a full kernel round trip. The browser size is not
  measured; the kernel should load there only when a 3D view or STEP action
  needs it.
- **Distance and assemblies.** Body distance, assembly export and a 3D view
  are not implemented here. The substrate tessellation supplies geometry only.
