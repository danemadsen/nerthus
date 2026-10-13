# Height

The Height node creates a selection mask based on terrain elevation, isolating areas within a specified height range. It is a core selector for building altitude-based effects such as snow lines, vegetation bands, shoreline masks, plateau targeting, and biome zoning.

The output is a grayscale mask where white represents selected elevations and black represents excluded elevations, with optional soft transitions at the boundaries.

## When to Use It
Use Height when you need a selection mask based on terrain elevation, isolating areas within a specified height range.

## Common uses

* Vegetation zoning by altitude bands.
* Beach and shoreline isolation near sea level.
* Targeting mid-elevation shelves for terraces, erosion tuning, or deposits.
* Creating stacked biome masks by splitting a terrain into multiple elevation ranges.

## Usage tips

* Decide whether your workflow is absolute or relative. If your terrain height range changes upstream, normalize or remap before using Height so the same range settings keep working.
* Use Falloff to avoid harsh edges. A small falloff produces crisp layers while a larger falloff produces natural transitions that blend better with erosion, texture mixing, and scattering.
* Combine Height with other selectors for realism, for example, Height + Slope for snow that avoids steep faces, or Height + Flow for wetness near rivers at lower elevations.
* If you need multiple bands, duplicate the node and stagger ranges with slight overlap and falloff to avoid visible seams.


## Properties

#### Range

Sets the minimum and maximum elevation values to include in the mask. Only terrain within this height range is selected.

#### Falloff

Controls the softness of the mask transition at the range boundaries. Higher values create a smoother, more gradual blend from selected to unselected areas.
