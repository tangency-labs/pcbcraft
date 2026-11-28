# Altium ASCII reader

`crates/altium-ascii` (`pcbcraft-altium-ascii`) reads Altium's plain-text (ASCII)
board and schematic saves. It is read-only: it takes any buffered input and never
writes it. It is the base for importing such saves; it does not yet create
PCBCraft documents, and nothing in the editor, CLI or MCP calls it yet.

Altium import ([#567](https://github.com/tangency-labs/pcbcraft/issues/567))
follows the path the [KiCad schematic import](../schematic/kicad-import.md)
set: a format reader that knows nothing of PCBCraft's model and is independent
of the GUI and CLI, like `crates/kicad`, then the engine's validated, atomic
import with its report of what was left out. This crate is the reader's first
layer; converting records into a project is later work on that shared path.

## Records and properties

A save is a sequence of records, each a list of `NAME=VALUE` properties separated
by `|`. One property, chosen by the dialect (for example `RECORD` or `KIND`),
names the record's kind. The framer does not assume one record per line:

- a record may start a line with or without a leading `|`;
- several records may share one line: a second record-kind property starts the
  next record;
- a text value of a property the dialect marks as multi-line may hold raw line
  breaks, and the record continues on the following lines until a line starts a
  new record.

Both CR LF and LF line breaks are accepted, and blank lines between records are
skipped. Values stay raw bytes at this level.

## Limits and errors

The reader checks each limit before storing the data it bounds: record size
(16 MiB by default), property count (65,536), and property name length (256 bytes).
Record size includes internal line breaks but not the terminating line break.
When records share a line, the `|` before the next kind counts toward the
preceding record. The reader yields one record at a time and keeps only enough
lookahead to recognize a line break or the next kind prefix.

A record cut off at the end of the file, a property without `=`, an empty
property name and a record over a limit are reported as errors carrying the byte
offset and line where the record or property starts. Completed records are
yielded before errors in later records. Parsing stops at the first error.
