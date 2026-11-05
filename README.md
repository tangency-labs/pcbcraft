# PCBCraft

Electronics design that feels like the tools you already know, built so a person and an agent can work on the same board, at the same time, with the same commands.

**Status: just started.** The window, menus, preferences, undo and an MCP server work, on the desktop and in the browser. The command set supports placing and inspecting resistors on an in-memory schematic. [VISION.md](VISION.md) says what it should be.

## Run it

```sh
cargo run -p pcbcraft                        # desktop
cargo run -p pcbcraft-cli -- mcp             # MCP server on stdin and stdout
cd apps/pcbcraft-web && trunk serve          # browser, at http://127.0.0.1:8781
```

The browser build needs `rustup target add wasm32-unknown-unknown` and [Trunk](https://trunkrs.dev).

To connect an MCP client, point it at `pcbcraft-cli mcp`. Today it runs its own session with no window, so it can't yet see the project open in the app.

## Where things go

| Path | What it is |
|---|---|
| `crates/engine` | The session and the command set. The schematic and board model goes here. No UI. |
| `crates/mcp` | The MCP server, a thin layer over the engine's commands. |
| `crates/app` | The window, shared by desktop and browser. Menus run engine commands. |
| `crates/chrome` | Theme, fonts, menus, bars and dialogs in the Craft style. Stands in for the shared Craft UI crate. |
| `apps/` | The desktop, command line and browser programs. They only start things. |

To add a feature: add a command to `crates/engine/src/lib.rs`, then call it from a menu or tool. MCP's `list_commands` and `execute` pick it up on their own.

## Checks

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo deny --locked check licenses sources bans
```

## License

MIT OR Apache-2.0, at your option. See [LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE) and [ATTRIBUTION.md](ATTRIBUTION.md).
