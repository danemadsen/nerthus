# Mask

The `Mask` node allows you to mask the node/effect **after** it has been created. You can connect the Mask node after an effect has been applied. For example, in a `Mountain > Erosion > Mask scenario`, the Mask will automatically use the applied mask between Mountain and Erosion.

> [!WARNING]
> If you're looking for a node to draw a mask by hand, see [Draw](../primitive/draw.md).

If you have multiple nodes that create the "effect" you wish to mask, just connect the second port of the Mask node to the desired "before" node. For example, in `Mountain > Erosion > Sandstone > Thermal2 > Mask`, you would connect the second input to Mountain.

## When to Use It
Use Mask when an effect is correct overall but needs to be applied only in selected areas after it has already been generated.

## Why a Mask Node

This is very useful because when a node is masked directly, a change to the mask will force a rebuild of the entire node. However, a Mask node adds masking as a post-process and is extremely fast.

> [!NOTE]
> There is no difference in results between the Mask port on a node and the Mask node. It is highly recommended that Mask **node** be used when possible as it will make your builds faster.
