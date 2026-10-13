# Trees

Trees creates superficial forests and vegetation. These forests are part of the heightfield while being "detached" enough for various export scenarios.

## When to Use It
Use Trees when you want forest or vegetation massing to appear as part of the heightfield for distant coverage, canopy breakup, or export-friendly vegetation forms.

## Properties

### Trees

#### Output

Determines the output type for tree placement. 'Instances' provides individual tree positions for rendering and export. 'AreaMask' is an experimental feature that outputs a mask of tree coverage, useful for blending or exclusion but may be less reliable.

- **Instances**: Outputs individual tree instance data for use in rendering, simulation, or export. Recommended for stable and reliable results.
- **AreaMask**: Outputs a mask indicating areas where trees are placed. Useful for visualization and blending, but currently experimental and may produce less predictable results.

#### Count in Thousands

Sets the number of trees to be placed, specified in thousands. Higher values increase density and mask quality but may impact performance and memory usage.

#### Is relative

If enabled, tree count scales with terrain resolution for consistent density. Recommended for predictable results at higher resolutions, but may increase resource usage.

#### LOD info

Provides Level of Detail (LOD) information for tree instances, enabling optimized rendering and export at different distances or resolutions.

#### Tree size

Controls the height of each tree instance. Affects the appearance of exported trees but not the mask. Useful for adjusting the final look of tree outputs.

#### Trim under

Removes tree instances below a certain size threshold, clamping down on smaller or weaker trees to refine the mask and output.

#### Shape

Specifies the distribution pattern or type of tree shapes. 'Point' is a single pixel, 'Big' is a larger square, and 'BigOnly' restricts placement to large clusters. 'Big' is preferred for mask export, while 'Point' is better for visualization.

- **Point**: Places trees as single small pixels, resulting in sparse and detailed distribution. Best for internal visualization.
- **Big**: Places larger tree shapes, creating denser and more legible masks. Preferred for exporting tree masks.
- **BigOnly**: Restricts placement to only large tree clusters, excluding smaller or individual trees for a bold mask.

#### Seed

Sets the randomization seed for procedural tree placement. Changing the seed produces different tree layouts and simulation results.

### Growth

#### Seek Water

Controls whether trees grow near water flows ('Seek') or away from them ('Avoid'). Useful for simulating species preferences and biomes.

- **Seek**: Trees preferentially grow near water sources and flow areas, simulating water-loving species.
- **Avoid**: Trees avoid water sources and flow areas, simulating species that prefer drier conditions.

#### Avoid trees

Masks out areas occupied by other tree nodes, useful for stacking multiple tree species or biomes without overlap.

- **None**: No avoidance; trees can be placed close together or overlap with other tree nodes.
- **Previous**: New trees avoid areas occupied by the previously placed tree node, reducing overlap in stacked nodes.
- **All**: New trees avoid all other tree nodes in the stack, maximizing spacing and separation.

#### Health

Controls the duration and vigor of the tree simulation. Higher health increases density and allows trees to grow into smaller valleys and more challenging terrain. Lower health restricts growth to main flow areas.

#### Patches

Clumps trees together into larger groups, creating stylized patches instead of individual trees. Useful for natural-looking forests.

#### Spread

Blurs and expands the area where trees can grow, merging small flows into larger masses and creating more natural, contiguous forest areas.

#### Random

Adds random noise to tree placement, scattering trees across the terrain without regard to inhibition or aspect controls. Rarely used for realistic masks.

### Aspect Inhibition

#### Slope

Restricts tree growth on steep terrain. Not recommended when using a custom inhibition mask, as the mask provides more accurate control.

#### Slope falloff

Controls how quickly tree density decreases as slope increases. Use with slope inhibition for gradual or sharp transitions.

#### Altitude

Sets altitude limits for tree placement, simulating elevation-based growth restrictions. Works well with both aspect inhibition and custom masks.

#### Altitude falloff

Controls the rate at which tree density decreases with altitude, enabling smooth or sharp transitions in elevation-based inhibition.

#### Peaks

Removes tree growth from exposed mountain peaks, preventing trees from growing in the most exposed areas.

#### Strict

Enforces strict adherence to inhibition rules, resulting in sharper falloff and less natural tree distribution. Use for precise, non-organic masks.

### Snow Inhibition

#### Dead flow

Represents areas where tree growth is suppressed due to snow mass or adverse conditions. Controls the amount of snow mass moving down the terrain.

#### Consolidate flows

Groups snow flows into larger masses, affecting how far snow moves down the terrain and influencing tree inhibition.

#### Bias

Directs snow flows to move less or more, affecting the inhibition of tree growth in snowy areas.

#### Snowline

Defines the altitude at which snow flows start, restricting tree growth above this line.

### Artistic Control

#### Chaos

Warps tree positions for a more natural, randomized look, breaking up straight flows and adding organic variation.

#### Blur edges

Softens and tapers the edges of tree patches, blending them into the terrain for a more natural transition.

#### Visualize data

Enables visualization of tree placement, flows, and snow inhibition, assisting with mask tuning and debugging.

### Custom Inhibitors

#### Custom influence

Applies custom influence maps or masks to control tree placement according to user specifications, enabling highly tailored results.

#### Trim

Removes trees from specified areas, allowing for manual or procedural clearing of tree patches for refined mask output.
