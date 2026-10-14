# Modifier Stack

## Modifier Stack

Modifiers are additional adjustments you can add on top of most nodes. These Modifiers are post-processed onto the node's output, so you can modify them without having to rebuild the node.

Many of these Modifiers provide the same functionality as nodes of the same names. However, when you use a Modifier, you create less overhead than creating individual nodes for small adjustments.

The Modifier Stack can be used to re-order the Modifiers applied to the node. Often this may change the end-result when using impactful Modifiers such as Autolevel or Height Remap. Drag a modifier up or down by its grip (or anywhere outside its sliders) and drop it on another to take that one's place. Min and Max always stay last.

Hover a modifier to show its close button in the top right corner. Clicking it asks before removing the modifier; hold **Shift** to remove it straight away.

> [!NOTE]
> Modifiers are flexible, powerful, and memory efficient: they adjust a node's output without adding nodes to the graph.

In addition to the array of Modifiers available, three unique Modifiers exist that are not directly available as nodes: Influence, Min, and Max. 

## Modifiers

| Height       | Effects   | Blend Modes | Masks     |
| ------------ | --------- | ----------- | --------- |
| Autolevel    | Blur      | Max         | Influence |
| Clamp        | Warp      | Min         | Height    |
| Drop         | Shaper    | Difference  | Slope     |
| Equalize     | Threshold | Invert      |           |
| Height Remap |           |             |           |
| Match Height |           |             |           |
| Strong       |           |             |           |

For strategies on using Modifiers, see [Using Modifiers](../../../using/managing-graphs/using-modifiers.md).
