# Build Settings

The **Build Settings** window gathers the settings that define how Nerthus builds and exports a terrain. Open it with Project › Build Settings (**Ctrl + B**). Use the tabs on the left to choose build scope, output destinations, tiled build behavior (the Tiles tab is available for tiled output), export nodes, scripts, Terrain Definition, regions and profiles; the buttons along the bottom start the build.

| Tab | Purpose |
|---|---|
| [Resolution](resolution.md) | Choose the build scope, resolution, subdivision mode, and output layout. |
| [Build](build.md) | Set the build destination, filename behavior, and overwrite handling. |
| [Tiles](tiles.md) | Configure tile naming, organization, overlap, and axis handling. |
| [Nodes](nodes.md) | Select exportable ports and per-node output settings. |
| [Script](script.md) | Run post-build commands automatically after a build finishes. |
| [Terrain](terrain.md) | Define terrain width, height, and real-scale values in the Terrain Definition panel. |
| [Regions](regions.md) | Create focused high-detail regions inside a larger terrain. |
| [Profiles](profiles.md) | Save and recall complete sets of build settings. |
| [Commands](commands.md) | The buttons along the bottom: start the build or copy the generated command line. |

## How the tabs work together

Two tabs usually define the foundation of a build:

* **Resolution** determines how many pixels are used to represent the build.
* **Terrain** defines the physical size of the world through the **Terrain Definition** settings.

Together, those settings determine the terrain's meters-per-pixel value and help keep erosion, scale judgments, and downstream exports grounded in a realistic size.

For background on scale, see [Scale and Resolution](../../../using/getting-started/important/scale-and-resolution.md) and [Dimensions and Scale](../../../using/advanced-topics/technical-information/dimensions-and-scale.md).
