# ErosionPro

The **ErosionPro** node is a powerful erosion algorithm designed to enhance terrain shaping beyond the traditional Classic Erosion.

Despite its complexity, ErosionPro remains user-friendly and delivers deterministic results with up to 10x faster performance, even on the CPU.

## Using ErosionPro

The **ErosionPro** node offers extensive control over the erosion process, allowing users to shape terrain with a range of new options and refinements.

### Primary Control

This group of controls is the primary driver of the erosion simulation. Each setting here can strongly affect the final terrain.

#### Duration

The longer the **Duration**, the more elaborate the resulting terrain will be. However, it is not always necessary to use long values. When you want to preserve your terrain shape, it is better to use multiple shorter Duration ErosionPro nodes than one very high Duration node. If you are crafting the terrain shape through erosion, then a high Duration is useful.

#### Downcutting

Downcutting controls how deep flowing water cuts into the terrain, while **Erosion Scale** determines the largest gullies' size across the entire terrain. Both settings can easily affect each other, so experiment with your specific terrain.

#### Erosion Scale

Small values in **Erosion Scale** can create very fine, sharp gullies and crevices, while large values create large ravines and broad flow structures. You can use multiple ErosionPro nodes with different scales to create complex erosion. Nodes with higher Erosion Scale should usually precede nodes with smaller scales so subsequent erosion does not destroy the finer details.

### Sedimentary Control

Sedimentation in Nerthus simulates the transport and deposition of debris generated during erosion, such as soil, pebbles, and rocks. This process is governed by three distinct sedimentation types: **Suspended Load**, **Bed Load**, and **Coarse Sediments**. These types represent increasing sediment mass and decreasing mobility.

Each type has its own **Discharge Angle**, which controls the slope angle at which sediment begins to settle. The lighter the sediment, the lower the angle it can travel on. Heavier sediments require steeper slopes to move or be deposited.

> [!WARNING]
> All three types contribute to the overall sedimentation equally. Setting all sliders to 1.0 will not triple the effect. Instead, each type contributes about a third of the total sedimentation. They are also interlinked, often blending visually and physically.

#### Suspended Load

These are the finest and lightest sediments, typically silt and sand-like materials. They are extremely mobile and tend to stay suspended in flowing water, depositing only when the flow slows down. Because of this, they are the most active component in terrain reshaping, especially within gullies.

While visually they may seem like rocks tumbling downslope, the simulation treats them as a dynamic agent tied closely to erosion. They flow across even gentle slopes, such as 0-24 degrees, and often travel long distances before settling. Increasing Suspended Load results in subtle ripples and branching patterns within existing gullies, and with high **Duration** and **Downcutting**, can even generate entirely new gully systems.

#### Bed Load

These are heavier sediments, such as gravel and small rocks. They are less mobile than suspended sediments but still quite active. Bed Load is less sensitive to acceleration/deceleration and more influenced by average water flow and terrain friction. It moves farther than coarse sediment, even across flat areas, but deposits more readily than suspended load.

Bed Load generates deposits like talus and can visually fill in or enhance slope structures. Use it carefully: high values can dominate the erosion outcome and overwhelm finer detail.

Bed Load Discharge is also useful for filling gullies created by erosion. It is good compensation for higher Downcutting.

#### Coarse Sediments

These represent the heaviest material: rocks, boulders, and debris. They are the least mobile and tend to settle rapidly, usually requiring steeper slopes to begin movement. A discharge angle of 24 degrees for coarse sediments means they will be deposited only on or near such steep inclines; you will not see them on flat areas.

Coarse sediments produce the most dramatic deposits, such as scree, debris piles, and alluvial fans, and are ideal for shaping realistic sedimentation structures.

> [!NOTE]
> Sedimentation and Downcutting are deeply linked because sedimentation can be a direct product of Downcutting. Experiment with your terrain to see how Downcutting can become a force multiplier for sedimentation.

Bed Load and Coarse Sediments also create shapes that are favorable for adding Scree, Debris simulations, or [HueErosion](../../../reference/nodes/colorize/hue-erosion.md).

### Shape Control

This group of controls is useful for artistic control over the simulation, from shaping the peaks to controlling the level of detail.

#### Shape

The Shape control lets you choose between retaining the original shape at low values or allowing erosion to naturally reshape the peaks of the terrain at high values.

This processing is low-level thermal erosion wrapped within the hydraulic erosion simulation.

#### Shape Sharpness

In conjunction with Shape, the Shape Sharpness control lets you shape the terrain with more concave, sharp ridges. This is a useful way to get the right kind of look for your terrain.

The Shape Detail Scale control is important for surface structure. This control lets you choose how much concentric smaller detail is simulated, from the largest ravines to the finest gullies within them. This is useful for reducing unnecessary detail, especially for distant terrains or if you plan to add further detail using [Surface Nodes](../../crafting-the-surface/surface-nodes.md).

### Orographic Influence

Orographic influence allows you to limit where erosion is inhibited.

#### Enable

Toggle all orographic influences.

#### Directional Precipitation

This enables rain shadows, forcing precipitation to occur more intensely on one side.

#### Direction

The direction from which rainfall occurs more strongly. This may not completely stop rainfall in other directions, but reduces its intensity.

#### Rain Shadow

Directional rain can be influenced by other nearby peaks and structures. Rain coming from a specific direction may be blocked by the rain shadow. This control allows you to choose the strength of the rain shadow.

#### Altitude and Slope

These controls allow you to choose the range for both Height and Slope within which rainfall occurs. This can be used to create both realistic and highly artistic results.

#### Reverse

Inverts the influence.

## Faster Performance

Nerthus's ErosionPro can utilize either GPU or CPU. It has an intelligent fallback: if your GPU runs out of VRAM, it will run the erosion on the CPU where it may be slightly slower but less likely to run out of memory.

In every scenario, ErosionPro can outrun classic Erosion by several magnitudes.
