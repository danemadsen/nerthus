# Tint

Tint converts a heightmap or mask into a color texture by blending between two colors across the input range. Dark values map toward the start color, bright values map toward the end color. This is essentially a two-color lookup table (CLUT) applied to a scalar map.

Tint is useful for quick colorization, visualization of masks, and generating simple gradients for export or downstream blending.

## When to Use It
Use Tint when a heightmap or mask needs to become a simple two-color texture, for quick colorization or to see a mask more clearly.

## How it works

* Input values are interpreted as a gradient position from black to white.
* The output color is interpolated between the chosen start and end colors.
* Optional hue, saturation, and lightness shifts apply a global adjustment to the gradient colors.
* Optional processing can enhance or rebalance the input range before mapping, which changes how much of the gradient is used and where transitions land.

## Common uses

* Colorizing a mask to make it easier to read while authoring a graph.
* Creating quick biome gradients, altitude tints, or stylized ramp maps.
* Generating a simple "material ID" or region visualization for review.
* Producing a lightweight color overlay that can be mixed with more detailed textures later.

## Practical tips

* Normalize the input if you need predictable mapping. If the input only occupies a narrow range, most of the output will sit near one color unless you remap or use enhancement.
* Use reverse when you want to flip the interpretation without rewriting upstream logic, for example, when a mask is inverted relative to your intended tint.
* Autolevel is a good default for unknown inputs. It maximizes gradient usage quickly, but it also changes absolute meaning, so avoid it for calibrated masks.
* Equalize can reveal structure in subtle masks by redistributing values, but it may produce "busier" transitions than expected. Use it when detail visibility matters more than strict thresholds.
* Hue, saturation, and lightness are best treated as finishing controls. Set start/end first, then do small global shifts to tune the palette.

## Properties

### Tint

#### Start

Specifies the starting color of the gradient, mapped to the darkest (black) areas of the input map.

#### End

Specifies the ending color of the gradient, mapped to the brightest (white) areas of the input map.

#### Hue

Adjusts the hue of the gradient colors, allowing for global color shifts across the entire gradient.

#### Saturation

Controls the intensity or vividness of the gradient colors. Higher values produce more vibrant colors, while lower values result in muted tones.

#### Lightness

Sets the overall brightness of the gradient colors, making the output lighter or darker.

### Processing

#### Enhance

Applies additional enhancement to the gradient's tonal range or contrast before tinting.

- **None**: No enhancement is applied; the gradient is mapped directly from the input.
- **Autolevel**: Automatically adjusts the levels to maximize the full tonal range, improving contrast in the gradient mapping.
- **Equalize**: Equalizes the histogram, distributing intensity values more evenly for a balanced gradient effect.

#### Reverse

Inverts the gradient mapping, swapping the start and end colors so that black maps to the end color and white maps to the start color.
