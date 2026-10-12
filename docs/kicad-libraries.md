# KiCad library reading

`pcbcraft-library` is a pure Rust reader for `.kicad_sym` symbol libraries and
`.kicad_mod` footprints. The parsers work on strings, have no filesystem or UI
dependency, and compile for WebAssembly. The engine owns file access, catalogs,
search and placement policy.

```rust
let symbols = pcbcraft_library::read_symbols(&source)?;
let footprint = pcbcraft_library::read_footprint(&source)?;
```

Symbols preserve arbitrary string pin numbers, names, electrical types, unit and
body-style indices, lengths and pin directions. The reader resolves `extends`
references within the same library, detects missing parents and inheritance
cycles, and applies child property overrides. Common graphics use unit `0`;
standard and alternate body drawings retain their body-style indices. Local
child units replace the corresponding inherited unit/body drawing. Rectangles,
polylines, circles, three-point arcs and text are parsed.

Symbol points are converted from KiCad's y-up coordinates to PCBCraft's y-down
coordinates. Pin and text angles retain KiCad's counterclockwise degree values.
For a symbol pin, the inner end is `position + (length*cos(angle),
-length*sin(angle))`. Footprint points already use y-down coordinates; footprint
angles also retain KiCad counterclockwise values.

Footprints retain pad numbers, types, shapes, positions, rotations, layer lists,
size, roundrect ratios, circular/oval drills and drill offsets. Copper pad
primitives for custom/chamfered shapes are explicitly reported as unsupported.
Line, rectangle, polygon, circle, arc and text graphics carry their layers.
The reader does not translate 3D models or execute any file reference.

`Graphic::path_points()` supplies a shared display approximation for the GUI,
CPU previews and vector export. Lines, rectangles and polylines retain exact
vertices; circles and three-point arcs produce at most 129 points. Arc paths pass
through the given midpoint and handle collinear points as a three-point path.
Invalid definitions and text produce empty paths; renderers handle text separately.

Each definition preserves its raw source node and each pad preserves its raw
source. Unsupported expressions, alternate pin functions and non-line pin styles
are reported; metadata such as properties and
3D model references is preserved in raw source without inventing graphics.
Inherited geometry is resolved in the DTO; a derived symbol's raw source is only
its own original node, so reconstituting an entire source library requires
retaining the parent definitions too. This is a read/import foundation, not a
lossless KiCad library writer or a complete symbol/footprint editor.

## Bounds and validation

The parser caps source bytes at 32 MiB, expressions at 1,000,000, nesting at 64,
and atoms at 256 KiB. A library permits at most 20,000 symbols; inheritance depth
is at most 64 and expanded output has a conservative 128 MiB budget. Each
definition caps pins/pads and graphics at 20,000, graphics points at 10,000,
unsupported entries at 10,000 and raw node source at 8 MiB. Coordinates and
angles must be finite and bounded. Dimensions and metadata are validated.

Public `SymbolDefinition::validate`, `FootprintDefinition::validate` and
`Graphic::validate` also validate definitions deserialized from native project
data. Pin identifiers must be unique within each unit/body style. Footprints may
legitimately have multiple pads with the same electrical number; safe initial
placement policy can be more restrictive than the reader.

## Reference fixtures and provenance

The tests include library data from the KiCad community, not KiCad C++ engine
code. Sources were read from installed Ubuntu packages
`kicad-library-symbols` and `kicad-library-footprints`, both version
`10.0.6~ubuntu24.04.1`, packaged on 2026-08-28.

| Fixture | Original installed source | Modification |
|---|---|---|
| `device.kicad_sym` | `/usr/share/kicad/symbols/Device.kicad_sym` | Extracted `R` and `LED` nodes into a minimal library wrapper |
| `lm358.kicad_sym` | `/usr/share/kicad/symbols/Amplifier_Operational.kicad_sym` | Extracted `LM2904` and its derived `LM358` into a minimal wrapper |
| `R_0805_2012Metric.kicad_mod` | `/usr/share/kicad/footprints/Resistor_SMD.pretty/R_0805_2012Metric.kicad_mod` | Unmodified |

Original source SHA-256 checksums:

```text
Device.kicad_sym: f65c9dd13ef986c482165e5c8237ccf4f691bc7321868cf05e01dafbc1326b24
Amplifier_Operational.kicad_sym: 70d51935f1e2563cf203a3d1b95657f166e140b357d3a8570f70f855a3dcd74e
R_0805_2012Metric.kicad_mod: f050b6a50cc7e6f86291fb7aa8f87b0d02255e21cfbddf291cdccc87e5889794
```

Upstream collections: [KiCad symbols](https://gitlab.com/kicad/libraries/kicad-symbols)
and [KiCad footprints](https://gitlab.com/kicad/libraries/kicad-footprints).
The data is copyright its KiCad community contributors and is licensed under
[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/legalcode) with
KiCad's design-output exception. The unmodified upstream license document is
included at `crates/library/tests/fixtures/LICENSE-kicad-libraries.md`.
These test fixtures retain that license independently of PCBCraft's GPL code.
The exception permits use of library data in electronic designs without forcing
the design's license; redistributing library collections retains CC BY-SA
attribution and share-alike requirements.

Tests cover resistor/LED graphics, LM358 inheritance and multi-unit pins, 0805
pad shapes and layers, arbitrary pin identifiers, drill metadata, unsupported
geometry, malformed/deep input and hostile deserialized definitions. An optional
installed-library test reads all symbols in Device and Amplifier_Operational:

```sh
cargo test -p pcbcraft-library -- --include-ignored
```

The optional test requires the two source files at `/usr/share/kicad/symbols`.
