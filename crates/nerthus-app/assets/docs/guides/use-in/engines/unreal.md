# Unreal

Unreal Engine's landscapes import 16-bit grayscale heightmaps as PNG or RAW (`.r16`).

## Exporting from Nerthus

1. Mark the terrain's final heightfield node for export (**F3**) and set its format to **PNG16** or **Ushort Raw 16-bit (.r16)** in [Build Settings](../../../ui/interface/build-options/index.md) › Nodes.
2. Mark any masks you want as landscape layers too, as **PNG16** or **PNG8**.
3. Build with Project › Build and Export. The heightmap's resolution is the build resolution.

By default, the filename for each output is decided by the name of the node. Rename a node to rename its file.

## Importing into Unreal

1. Enter **Landscape** mode.
2. Click **Import from File**.
3. Click the button next to **Heightmap File** and select your heightmap. Unreal reads its resolution from the file.
4. Calculate the Z scale for your heightmap using the formula (Max height of your heightmap in meters) x (100 to convert to centimeters) x 0.001953125. The terrain's height in meters is set in Build Settings › [Terrain](../../../ui/interface/build-options/terrain.md).
5. Click **Import**.

Import the masks as the landscape's layer weight maps.

You can follow the exact instructions on Unreal's official documentation:

https://dev.epicgames.com/documentation/en-us/unreal-engine/importing-and-exporting-landscape-heightmaps-in-unreal-engine
