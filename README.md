<div align="center" id = "top">
  <img alt="logo" height="200px" src="https://raw.githubusercontent.com/danemadsen/nerthus/refs/heads/main/crates/nerthus-app/assets/nerthus.svg">
</div>

# Nerthus

Nerthus is a clean room reimplementation of QuadSpinner's Gaea 2.0, the
node-based terrain generator, written in Rust. It opens and saves Gaea 2
`.terrain` projects, and projects it saves open in Gaea 2.3.1.0. Its goal is
to build them into the same heightfields, masks and colour maps that Gaea
does.

Nerthus is an independent project. It is not affiliated with or endorsed by
QuadSpinner.

## Building

Nerthus needs Rust 1.99 or newer. It is developed on macOS; Windows and Linux
builds are untested.

```sh
cargo build --release
```

## Running

The editor:

```sh
cargo run --release --bin nerthus -- path/to/project.terrain
```

Command-line builds:

```sh
cargo run --release --bin nerthus-build -- build path/to/project.terrain
```

Builds go to a new numbered folder under `Documents/Nerthus/Builds/<project
name>/` unless the project's Build Settings say otherwise. Run
`nerthus-build --help` for the other commands and options.

## MCP server

The editor can run an [MCP](https://modelcontextprotocol.io) server, so AI
assistants such as Claude Code can work on the open project: add and connect
nodes, set parameters, build nodes and look at the results, and run Build and
Export. Their edits show in the editor as they happen and can be undone.

Turn it on with Tools › MCP Server (it stays on for later starts), or start
the editor with `--mcp`. It listens on `http://127.0.0.1:47110/mcp`;
`--mcp-port <port>` picks another port. To connect Claude Code:

```sh
claude mcp add --transport http nerthus http://127.0.0.1:47110/mcp
```

The server only takes connections from this machine and has no other
authentication, so while it is on, any program running on this machine can
use it.

## License

MIT (see [LICENSE](LICENSE)). The Roboto and Roboto Mono fonts in
`crates/nerthus-app/assets/fonts` are under the Apache License 2.0, with their
licence texts beside them. The palettes in `crates/nerthus-nodes/data/color`
are Gaea's colour library, included so that SatMap and the texture nodes match
Gaea.
