# Craft control transport

`craft-control` owns bounded line framing and the native loopback connection.
The peer puts its token on the first JSON request. The receiving app
authenticates that request before dispatch and gets the request back without
the token. Each socket read has a frame deadline. The framing used by the MCP
stdio server also builds for WebAssembly.

PCBCraft uses the framing in its existing MCP server. The live desktop
integration can use the listener and client for that same command service.
The app supplies token distribution, discovery, and permissions policy.
The MCP adapter supplies the protocol and tool catalog. The editor supplies
command dispatch and session history.

A future SolidCraft extraction can move its socket and framing code behind
this boundary. Start with `apps/solidcraft/src/control_server.rs` and
`crates/mcp/src/backend.rs`, preserving their wire contract, discovery,
and reconnect behavior at the SolidCraft entry points.
