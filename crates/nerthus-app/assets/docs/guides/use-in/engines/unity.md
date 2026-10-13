# Unity

Unity's terrains import heightmaps as RAW files.

## Exporting from Nerthus

1. Mark the terrain's final heightfield node for export (**F3**) and set its format to **Ushort Raw 16-bit (.raw)** in [Build Settings](../../../ui/interface/build-options/index.md) › Nodes. Nerthus writes raw files as little-endian 16-bit values.
2. Mark any masks you want as splat maps too, as **PNG8** or **PNG16**.
3. Build with Project › Build and Export.

## Importing into Unity

1. Select the Terrain, and open **Terrain Settings** (the gear in its toolbar).
2. Under **Texture Resolutions (On Terrain Data)**, click **Import Raw**.
3. Select the heightmap, and set **Depth** to 16 bit, **Byte Order** to Windows (little-endian), and **Resolution** to the heightmap's size.
4. Set **Terrain Size** to the terrain's width, length and height in meters, as set in Build Settings › [Terrain](../../../ui/interface/build-options/terrain.md).
5. Click **Import**.

Unity's terrain heightmaps are a power of two plus one pixels across (such as 2049), so choose the build resolution closest to the terrain you need.
