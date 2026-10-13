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

## Not done yet

These parts of Gaea 2.0 are not in Nerthus yet. Left out on purpose:
licensing, the Houdini, Unity and Unreal bridges (and Tools › Install
Plugins), and features Gaea ships disabled or unfinished, such as Network
Build.

### Build and Export

- Close Gaea and Build, the Build and Export dialog's choice to close the
  editor to free memory while the build runs.
- Save cached build: when the preview resolution matches the build's
  (single images up to 8K) and every marked node is built, Gaea offers to
  export the editor's results instead of building again.
- The warning before a bake that may need more than a third of the
  installed memory.
- `nerthus-build` keeps no log file (Gaea's command-line build writes one to
  `Logs` for every run) and has no `--verbose` or `--safemode` (build with
  default preferences).
- A resident build host, Gaea.SwarmHost, which stays loaded and builds the
  projects that later runs hand to it.

### Files

- File › Open's repairs of damaged projects: nodes on graph tabs that no
  longer exist (offered back as "Recovered Graph" tabs), stale export
  settings, and bookmarks or selected and locked nodes that no longer exist.
- Saving the selected node, the open graph tab and a preview lock (F) with
  the project, and reopening on them. Nerthus reopens on the selected node
  of a project Gaea saved, but saves none of the three.
- Macros that use other macros. Gaea lists them when it exports a macro and
  adds them to a project along with it.
- Remembering the last Open and Save folders between sessions.
- Progress windows for long operations other than baking: opening a file,
  writing the preview cache to disk, loading baked caches and loading a
  region preview.
- Reading Windows metafiles (`.emf`, which the File node offers) and
  Photoshop files (`.psd`), which Gaea reads through ImageMagick.
- Example projects. Gaea ships 59 for File › Open Examples; Nerthus ships
  none.

### Options with no effect yet

Nerthus shows these in Options but does not act on them:

- Toolbox: Show SatMaps List as a Single Stack.
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
are Gaea's colour library, included so that SatMap and the texture nodes match
Gaea.
