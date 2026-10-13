# Introduction

Follow this brief tutorial to familiarize yourself with basic terrain design and texturing: a mountain, eroded, colored, and built into files.

## 1. Start a Project

Choose File › New (**Ctrl + N**). The graph is empty, ready for the first node.

## 2. Add a Mountain

Press **Tab** over the graph to open the node search, type `mountain`, and press **Enter**. A Mountain node appears on the graph, selected, and the viewport shows it.

Its properties show on the right. Try a few: **Scale** makes it broader or narrower, **Height** taller or lower, and **Style** changes its character. **Seed** gives a different mountain of the same kind.

> [!TIP]
> The toolbox lists every node by family too: the Mountain is under **Terrain**.

## 3. Erode It

With the Mountain selected, press **Tab** again, type `erosion` and press **Enter**. The Erosion node is added after the Mountain and connected to it, and the viewport now shows the eroded mountain.

Erosion carves channels where water would run and deposits sediment below them. Its **Duration** sets how long it runs; **Downcutting** how deep the channels cut. See [Understanding Erosion](../../understanding-erosion/index.md) for more.

To connect nodes yourself, drag from one node's output port (on its right) to another's input port (on its left).

## 4. Color It

With the Erosion node selected, add a **SatelliteMap** the same way. It colors the terrain from palettes taken from real places: pick one in its properties. The viewport shows the colors on the terrain.

## 5. Save It

Choose File › Save (**Ctrl + S**) to save the project as a `.terrain` file.

## 6. Build It

Select the Erosion node and press **F3** to mark it for export; do the same for the SatelliteMap. Then choose Project › Build and Export (**Ctrl + Shift + B**).

Nerthus builds the terrain at the build resolution (Project › Build Settings, **Ctrl + B**) and writes the heightfield and the color map to a new numbered folder under `Builds` in your Nerthus documents folder. Project › Open Build Folder opens it.

## Next Steps

- [Masks](masks.md): Limit an effect to part of the terrain
- [Exporting Elements](exporting-elements.md): More on what to export, and how
- [Understanding Erosion](../../understanding-erosion/index.md): The erosion nodes in depth
- File › Open Examples: finished projects to take apart
