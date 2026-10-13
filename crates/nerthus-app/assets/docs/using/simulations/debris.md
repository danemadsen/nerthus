# Debris

Simulate mechanical erosion with thousands (even millions) of individual, physical rocks and other geological debris using our physics-engine powered Debris simulation.

You can keep them in the heightmap, or export a point-cloud for later use. You can even color each rock individually.

## Emission Source

The Debris simulation lets you control the emission source so you can place the debris as needed. For example, connect the terrain's flow map to the **Emitter** input: the debris then forms only within those lanes and takes advantage of the inherent physics of the flows to create scree rivers.

Wear data from [ErosionPro](../../reference/nodes/simulate/erosion-pro.md) can often be a great source.

## Friction and Restitution

The Friction and Restitution controls determine how debris behaves when it falls and slides across the terrain. While we often describe it as controlling how far debris can travel, in the underlying physics, it’s about **energy loss** during movement:

* **Restitution**: Energy lost during impacts – how far debris bounces or slides after hitting a surface.
* **Friction**: Energy lost while sliding – how easily debris moves across the terrain.

So, adjusting friction changes how steep or shallow debris deposits will appear in your terrain simulations. Higher friction = steeper, shorter slides. Lower friction = longer, shallower debris spreads.

> [!NOTE]
> For natural rock surfaces, the friction coefficient averages around **0.62**. This value directly relates to the **slope of repose** – the stable angle at which debris piles up. A friction coefficient of 0.62 corresponds to a **32° slope**, which is the typical angle of a talus (debris pile) formed under gravity.

## Size, Shape, and Layering

You vary debris sizes and even layer multiple Debris simulations to mix and match settings and shapes. The Debris node provides Sharp and Round rock shapes.

See the Debris examples that ship with Nerthus to see how layering multiple simulations work.

> [!NOTE]
> When using multiple Debris nodes, you can use [Accumulators](../managing-graphs/accumulators.md) to combine all the output masks for colorization and other purposes.
