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

These parts of Gaea 2.0 are not in Nerthus yet.

### Properties

- Typing formulas into sliders. Gaea works a typed value out as an
  expression, with the project's variables, `Resolution`, `PX` and the
  `m()`, `cm()` and `km()` functions for heights in metres. Nerthus takes
  only a plain number (its seed box already takes formulas).
- Typing a value by right-clicking a slider's number, and the box in the
  nudge bar (right-click elsewhere on the slider) for typing one. Nerthus
  types on a double-click, and its nudge bar has only the buttons.
- Range sliders: typing either value (right-click near it), a nudge bar for
  each end, and Shift-drag to move the whole range. Nerthus's range sliders
  only drag their two ends.
- The marks at the terrain's lowest and highest heights on the Snow node's
  Snow Line and the Trees node's Snowline sliders.
- Each modifier's close button (it asks first unless Shift is held), and
  dragging modifiers to change their order. Nerthus removes a modifier only
  through the modifier toolbar or the Add Modifiers menu, and keeps them in
  the order they were added.
- The Gate node's panel (Bake Node, or Unbake Node once it is baked), and
  the panel a baked node shows instead of its parameters (Load Offloaded
  Cache).
- Showing a parameter that is bound to a variable as the variable's value,
  with a tooltip naming the variable: a click edits the value and a
  double-click opens the Data View's variables. Nerthus keeps showing the
  parameter's own control.

### Graph and baking

- The questions when a node is renamed: whether to rename its export file
  too (when it is marked for export), and whether to rename the variable
  bound to it (when exactly one is).
- Refusing to bake a node with a Macro upstream ("Macro Detected in Bake
  Chain"). Nerthus only checks whether the node itself is a Macro.
- The button that filters the Data View to the bypassed nodes, in the
  message shown when bypassed nodes stop a bake. Nerthus reports it in the
  status bar.

### Files

- Reading every image the way Gaea does. Compared with Gaea's own File
  node:
  - JPEG files decode to slightly different pixels, and CMYK JPEGs come out
    wrong.
  - GIF and SGI (`.rgb`) files don't open, nor does Gaea's own Pure Raw
    format (`.graw`).
  - Radiance `.hdr` files are read at 8 bits a channel, losing everything
    above 1.
  - TIFF files with a palette, in CIELab, with 16-bit floats or with LZMA
    compression don't open. CMYK, grey-with-alpha and planar (one plane per
    channel) TIFFs load with the wrong values, and planar ones make Nerthus
    panic when Enforce Linear Gamma is on.
  - MIFF files compressed with BZip or LZMA, or stored as CMYK or Lab,
    don't open.
  - EXR files come out gamma-encoded, so brighter, when Enforce Linear Gamma
    is on (Gaea keeps them linear), and lossy DWAA files differ in a few
    pixels.
  - Run-length-encoded BMPs load without the alpha channel Gaea gives them.
- Mesh formats besides OBJ, FBX, Collada and glTF in the Object node. Gaea's
  Assimp also reads about 40 others (STL, PLY, 3DS, Blender, DirectX, IFC,
  X3D…) when their path is typed or dropped in, and glTF 1 files compressed
  with Open3DGC.
- The offer, when a project opens, to delete bake data that an older Gaea
  left beside it (its `BakeData` folder).
- Saving over a read-only file: Gaea keeps an alert open and autosaves the
  unsaved changes; Nerthus only reports the error in the status bar.

### Startup and the system

- Registering `.terrain` files with Windows, which Gaea does at every start,
  so projects open from Explorer and show their own icon.
- Opening maximised on the monitor chosen in Options › General › Start
  Nerthus on. Nerthus opens a 1600 × 960 window wherever the system puts it.
- Showing What's New by itself the first time a new version starts.
- The editor's command-line options `-?` (lists the options), `-NoStart`
  (skips the Start dialog), `-SafeMode` (starts with the default preferences
  and options) and `-Proxy <url>` (turns the proxy on with that address and
  saves it), and a `proxy.txt` in the settings folder setting the proxy
  address. Nerthus treats an option it doesn't know as a project to open.
- Detecting the Nahimic audio software on Windows, unloading it from the
  process and warning about it, since it can break rendering.
- The console's joke `eject` and `uninstall` commands.

### Left out on purpose

- Licensing.
- The Houdini, Unity and Unreal bridges (and Tools › Install Plugins).
- Features Gaea ships disabled or unfinished, such as Network Build.
- Drawing Windows metafiles (`.emf`) exactly as Windows does. Nerthus draws
  them itself, so edges and text differ slightly, and fonts that aren't
  installed are replaced by similar ones.
- Reading EPS files. Gaea hands them to Ghostscript, a separately installed
  program that draws them with its own fonts.
- Gaea's interface fonts. Nerthus uses Roboto and Roboto Mono, so text is
  spaced a little differently and monospaced text is about a fifth wider.
- The icons that are Gaea's own or licensed artwork, about 100 of them
  (45 node icons, 13 family and toolbox icons, and interface icons).
  Nerthus shows Tabler icons in their place.
- Gaea's overlay images and example projects. Nerthus draws its own overlay
  patterns and ships 12 examples of its own.
- Gaea's colour library. Nerthus's palettes are its own, taken from
  Sentinel-2 satellite imagery to look like Gaea's palette with the same
  number, so SatelliteMap, SuperColor, and Cartography through a
  SatelliteMap build with somewhat different colours from Gaea's, and so do
  Weathering and TextureBase masks with New accentuation (which read Rock
  palette 117). Projects saved by Gaea keep their palette numbers.

## License

MIT (see [LICENSE](LICENSE)). The Roboto and Roboto Mono fonts in
`crates/nerthus-app/assets/fonts` are under the Apache License 2.0, with their
licence texts beside them. The palettes in `crates/nerthus-nodes/data/color`
contain modified Copernicus Sentinel data 2023–2024; the
[README](crates/nerthus-nodes/data/color/README.md) beside them says where
each one comes from.

The documentation in `crates/nerthus-app/assets/docs` (Help › User Guide) is
adapted from Gaea 2's documentation, which is published under the MIT licence;
its licence text is in `crates/nerthus-app/assets/docs/LICENSE.txt`.
