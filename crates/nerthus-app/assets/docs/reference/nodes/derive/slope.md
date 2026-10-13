# Slope

The Slope node creates a selection mask based on the terrain's slope angle, isolating areas within a specified slope range. It is commonly used to target steep cliffs, gentle hillsides, and transitional faces for erosion control, material placement, and biome logic.

The output is a grayscale mask where white represents slopes inside the range and black represents slopes outside it, with optional soft transitions at the boundaries.

## When to Use It
Use Slope when you need a selection mask based on the terrain's slope angle, isolating areas within a specified slope range.

## Common uses

* Cliff masking for rock materials and sharp detail passes.
* Excluding steep faces from snow, vegetation, or soil accumulation.
* Targeting mid-slope zones for talus, scree, or weathering overlays.
* Driving erosion strength or deposition limits based on incline.
* Creating slope-aware blends for albedo, roughness, and displacement.

## Usage tips

* Start by finding your cliff threshold. In many terrains, a single slope band can separate "cliffs" from "walkable" surfaces cleanly.
* Use Falloff to avoid hard material seams. A small falloff yields crisp cliff edges; a larger falloff creates more natural blending, especially after erosion.
* Choose the slope type based on intent. Accurate is better for physically plausible selection and preserves real terrain variation; Normalized is useful when you want a more even response across the slope range or a more stylized, controllable mask.
* Use Micro Accent sparingly. It can help pick up subtle ridges and small breaks in slope for rock breakup and micro-masking, but too much can introduce noisy selections that fight downstream blends.
* Combine with Height for stronger zoning. Height + Slope is a common pairing for snowlines, rock exposure, vegetation rules, and shoreline cliff detection.

## Properties

#### Range

Sets the minimum and maximum slope angles to include. Only terrain within this range is selected in the mask.

#### Falloff

Controls the soft transition at the range boundaries. Higher values create a smoother, more gradual blend from selected to unselected areas.

#### Type

Defines the slope calculation method, affecting how the mask interprets and distributes slope values.

- **Accurate**: Uses precise slope computation for more realistic, detail-preserving results.
- **Normalized**: Produces a consistent, even distribution across the slope range, useful for stylized or uniform masks.

#### Micro accent

Enhances fine details and subtle variations in the slope mask, adding extra texture and definition to small-scale features.
