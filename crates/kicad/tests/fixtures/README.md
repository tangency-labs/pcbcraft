# KiCad test fixtures

Every file here was written by hand for PCBCraft from KiCad's published file format
documentation (<https://dev-docs.kicad.org/en/file-formats/>). None was produced by KiCad,
copied or trimmed from a third-party design, or derived from KiCad's source or tests. Their
identities are made-up, patterned UUIDs.

| File | What it exercises |
| --- | --- |
| `divider.kicad_sch` | KiCad 8 format: two resistors, `+3V3` and `GND` power symbols, a junction, local and global labels, sheet text, and a two-unit symbol with common pins and a no-connect marker |
