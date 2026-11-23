# KiCad test fixtures

`divider.kicad_sch` was written by hand for PCBCraft from KiCad's published file
format documentation (<https://dev-docs.kicad.org/en/file-formats/>). It was not
produced by KiCad, copied or trimmed from a third-party design, or derived from
KiCad's source or tests. Its identities are made-up, patterned UUIDs.

The KiCad 10 files derive from that divider (as of commit `3a6acbd`) and use its
original artwork. KiCad 10.0.8 (`Eeschema 10.0.8-unknown-1000.20261030gitf38aee7.fc44`)
was run as a black box to save them, as file format `20260306`, and to export
each `.xml` netlist with `kicad-cli sch export netlist --format kicadxml`. The
exception is `jumper-control.kicad_sch`, written by hand as `jumper-kicad10.kicad_sch`
without its jumper group and with its two wires and label in compact form; KiCad
didn't save it, but its netlist was still exported by KiCad.
travle1 made them for the review of
[#503](https://github.com/tangency-labs/pcbcraft/pull/503#issuecomment-7094236070)
(archive `pcbcraft-pr503-kicad10-fixtures.zip`, SHA-256
`462514ff01de906846772b908d0b3f24cb445141fefe13e633c34b2b2a9f0067`). They are
committed byte for byte. Keep each schematic and its netlist together, since the
tests compare the import with KiCad's nets.

| File | What it exercises |
| --- | --- |
| `divider.kicad_sch` | KiCad 8 format: two resistors, `+3V3` and `GND` power symbols, a junction, local and global labels, sheet text, and a two-unit symbol with common pins and a no-connect marker |
| `divider-kicad10.kicad_sch`, `.xml` | The divider resaved by KiCad 10. KiCad's named nets are `+3V3`, `/VOUT`, `GND` and `IN` |
| `exclusions-kicad10.kicad_sch`, `.xml` | One resistor, R1, placed with `(exclude_from_sim yes)` and `(in_pos_files no)` while its library symbol keeps the defaults. KiCad's netlist marks R1 `exclude_from_pos_files` |
| `jumper-kicad10.kicad_sch`, `.xml` | A part whose library symbol has `(jumper_pin_groups ("1" "2"))`, with separate wire stubs on its two pins and the label `JUMPER_BUS` on pin 1 only. KiCad puts J1.1 and J1.2 on `/JUMPER_BUS` |
| `jumper-control.kicad_sch`, `.xml` | The same without the jumper group: KiCad puts J1.2 on a net of its own |
