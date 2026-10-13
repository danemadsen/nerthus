# TextureBase

Creates a mask simulating natural material distribution onto which color from [SatelliteMap](../colorize/satellite-map.md) or [CLUTer](../colorize/cluter.md) or other color node is "flowed". Useful for driving texturing, shading, or erosion masks with a natural, layered look.

## Color Lookup

Color texture generation in Nerthus is based on color lookup tables, or CLUTs. A CLUT maps grayscale values from a heightfield, mask, or data map to a color ramp. Dark values use one end of the ramp, bright values use the other end, and intermediate values sample the colors between them.

When TextureBase is used before [SatelliteMap](../colorize/satellite-map.md), [CLUTer](../colorize/cluter.md), [Synth](../colorize/synth.md), or another color node, it acts as the mask that controls where those colors appear. Instead of coloring only by raw elevation, the color node follows TextureBase's natural material distribution, including slope response, flow-like streaks, soil buildup, patches, and random breakup.

Use this workflow when a direct height-based color ramp feels too clean or uniform. TextureBase gives the color lookup a more terrain-aware input, so rocks, soil, sediment, and vegetation colors can separate into more believable regions.

## When to Use It
Use TextureBase when you want a broad natural-distribution mask that gives SatelliteMap, CLUT, or other color nodes a believable base to follow.

## Properties

### Structure

#### Slope

Controls the influence of terrain slope on the texture, allowing highlights or patterns to follow steeper areas.

#### Flows

Adds flow-based features to the texture, simulating the effect of water or material movement across the surface.

#### Soil

Introduces soil-like patterns or noise, adding organic variation to the texture.

#### Patches

Controls the presence and size of patchy areas within the texture, creating irregular or clustered features.

#### Chaos

Adjusts the randomness and disorder in the texture, increasing natural variation and reducing uniformity.

#### Peaks

Emphasizes or introduces features that follow terrain peaks, adding detail to high areas in the texture.

### Processing

#### Accentuate

Enhances specific features or contrasts in the texture, making details more pronounced.

- **Off**: No accentuation is applied; the texture remains unmodified.
- **Old**: Applies the legacy accentuation method, increasing contrast and detail using the previous algorithm.
- **New**: Uses the latest accentuation method for improved feature enhancement and clarity.

#### Enhance

Applies additional enhancement to the texture's tonal range or contrast.

- **None**: No enhancement is applied.
- **Autolevel**: Automatically adjusts the levels to maximize the full tonal range, improving contrast.
- **Equalize**: Equalizes the histogram, distributing intensity values more evenly for a balanced look.

#### Reverse

Inverts the black and white values of the texture, swapping dark and light areas.

#### Seed

Randomization seed for generating the texture. Changing the seed produces different pattern variations.
