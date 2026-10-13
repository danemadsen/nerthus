# Render Modes

The viewport draws the terrain in one of three styles, chosen from the toolbar's render style button or Preview › Preview Renderer:

- **Realistic Render**: the terrain lit by the sun and sky, with shadows and ambient occlusion. Use it to judge the look of a terrain.
- **Clay Render**: the terrain as uniform clay, lit to show its shape. Use it to judge forms without color distracting.
- **Visualize Data (Render Off)**: the data as it is, without lighting: heights in grey, colors as they are. Use it to read values rather than looks.

The lighting button opens the sun, sky and fog settings, which Realistic Render uses; **Reset Sky** puts them back.

## Colors and Masks

A node that makes colors shows them on the [underlay](../../graph/basic-workflow/underlays.md) terrain. A mask shows on the underlay terrain in the mask gradient, chosen in the lighting panel's **Visualization** group. The [2D viewport](2d-viewport.md)'s **Mask Enhancement** shows masks raw, autolevelled or equalized, so that faint masks are easier to read.
