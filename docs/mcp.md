# MCP and live control

The executable `pcbcraft-cli mcp` implements a stdio MCP server. It negotiates
2025-11-25, 2025-06-18 or 2024-11-05 protocol versions, advertises tools, and returns
tool failures as `isError` responses. Messages are capped at 1 MiB.

Example client configuration after `cargo build -p pcbcraft-cli`:

```json
{
  "mcpServers": {
    "pcbcraft": {
      "command": "/absolute/path/to/target/debug/pcbcraft-cli",
      "args": ["mcp"]
    }
  }
}
```

For a visible session, launch:

```sh
pcbcraft --sample --control 127.0.0.1:7991
pcbcraft-cli mcp --connect 127.0.0.1:7991
```

The bridge binds only to literal loopback addresses. It is trusted local automation
with filesystem access as the invoking user. It has no remote authentication.
Only tools/call requests are forwarded; tool discovery comes from the same registry.
Do not expose this port through forwarding to an untrusted network.

## Recommended agent flow

1. `inspect_design`: read current revision, stable IDs and capability limits.
2. `list_commands`: inspect schemas and millimetre/degree units.
3. `execute` or `batch`: supply `expected_revision` when coordinating concurrent edits.
4. `checks`: inspect scope, findings and object IDs; initial checks are not full ERC/DRC.
5. `screenshot`: obtain a PNG design preview of `schematic` or `pcb`.
6. `save` / `export`: use server-host paths. WSL paths differ from Windows paths.

Every engine command is also a directly callable MCP tool, such as `component.add`
or `board.cutout.add`. Use the generic `execute` wrapper for revision guards.

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"example","version":"1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"execute","arguments":{"command":"component.add","params":{"kind":"resistor","position":{"x":30,"y":20}},"expected_revision":0}}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"inspect_design","arguments":{}}}
```

Atomic batch example:

```json
{"commands":[{"command":"board.resize","params":{"width":80,"height":50}},{"command":"board.cutout.add","params":{"center":{"x":5,"y":5},"radius":1.6}}],"expected_revision":1}
```

The entire batch is one undo step. Failure leaves document, revision and history
unchanged. New/open/sample replacements are undoable. IDs are not recycled after undo.

`screenshot` renders a headless design preview; it does not capture desktop chrome.
To capture the actual UI, use the desktop's `--capture /absolute/path.png` option.

## Library workflow

`library.catalog` discovers host folders. `library.search` accepts `path`, `kind`
(`symbol` or `footprint`), optional `query`, and `limit` (1–100). A symbol path is
a `.kicad_sym` file; a footprint search path is a `.pretty` directory. Results
report search and directory-scan truncation. `library.inspect_symbol` accepts
`path` and `name`; `library.inspect_footprint` accepts a `.kicad_mod` `path`.
Read tools return resolved geometry and unsupported features without changing or
locking the project. Symbol catalogs cache two files using length/mtime; restart
to refresh external replacements deliberately preserving both metadata fields.

`component.place_library` accepts `path`, `symbol`, `position` and optional
`reference`/`value`. `footprint.assign` accepts component `id` and footprint `path`.
The engine validates supported placement and exact numbered-pin/pad correspondence,
embeds the used definitions, and makes assignment undoable. `board.sync` returns
`unassigned_components` rather than inventing packages for imported symbols.
These commands also work in atomic batches and the live desktop bridge.

```sh
pcbcraft-cli tool library.catalog
pcbcraft-cli tool library.search --args '{"path":"/usr/share/kicad/symbols/Device.kicad_sym","kind":"symbol","query":"resistor","limit":20}'
```
