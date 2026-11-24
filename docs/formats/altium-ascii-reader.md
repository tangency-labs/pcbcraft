# Altium ASCII reader

`crates/altium-ascii` (`pcbcraft-altium-ascii`) reads Altium's plain-text (ASCII)
board and schematic saves. It is read-only: it takes any buffered input and never
writes it. It is the base for importing such saves; it does not yet create
PCBCraft documents, and nothing in the editor, CLI or MCP calls it yet.

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

Each record is bounded before memory for it is allocated: its size in bytes
(16 MiB by default, counting every line of a multi-line text), its number of
properties (65,536) and the length of each property name (256 bytes). The reader
keeps at most one record and one line in memory.

A record cut off at the end of the file, a property without `=`, an empty
property name and a record over a limit are reported as errors carrying the byte
offset and line where the record or property starts. Reading stops at the first
error; nothing after it is read.
