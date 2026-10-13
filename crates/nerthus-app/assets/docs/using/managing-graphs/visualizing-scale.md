# Visualizing Scale

## Using Overlays

Nerthus has several overlays that show relative scale on your terrain. They are drawn over the render, so you can judge sizes without changing anything in your graph. Pick an overlay and its opacity in the viewport toolbar's **Viewport Helpers** menu.

A great way to visualize a specific scale is to make an overlay of your own with precise shapes and sizes.

### Creating a Custom Overlay

Choose **Custom** among the overlays in the Viewport Helpers menu to load an image of your own. The image is stretched over the whole terrain.

To draw shapes of a precise size, work out how many pixels a meter is. If your overlay is 2048 x 2048 pixels and your terrain is 2500 m wide (Build Settings › Terrain › Width), then `1m = 2048 / 2500` (ie, `0.8192` pixels). A 25 m square is `0.8192 x 25 = 20.48` pixels across, so draw a square of 20.48 x 20.48 px in your image editor and place it where you wish. 50 m would be 40.96, and so on.

The overlay is blended over the render: mid-gray leaves the render as it is, darker shades darken it and lighter shades lighten it. Draw your shapes in black or white on a mid-gray background, and save the image as a PNG to keep its edges crisp. Then load it as a custom overlay.

Now you can visualize any specific area, shape, perimeter, etc. on your terrain.

### Creating Overlays in Nerthus

Any image editor can make an overlay, but you can also make a simple one in Nerthus.

Make a scale block as explained in [Creating a 3D Scale Dummy](#creating-a-3d-scale-dummy) below. Then add a Curvature node, and set its **Type** to Vertical with a **Falloff** of 1. This outlines the block.

Invert the output if you want a dark outline on a light ground. Once your visual is ready, right-click the 2D Viewport and choose **Save to file** to save it as an image, which you can then load as a custom overlay.

You can create more elaborate overlays with Grid, Halftone, Tint, and various other nodes.

## Creating a 3D Scale Dummy

This technique creates a simple block of a specific size that you can add to your terrain preview.

First create a `Constant` node and set its **Height** to 1. Then connect it to a `Transform` node.

The Transform node's **Scale** is a fraction of the terrain's width. To make a 25 m block on a terrain 2500 m wide (Build Settings › Terrain › Width), set Scale to `25 / 2500` (ie, `0.01`). This scales the Constant into a 25 x 25 m block. To set the block's height, turn on the **Height Remap** modifier and lower its upper value: the terrain's full height is the Height in Build Settings › Terrain, so a 50 m block on a 1000 m tall terrain is `0.05`.

Now you have a scale block.

To view it on your terrain, add a Combine node set to Add, with the terrain in its first input and the block in its second.

If you want to toggle this block and use it frequently in various places across your graph, use the Switch utility node. Feed your main terrain into its `In` port and the combined output into its `Alternate` port. The Switch node's **Switch Route** toggle then quickly shows and hides the version with the block.

> [!NOTE]
> Send the Transform's output through a [portal](../../ui/graph/organization/portals-and-chokepoints.md) to reuse it wherever needed in the graph without cluttering the workspace.

### Creating a Custom Reference Object

If you want to use a custom mesh instead of a block, use the Object node to import a mesh and scale it instead of the Constant node.
