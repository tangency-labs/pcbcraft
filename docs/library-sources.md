# Library sources and the parts interface

November 6, 2026 · **Proposal for discussion.** Nothing here is decided until it
has been discussed; push back on any of it. It extends issue
[#28](https://github.com/tangency-labs/pcbcraft/issues/28), which holds the
broader library requirements, the symbol format proposal and the community
threads behind them.

The Parts card is meant to become the interface to part resources: symbols,
footprints and parts, from local and remote sources, including KiCad libraries
and vendor downloads. This document proposes the model behind that interface,
so each later slice builds on the same terms.

## What exists today

- **One built-in source.** The starter library in `libraries/starter` is embedded
  at build time (see [symbol-library.md](symbol-library.md)). It holds symbols and a bounded set of original
  [SMD footprint definitions](footprint-library.md).
- **Symbols are pinned into projects.** Each project keeps a full copy of every
  symbol it uses at `library/symbols/<library>/<symbol>.r<revision>.json`, so it
  opens without the library (see [native-project-storage.md](native-project-storage.md)).
- **Pinned physical footprints.** `libraries/starter/footprints` supplies rounded
  0402/0603 resistor/capacitor, 0402 LED, SOT-23 and plated two-pin header definitions. The shared placement
  command pins definitions and validates explicit terminal mappings. The legacy
  `generic-through-hole-2` remains a marker-only compatibility definition; source
  registration remains proposed. A shared schematic/PCB chooser now assigns exact starter or pinned packages and explicit mappings without placing them, then places those assignments separately.
- **The Parts card** lists the built-in symbols with search and three tile sizes,
  at a fixed 260 × 490 points, floating or docked. Clicking or dragging places a
  symbol through `schematic.place_symbol`.

## Goals

- Find and use symbols, footprints and parts from any connected source in one place.
- Read KiCad libraries, so a person's existing libraries and the KiCad files that
  vendors such as SnapMagic and Ultra Librarian distribute work without a
  separate plugin. VISION.md names this as the most common request.
- Keep every project openable offline, with the exact definitions it uses.
- Record where every definition came from, its revision and its license.
- Keep library actions in the shared command set, so agents can search, place
  and pin through MCP exactly as the GUI does.
- Keep saved files friendly to Git: stable identities, no timestamps or caches.

## Three kinds of library item

| Kind | Holds | Never holds | Used in |
| --- | --- | --- | --- |
| **Symbol** | Schematic drawing, terminals and units | Footprints, pads | Schematic |
| **Footprint** | Pads, outline, courtyard, silkscreen, paste and mask | Symbols, terminals | PCB layout |
| **Part** | A symbol and a footprint by identity, the terminal-to-pad mapping, and fields such as value and manufacturer part number | Drawings or pads of its own | Both |

Symbols and footprints are stored and versioned independently. One 0805
footprint can serve every resistor and capacitor part, and a team can share a
footprint-only library across projects. A part is the only place the two meet,
so the mapping, and its validation for package variants such as FET pinouts,
lives on the part.

This supports both workflows from #28:

- **Generic first.** Place a symbol, then assign a footprint during layout. The
  assignment creates the mapping, and a mismatch must be resolved before it applies.
- **Specific first.** Place a part, which pins its symbol, footprint and mapping together.

Each of the three kinds is optional until needed, as VISION.md proposes. 3D
models and supplier data attach to parts in later work.

## Sources, libraries and items

A **source** is somewhere items come from. It contains one or more **libraries**,
and each library contains items of any of the three kinds. A library folder may
hold `symbols/`, `footprints/` and `parts/`, or only some of them, so
symbol-only and footprint-only libraries are both ordinary libraries.

| Source | Reads | Status |
| --- | --- | --- |
| Built-in starter | Embedded native library | Exists (symbols and starter SMD footprints) |
| Native folder | A folder with `library.json` | Proposed, [#92](https://github.com/tangency-labs/pcbcraft/issues/92) |
| KiCad library | `.kicad_sym` files and `.pretty` footprint folders | Proposed below |
| Vendor download | A zip of KiCad files from a part provider | Proposed below |
| Git | A native library at a pinned commit | [#93](https://github.com/tangency-labs/pcbcraft/issues/93) |
| Remote provider or team catalog | An API or database | [#94](https://github.com/tangency-labs/pcbcraft/issues/94) |

Every source adapter answers the same questions: which libraries and items it
has, the full definition of an item, and why it is unavailable or why an item is
invalid. One bad item never hides the rest of its library, as the starter loader
already does.

## Identity and revisions

An item is addressed by its library ID, its kind, its item ID and its revision.
A path or URL only says where to find a library; moving or renaming a folder must
not change what a project refers to. Two definitions claiming the same identity
and revision with different content are reported as a conflict, never chosen
silently.

Native libraries carry an explicit revision on each item. KiCad files have none,
so the adapter derives one from a hash of the item's normalized content. Any
change to a KiCad symbol then reads as a new revision rather than silently
changing projects that pinned the old one.

## Registering sources

A registration tells PCBcraft where a source is. As KiCad does with its global
and project library tables, this proposes both:

- **Per person,** kept with the person's preferences: libraries they use in every project.
- **Per project,** kept in the project folder: libraries a project's team expects
  to share. The file format is open; it must stay small, sorted and free of
  machine-specific absolute paths where possible.

A project must open even when none of its registered sources is available.
Removing a registration never removes definitions a project has already pinned.

## Pinning into a project

Pinning copies the exact definition into the project with its provenance: source
kind, library ID, item ID, revision, license and the locator it was read from.
Symbols and starter footprints are pinned today; parts would use the same
layout:

```text
library/
  symbols/<library>/<symbol>.r<revision>.json
  footprints/<library>/<footprint>.r<revision>.json    implemented
  parts/<library>/<part>.r<revision>.json              proposed
```

Opening a project never upgrades a pinned definition. Updating is a deliberate
step: preview the newer revision against the project's usage, then apply it as
one undoable command (#28). A project-local variant is a pinned copy the project
edits, leaving the shared definition untouched, as in the custom nRF9151
footprint example in #28.

## KiCad libraries

The KiCad adapter would read symbol libraries (`.kicad_sym`) and footprint
libraries (`.pretty` folders of `.kicad_mod` files). It reads them where they are
and converts an item into PCBcraft's native definitions only when it is pinned.
That keeps a person's KiCad libraries as the single copy and avoids copying
licensed data into PCBcraft.

- **Clean room.** The adapter is written only from KiCad's published file-format
  documentation ([S-expression format](https://dev-docs.kicad.org/en/file-formats/sexpr-intro/),
  [symbol libraries](https://dev-docs.kicad.org/en/file-formats/sexpr-symbol-lib/),
  [footprints](https://dev-docs.kicad.org/en/file-formats/sexpr-footprint/)),
  the [user manual](https://docs.kicad.org/9.0/en/eeschema/eeschema.html) and
  black-box tests against KiCad. Never from KiCad source, headers, patches or
  source-derived fixtures (AGENTS.md). PCBcraft Next's KiCad code stays out: its
  source-exposure record and pause still apply.
- **Refuse rather than approximate.** A KiCad feature the native model can't
  represent yet is reported on that item, which stays unpinnable. PCBcraft never
  guesses at a partial conversion.
- **Licenses travel with the data.** KiCad's official libraries have
  [their own license terms](https://www.kicad.org/libraries/license/), and vendor
  parts have theirs. PCBcraft records the license on every pinned item and does
  not bundle third-party libraries by default.

## Vendor downloads

Part providers commonly distribute a part as a zip containing KiCad symbol,
footprint and 3D files. Adding such a zip would make it a library: either a
personal one or one stored with the project. Its items are then read through the
KiCad adapter, pinned with the provider's terms recorded, and searchable like
any other source. This replaces the hand-made import workflow described in VISION.md.

## Remote providers and team catalogs

Network sources run in the app on background workers, never in the engine, and
report progress, failure and staleness in the Parts card. Search caches stay per
person and outside projects, and they follow each provider's terms. #28 notes
that Mouser's Search API restricts storing results. Availability and pricing are
a separate capability from the saved design (#28, #94).

## The parts interface

The Parts card becomes the front end to all of this:

- **Sources** with their status: available, unavailable and why, item counts and problems.
- **Search across sources,** filtered by kind. The Schematic workspace offers parts
  and symbols; PCB layout offers footprints and parts.
- **Results** labelled with kind, source and revision, so two similar items from
  different libraries are never confused.
- **Details** for the selected item: symbol and footprint previews, terminals and
  pads, the mapping, license and provenance, and validation errors.
- **Actions:** Place, Add to project (pin without placing) and, later, Edit.

Every action that changes a project is an engine command. Read-only actions,
such as listing sources and searching, are commands too, so MCP can drive the
same workflow.

Creating and editing items belongs to dedicated **Symbol editor** and
**Footprint editor** workspaces
([#78](https://github.com/tangency-labs/pcbcraft/issues/78),
[#79](https://github.com/tangency-labs/pcbcraft/issues/79)). A workspace provides
the tools; the document it edits, a shared library item or a project's pinned
copy, owns its own save and undo. Windows from the
[PCB workspace plan](board-workspace-plan.md) host those workspaces, so an
editor can open beside the schematic or in its own window.

## Proposed order

1. **Model and native folders** (#92): the source adapter interface, per-person
   registration and local native libraries in the Parts card.
2. **KiCad symbol libraries:** read live, converted and pinned with provenance.
3. **Footprint libraries and assignment** (#92, after the PCB canvas in
   [#54](https://github.com/tangency-labs/pcbcraft/issues/54)), including KiCad
   footprints and parts with validated mappings.
4. **Vendor downloads.**
5. **The fuller Parts card** described above, once per-window panels from the
   current multi-window work have settled.
6. **Symbol and footprint editor workspaces** (#78, #79), once views can target
   documents other than the open project.
7. **Git, remote providers and team catalogs** (#93, #94).

## Open questions

- **KiCad libraries: live or imported?** This proposes reading them live and
  converting at pin time. Importing up front would allow editing them as native
  libraries, at the cost of a second copy.
- **Where registrations live.** Per person, per project, or both as proposed? And
  should PCBcraft offer to read a person's existing KiCad library tables?
- **The project registration file.** Its name, format and how it avoids
  machine-specific paths.
- **Revision identity for KiCad items.** Is a content hash the right revision, and
  how should it read in the interface?
- **Parts in the starter library.** Should the starter library ship parts (for
  example, a resistor with an 0805 footprint), or only symbols and generic footprints?
