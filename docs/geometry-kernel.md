# Geometry kernel and STEP library

`crates/geometry` (`pcbcraft-geometry`) is the one place PCBcraft builds solid
bodies, reads and writes STEP (ISO 10303-21) files and measures them. It is the
shared base for 3D board inspection and STEP assembly export
([#86](https://github.com/tangency-labs/pcbcraft/issues/86)). This page records
which kernel it uses, why, how the dependency was qualified and what is still
open. No command, panel or saved format uses the crate yet.

## Choice

The kernel is SolidCraft's `solidcore`, a Rust port of the SMLib and SM_API parts
of NVIDIA's USD BRep. PCBcraft uses three crates from it:

| Crate | Job |
| --- | --- |
| `solidcore` | B-rep solids (already in `Cargo.lock` through the Craft UI crates) |
| `solidcore-ops` | Modelling calls on kernel bodies: primitives, record conversion, bounding boxes, the session's panic guard |
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
  `Body`, `BoundingBox`, `StepRead`, `RefusedItem` and `GeometryError`, so a
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
| `kernel_crash_test()` | `kernel_internal` from a deliberate kernel panic |

Error codes are stable snake_case names that MCP and CLI replies can carry:

| Code | Meaning |
| --- | --- |
| `invalid_params` | An argument was refused before the kernel saw it, including a name with an ASCII control character, which a STEP string cannot carry |
| `model_unreadable` | Not UTF-8 text, not an ISO 10303-21 exchange structure, or a malformed solid |
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
| Body names with an apostrophe, a backslash, non-ASCII characters and leading or trailing spaces | read back unchanged |
| An empty body name; part names `board.STEP`, `board.stp.STEP`, `board.step.stp`, `.step`, blank | the body reads back with its part's name; the part name loses its trailing `.step` suffixes and then its trailing `.stp` suffixes (any case), so `board.step.stp` becomes `board.step`; a part name left empty or blank is written as `Part` |
| 1 mm blocks touching each end of the coordinate range (±`MAX_LENGTH_NM`) | accepted, box read back exactly; one nanometre past either end refused |
| Writing the same bodies twice | identical bytes |
| The crate on `wasm32-wasip1` (no kernel) | making, reading and the crash test give `kernel_unavailable`; refused arguments keep `invalid_params` and `model_unreadable` |
| Deliberate kernel panic, and a panic outside the kernel's own guard | `kernel_internal`; the kernel keeps working |
| Non-UTF-8 bytes, text that is not STEP, an empty file | `model_unreadable` |
| A solid whose surfaces use an unsupported entity | that solid refused with `model_unsupported_entity` and its entity number |
| Non-positive sizes, coordinates out of range, names with control characters, nothing to write | `invalid_params` |

The automated checks are the unit tests in `crates/geometry/src/tests.rs`.

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
  but is not asked to yet. Assembly placements, names of assembly occurrences
  and colours are not read into bodies yet, and the writer writes no colours.
- **Qualification set.** A set of synthetic STEP fixtures (cylinders, holes,
  assemblies, inch units, Y-up files) with expected bounding boxes in a
  kernel-neutral file is still to come, so every later kernel bump or swap passes
  the same gate.
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
  distance, assembly export and placement types are not in the crate yet. They
  are added as new functions over the same `Body` type without changing the
  error codes or units.
