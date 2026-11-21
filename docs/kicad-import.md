# KiCad schematic import

PCBcraft can open a self-contained KiCad schematic as a new, editable project
([#60](https://github.com/tangency-labs/pcbcraft/issues/60)). This first slice
covers one `.kicad_sch` file written by KiCad 6 to 9. Board import follows once
the representation of footprints that arrive without a schematic is agreed.

The import is exact or refused: a project it produces connects the same pins
the file describes. Anything that would change a connection refuses the whole
file, listing up to 20 reasons; appearance the native model doesn't hold is left
out and listed.

## Running it

- **MCP and agents:** `project.import_kicad_schematic` with `text` (the file's
  UTF-8 contents, at most 8 MiB) and an optional `name`. It replaces the open
  project in one undoable step and returns the revision and a `report` of what
  was left out, each entry with a description and a count. The `[{what, count}]`
  shape is stable; the descriptions are for people and may be reworded, so don't
  match on them. Read-only sessions can't run it.
- **Command line:** `pcbcraft-cli import-kicad SCHEMATIC DESTINATION` writes a
  new project folder, or a `.pc` package when the destination ends in `.pc`, and
  prints the report. Existing destinations are never replaced.

There is no desktop menu entry yet.

## What is imported

| KiCad | PCBcraft |
| --- | --- |
| Symbols in the file's `lib_symbols`, including ones that `extends` another | Native `pcbcraft-symbol/4` definitions pinned in the project under a library `kicad-…` named by a digest of the converted symbols (so different imports never claim one identity for different content; the `kicad-` prefix is a naming convention, not reserved), with the library name *Imported from KiCad* and license `NOASSERTION`, since a design's embedded symbols carry their own terms |
| Units 1 to 32 and the common unit 0 | One native unit per KiCad unit (`main` for single-unit symbols, else `a`, `b`, …), each with the common pins and drawing |
| Pins, with electrical type, name, number, length, direction and inverted/clock marks | Terminals and integrated pin appearance; `free` pins become `unspecified` |
| Placed symbols, rotated by quarter turns and mirrored | Placed parts and units in the same pose |
| References and units from the instance records (KiCad 7 and later) or `symbol_instances` (KiCad 6) | Part references; units of one reference share a part. A unit placed twice under one reference (a copy) becomes a second part with a new reference, as do unannotated (`R?`) and invalid references; each is reported |
| Value, Datasheet, Description and other fields; DNP, in-BOM and on-board flags | Value, standard and custom fields, and the part's fitting attributes. The Footprint field is kept as a `KiCad footprint` field; assigning footprints is separate work |
| Power symbols | Parts whose power terminal joins the global net their Value names. Power flags (`PWR_FLAG`: a power symbol whose one pin is a power output) only drive KiCad's electrical rules check and are left out, reported; flags sitting on crossings without a dot, so that alone or together they join those wires, refuse the file instead |
| Wires, junction dots, local and global labels | Native wire routes, junctions and net labels |
| No-connect marks on pins or wire ends | No-connect intent on that terminal or wire end |

## What refuses a file

- Anything the reader couldn't read that can carry a connection: a pin without a
  number, a wire, junction, label or symbol out of range, a symbol unit whose name
  gives no unit, or an item it doesn't know.
- Sub-sheets, hierarchical labels, buses and bus entries
  ([#43](https://github.com/tangency-labs/pcbcraft/issues/43)).
- Diagonal wires, and symbols turned by other than a quarter turn.
- Pins of two ordinary symbols touching without a wire. Power symbols and labels
  on pins connect as in KiCad.
- One name used by more than one kind of named marker (local label, global label,
  power symbol), such as a global label `+3V3` beside a `+3V3` power symbol.
  PCBcraft joins every marker of a name on a sheet; KiCad's rule across those
  kinds isn't confirmed.
- In a file that doesn't say it comes from KiCad 8 or later (through its
  `generator_version`), a power symbol whose Value differs from its power pin's
  name: KiCad 6 and 7 name the net after the pin, KiCad 8 and later after the
  Value, as the import does. Power symbols whose two names match import from any
  version.
- A power symbol whose Value, its net name, isn't a valid one-line Value of at
  most 64 characters. Other Values are shortened or fall back to the symbol's
  default, reported.
- Symbols the native format can't hold exactly: pins off the 1.27 mm grid or
  more than 12.7 mm from the symbol's origin, two pins at one position, pin
  numbers other than 1 to 8 letters, digits, `+`, `-` or `_`, a number with a
  different name or type in different units, more than 32 units, hidden power
  pins on ordinary symbols, which KiCad joins to a net by name, and an alternate
  (De Morgan) body style that places, names, types or hides pins differently from
  the normal one, or whose pin names are too long (over 512 bytes) to compare.
- More than 1,000 placed symbols (this slice's own limit; a sheet holds 4,096),
  4,000 straight wire pieces, 256 labels or 4,096 native wires, for now: the import runs every placement and wire through the ordinary commands
  and can't be cancelled, so this slice keeps it within seconds, and a sheet holds
  at most 256 named labels and 4,096 wires. Symbols may also take at most
  2,000,000 points of conversion work in all, counting a common part once for each
  unit it is drawn in: each drawing costs its points, each text and pin one, and
  reading a symbol one per unit and pin it has.
  Drawing beyond a unit's 256 entries is left out unconverted and costs nothing,
  except a long polyline that crosses the limit, which costs all its points.
  The converted symbols may come to at most 32 MiB less 64 KiB as native documents,
  and one symbol to 1 MiB, the library loader's limits. These refusals use the
  `cap_exceeded` error code, except a single oversized symbol, which is refused by name.
- A symbol both mirrored and turned a quarter, when a wire ends where its pins
  would be if KiCad mirrored before turning: the one case the turning order
  matters, refused rather than guessed.
- Anything else a native command refuses; the message names the step.

Wire runs with more than 64 corners, and closed loops of wire, are drawn as
several joined native wires.

After building the project, the import works out the nets the file describes,
from its geometry alone (wire ends and pins on wires join, crossings without a
dot don't, labels and power symbols join by name, a pin every unit shares is one
pin) and refuses the result if the native project's connections differ. That
check uses the same reading of the file as the import, including how a placed
symbol's rotation and mirror move its pins (turned first, then mirrored, as the
files observed show). The report counts wire ends that touch nothing, which is
also what a misplaced pin would leave behind.

## What is left out, and reported

Sheet text and drawings, images, text boxes and tables; field text positions,
which use the native defaults; global label shapes; alternate (De Morgan) body
styles that place, name, type and hide their pins like the normal one, and any further
styles; power flags; no-connect marks that are not
on a pin or wire end; and drawing details the native format holds differently. Curves are drawn
as polylines, fills are kept only on convex outlines, and very long pin names are
shortened to 32 characters.

## Sources

The reader in `crates/kicad` is adapted from CircuitCraft, whose KiCad readers
were written from KiCad's
[published file format documentation](https://dev-docs.kicad.org/en/file-formats/)
and from reading schematic files of public designs saved by KiCad; see
[ATTRIBUTION.md](../ATTRIBUTION.md#adapted-from-circuitcraft). No KiCad source
was consulted. The test fixtures here are original, written by hand.

The import builds the project through the ordinary commands in a scratch
session, so the command path, validation and undo are the same as for edits
made by hand. Each command costs a few milliseconds; a 300-part schematic takes
about 3 seconds in the `ci` profile.
