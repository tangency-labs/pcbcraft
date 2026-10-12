# Attribution

| Component | Source | Revision / license | Use |
|---|---|---|---|
| craft-ui | https://github.com/storytold/craft-libs | 1aa81a773aa9198a0e4c0daeda954d622475af38; MIT OR Apache-2.0 | Shared egui tab and widget components, Cargo dependency |
| SolidCraft architecture | https://github.com/bherbruck/solidcraft | README / AGENTS consulted; MIT OR Apache-2.0 | Architectural reference; no implementation copied |
| KiCad board format | https://dev-docs.kicad.org/en/file-formats/sexpr-pcb/ | Public format documentation | Original limited board writer |
| KiCad library formats | https://dev-docs.kicad.org/en/file-formats/ | Public format documentation | Original Rust symbol/footprint readers |
| KiCad library fixtures | https://gitlab.com/kicad/libraries/kicad-symbols and https://gitlab.com/kicad/libraries/kicad-footprints | Ubuntu packages 10.0.6~ubuntu24.04.1; CC-BY-SA 4.0 + KiCad design-output exception | Extracted R/LED/LM2904/LM358 symbols and unmodified R_0805 footprint; community contributors; exact provenance/checksums and modifications in docs/kicad-libraries.md; original license in crates/library/tests/fixtures/LICENSE-kicad-libraries.md |

No KiCad implementation code has been copied or ported in this initial implementation.
PCBCraft sample circuits are original; imported symbol/footprint artwork retains
its KiCad data license and design-output exception. Dependencies retain their own licenses.
