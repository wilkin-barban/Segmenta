# Segmenta MCP server

`segmenta-mcp` exposes the existing daemon's download operations to MCP clients over newline-delimited JSON-RPC on standard input and output. It never starts the daemon or changes client configuration.

Build with `cargo build -p segmenta-mcp`, then configure your MCP client to launch `target/debug/segmenta-mcp` with no arguments. The daemon must already be running. The server uses `SEGMENTA_SOCKET` when set, otherwise `$XDG_RUNTIME_DIR/segmenta/daemon.sock`, or `/tmp/segmenta-<uid>/daemon.sock` if the runtime directory is unset.

The server exposes `segmenta_list`, `segmenta_add`, `segmenta_pause`, `segmenta_resume`, and `segmenta_delete`. The delete tool removes the task; `delete_file: true` also requests deletion of its file. Grant access only to clients trusted to perform these operations. Download results may contain URLs and filesystem paths, so clients must treat them as sensitive. The server does not print diagnostics to stdout and reports daemon failures without echoing request data.

The stdio protocol version is `2025-06-18`. This server currently exposes tools only; it does not configure the desktop or other MCP clients automatically.
