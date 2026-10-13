# Masks

Masks are powerful tools used to control and refine where and how various effects and modifications are applied to a terrain. By using masks, you can selectively influence areas of your terrain, enhancing the precision and complexity of your designs.

## What is a Mask?

A mask in Nerthus is essentially a grayscale image where white areas represent sections of the terrain that are fully affected by an operation, black areas are unaffected, and gray areas receive partial effects based on the grayscale value.

## Creating and Applying Masks

### **Creating a Mask**

* Masks can be generated using any node that outputs a grayscale image. Common sources for masks include specific terrain features such as slopes, altitudes, or noise patterns.
* Use nodes like `Height`, `Slope`, or custom `Noise` patterns to generate masks based on terrain characteristics.

### **Direct Masking**

* To apply a mask, connect the mask output from one node into the mask input of another node. This setup allows the second node’s effects to be limited to the areas defined by the mask.
* For example, if you want to apply erosion only to the upper regions of a mountain, use a height-based mask that isolates those areas.

### **Post-Masking**

* The `Mask` node allows you to mask the node/effect **after** it has been created. You can connect the Mask node after an effect has been applied. For example, in a `Mountain > Erosion > Mask` scenario, the Mask will automatically use the applied mask between Mountain and Erosion.
* If you have multiple nodes that create the "effect" you wish to mask, just connect the second port of the Mask node to the desired "before" node. For example, in `Mountain > Erosion > Sandstone > Thermal2 > Mask`, you would connect the second input to Mountain.
* This is very useful because when a node is masked directly, a change to the mask will force a rebuild of the entire node. A Mask node, however, adds masking as a post-process and is extremely fast.

> [!NOTE]
> There is no difference in results between the Mask port on a node and the Mask node. It is highly recommended that Mask **node** be used when possible as it will make your builds faster.

## Practical Uses of Masks

### **Selective Modification**

Selectively apply specific effects like erosion, deposition, or texturing across the terrain. Masks will ensure that these effects enhance the detail and realism of the terrain without overwhelming the entire landscape.

### **Layering Effects**

By using multiple masks with different nodes, you can layer effects on a terrain. This layering can create complex interactions between different processes, such as having erosion only affect the areas not covered by vegetation.

### **Controlling Intensity**

Adjust the intensity of effects in different areas by modifying the grayscale values of the mask. Lighter areas can have more pronounced effects, whereas darker areas will have less or none.

### Tips for Effective Masking

* **Soft Edges**: When possible, use masks with soft edges to create natural transitions between affected and unaffected areas.
* **Test and Refine**: Experiment with different mask configurations and monitor the results. Adjust the mask's properties to fine-tune how it influences the terrain.
* **Combine Masks**: For complex terrains, consider combining multiple masks using blending nodes. This approach can give you more control over how different effects interact on the terrain.
