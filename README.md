<div align="center" id = "top">
  <img alt="logo" height="200px" src="https://raw.githubusercontent.com/danemadsen/nerthus/refs/heads/main/crates/nerthus-app/assets/nerthus.svg">
</div>

# Nerthus

Nerthus is a clean room reimplementation of Gaea 2.0, the node-based terrain
generator, written in Rust. It opens and saves Gaea 2 `.terrain` projects, and
projects it saves open in Gaea 2.3.1.0. Its goal is to build them into the
same heightfields, masks and colour maps that Gaea does.

Nerthus is an independent project. It is not affiliated with or endorsed by
the makers of Gaea.

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
name>/` unless the project's Build Settings say otherwise. Each run writes a
log to `Logs` in the Nerthus user folder (`~/.nerthus`, or
`%LOCALAPPDATA%\Nerthus` on Windows); `--verbose` adds the debugging
messages, and `--safemode` builds with the default preferences instead of
the saved ones. Run `nerthus-build --help` for the other commands and
options.

For many builds in a row, `nerthus-build host` keeps a build host loaded
between them: the first run starts it, later runs hand their project and
options to it and print its output, and `nerthus-build host --exit` closes
it.

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

## Not done yet

These parts of Gaea 2.0 are not in Nerthus yet. Left out on purpose:
licensing, the Houdini, Unity and Unreal bridges (and Tools › Install
Plugins), and features Gaea ships disabled or unfinished, such as Network
Build.

### Files

- Dropping a file onto a file parameter, or at a chosen spot on the graph.
  The windowing library Nerthus uses does not report where a dropped file
  lands, so files dropped on the graph go to its middle. A file parameter
  takes a path typed into it or picked with Browse.
- Drawing Windows metafiles (`.emf`) exactly as Windows does. Nerthus draws
  them itself, so edges and text differ slightly, and fonts that aren't
  installed are replaced by similar ones.

### Options with no effect yet

Nerthus shows these in Options but does not act on them:

- Toolbox: Show Satellite Maps List as a Single Stack.
- Graph: Allow node shortcuts only when cursor is inside the graph; Hide
  Graph Tabs (when there is only one); Record Graph Movements and Zoom in
  Undos (on by default: panning the graph adds an undo step).
- Node: Node Style (Muted or Vibrant); Show shine effect on Node when build
  completes; Show values with high precision; Accept values when clicking
  outside of editable field; Use slider for Seed instead of new Seed control.
- Viewport: Antialiasing; Do not clear viewport when unselecting a node
  (Nerthus never clears it, Gaea does unless this is on); Conserve memory
  when displaying 4K terrains.
- Build: Purge unnecessary cache during build to conserve memory.
- Experimental: Compact Interface; Use new Auto Layout method; Use Adaptive
  Debounce for Smoother Node Updates.
- Two settings Gaea reads from its settings files without showing them:
  `ShowTicks` (slider ticks) and `AllowValuesBeyondRange` (node results not
  clamped to 0–1).

### Compute

- The NVIDIA profile that Options › Compute and the first-run GPU setup
  create on Windows (prefer the NVIDIA GPU; CUDA without system-memory
  fallback).

### Help and Tools

- Project › Statistics, which says it isn't available (the Data View opens
  the same window).
- Documentation. Help › Node Reference, the property panel's Show Help for
  this Node, a parameter's Help with Automation and the "More Info" and
  "Learn more" links open Gaea's online documentation; Nerthus has none, and
  Help › User Guide opens this README.
- Installing updates. Gaea downloads the new installer and runs it; Nerthus
  opens the release page.
- The console's hidden commands: `error` (a simulated crash, for testing
  crash reporting) and the joke ones (`iddqd`, `idkfa`, `idclip`,
  `crocodile`, `openpodbaydoors`).

## License

MIT (see [LICENSE](LICENSE)). The Roboto and Roboto Mono fonts in
`crates/nerthus-app/assets/fonts` are under the Apache License 2.0, with their
licence texts beside them. The palettes in `crates/nerthus-nodes/data/color`
are Gaea's colour library, included so that SatelliteMap and the texture nodes
match Gaea.
