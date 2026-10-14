<div align="center" id = "top">
  <img alt="logo" height="400px" src="https://raw.githubusercontent.com/danemadsen/nerthus/refs/heads/main/crates/nerthus-app/assets/nerthus.svg">
</div>

# Nerthus

Nerthus is a clean room reimplementation of Gaea 2.0, the node-based terrain
generator, written in Rust. It opens and saves Gaea 2 `.terrain` projects, and
projects it saves open in Gaea 2.3.1.0. Its goal is to build them into the
same heightfields, masks and colour maps that Gaea does.

Nerthus is an independent project. It is not affiliated with or endorsed by
the makers of Gaea.

![The Nerthus editor](media/editor.png)

## A look around

A terrain is built as a graph of nodes. The viewport shows the selected node in
3D as it builds, and the property panel beside it changes the node's settings.
Most terrains take the same few steps: a shape to start from, simulations that
wear it down, detail on its surface, then colour.

![The Volcanic Isle example a node at a time: Volcano, ErosionPro, Sea, then its colour](media/workflow.gif)

### Create

Primitives and terrain nodes make the first shape: mountains, ranges, ridges,
canyons, volcanoes, islands, dunes, craters and noises.

![Volcano, MountainRange, DuneSea and CraterField](media/create.jpg)

### Simulate

Erosion, rivers, lakes, the sea, thermal erosion, snow and glaciers work the
shape over.

![ErosionPro, Lichtenberg, Snow and Glacier](media/simulate.jpg)

### Surface

Surface nodes add the rock: strata, terraces, sandstone, shattered stone and
outcrops.

![Stratify, Sandstone, Shatter and Outcrops](media/surface.jpg)

### Colorize

Colour nodes paint the terrain from palettes, gradients and the masks the
other nodes make.

![The Badlands example as a heightfield, and coloured by CLUTer](media/colorize.jpg)

### Export

Build and Export writes the nodes marked for export as heightfields, masks and
colour maps (EXR, PNG, TIFF, JPEG or RAW), as one image each or as tiles.

![Height, flow, wear, deposits and colour maps from the Erosion Data Maps example](media/maps.jpg)

![Build Settings listing the nodes marked for export](media/build-settings.png)

### Examples

File › Open Examples… opens one of the 16 example projects Nerthus comes with.

![The Fractal Terraces example turning in the viewport](media/turntable.gif)

![The example projects](media/examples.jpg)

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

`nerthus --help` (or `-?`) lists the editor's options, such as `--no-start`
to skip the Start dialog, `--safe-mode` to start with the default
preferences and options, and `--proxy <url>` to set the proxy used for
update checks.

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

## Web version

The editor also runs in a browser with WebGL 2. The Web workflow builds it
for every pull request and publishes main's with GitHub Pages, at
<https://danemadsen.github.io/nerthus/>.

To build it yourself, add the WebAssembly target and the `wasm-bindgen`
command of the version in `Cargo.lock`, then run the build script, which
writes the site to `target/web`:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <version in Cargo.lock> --locked
crates/nerthus-app/web/build.sh
python3 -m http.server --directory target/web
```

A web page has no files, threads or other programs to use, so the web
version differs from the desktop one:

- Nodes build on the CPU alone and on the page's one thread, a node at a
  time between frames, so builds take longer.
- Projects are the examples it comes with and files you pick with Open;
  saving downloads the project. Settings, recent files and autosaves are
  not kept between visits.
- Build and Export, baking and the disk cache, screenshots and turntables,
  importing files, installing and exporting macros, loading and saving
  cameras, File nodes reading from disk, Diagnostics, update checks and the
  MCP server are left out.

## MCP server

The editor can run an [MCP](https://modelcontextprotocol.io) server, so AI
assistants such as Claude Code can work on the open project: add and connect
nodes, set parameters, build nodes and look at the results, and run Build and
Export. Their edits show in the editor as they happen and can be undone.

![A graph built over MCP: a Mountain, ErosionPro and SatelliteMap added, the Mountain's seed changed and the camera moved](media/mcp.gif)

Turn it on with Tools › MCP Server (it stays on for later starts), or start
the editor with `--mcp`. It listens on `http://127.0.0.1:47110/mcp`;
`--mcp-port <port>` picks another port. To connect Claude Code:

```sh
claude mcp add --transport http nerthus http://127.0.0.1:47110/mcp
```

The server only takes connections from this machine and has no other
authentication, so while it is on, any program running on this machine can
use it.

Its tools:

- **The project**: `get_project`, `open_project`, `new_project`,
  `save_project`, `set_project_settings`, `list_variables`, `set_variable`.
- **Node types**: `search_node_types`, `describe_node_type` (with how the
  example projects use each type and what `calibrate_nodes` measured it to
  make), `describe_modifier`, `suggest_nodes`. Types can also be named as
  `.terrain` files name them (`Erosion2` for ErosionPro, `SatMap` for
  SatelliteMap).
- **Building graphs**: `create_graph` (a whole graph of nodes, modifiers,
  export marks and connections as one undoable edit), `list_examples`,
  `open_example`, `add_node`, `set_parameters`, `update_node`,
  `connect_nodes`, `disconnect_input`, `delete_nodes`, `set_modifier`,
  `undo`, `redo`.
- **Checking graphs**: `list_nodes`, `get_node`, `select_node`,
  `validate_graph` (the open project, a file or a proposed graph; it can fix
  what is safe to fix and build it small to be sure), `analyze_graph`,
  `optimize_parameters`.
- **Building and looking**: `build_node`, `get_node_image`, `capture_view`,
  `set_camera`, `export_node_map`, `mark_for_export`, `build_and_export`,
  `run_project` (any saved project, with the command line's options:
  resolution, build folder, profile, region, mutation seed, variables, one
  node), `build_history`, `analyze_build`, `analyze_terrain` (relief in
  metres, gentle ground, edges against the middle, profiles),
  `calibrate_nodes`.
- **Project files**: `get_project_file`, `list_projects`,
  `repair_project_file`, `patch_project_file`, which change a `.terrain`
  file without opening it in the editor.

## Not done yet

These parts of Gaea 2.0 are not in Nerthus yet.

### Files

- Some images the File node reads differently from Gaea:
  - EXR files with subsampled channels (luminance/chroma images, for
    example) don't open.
  - TIFF files compressed with old-style JPEG don't open.
  - MIFF files in a colorspace other than sRGB, linear RGB, grey, CMYK, Lab
    or XYZ (HSL, for example) don't open.
  - Other formats ImageMagick reads, such as TGA and PNM, go through a
    general-purpose decoder rather than ImageMagick's, so their pixels can
    differ.

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
  patterns and ships 16 examples of its own.
- Gaea's colour library. Nerthus's palettes are its own, taken from
  Sentinel-2 satellite imagery to look like Gaea's palette with the same
  number, so SatelliteMap, SuperColor, Cartography through a SatelliteMap,
  and Weathering build with somewhat different colours from Gaea's, and
  TextureBase with New accentuation builds a somewhat different mask
  (Weathering and TextureBase read Rock palette 117). Projects saved by Gaea
  keep their palette numbers.

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
