# Combine

The Combine node is one of the most important nodes in Nerthus. It allows you to take two outputs (terrains, masks, or color maps) and combine them using various mathematical operations, often known as 'Blend Modes'.

In addition, the Combine node also gives you a separation mask for texturing purposes when working with terrains and masks.

## When to Use It
Use Combine when two terrains, masks, or color maps need to be blended, compared, or merged into one controllable result.

## Using Combine
> [!NOTE]
> Min and Max blend modes compare Input A and Input B per pixel. Max keeps the higher value from the two inputs, while Min keeps the lower value. This is different from the Min and Max modifiers, which compare a node's processed result against its own incoming terrain.

> [!NOTE]
> Combine is deceptively simple, but may be the single most important node in Nerthus.

**Color Mixing**

Combine can also blend color maps, which gives you additional flexibility.

Combine will switch to color blending mode when the first input is a color map. The second should be a color map too. If the second input is a mask or heightfield it will be converted into an RGB color map.

Inversely, if the first input is a heightmap or mask and the second is a color map, the second map will be converted into a grayscale mask.

**Example**

This is an example of equally blending (Mode: Blend, Ratio: 50%) Perlin and Mountain nodes.

If you switch the mode to Max and Ratio to 100%:

If you plug in a shape of any kind to the Mask input of the Combine node, the brighter parts of the mask will be replaced with the first input, while darker areas will be replaced with the second input. If your mask is soft (ie - lots of grays instead of just black and white) then the blending between the two will be soft. This is usually the preferred method. 

## Additional Scenarios

**Clamped Output**

By default, Combine will clamp the output between `0..1`. This is a safety precaution for mixing masks and other values where the value scale is not readily apparent and can cause issues in other nodes.

If you're adding two terrains where the combined output goes beyond `1.0`, it will return flat portions in such areas. To prevent this, you can turn off `Clamp Output`. However, to properly use this data you must bring it within the "acceptable range". To do this, you should enable `Autolevel (Lv)` in the [Modifier Stack](../../../ui/interface/property-editor/modifier-stack.md) and then `Clamp`. This will bring the whole mask or terrain within the safe range.

Alternatively, you can add an Fx node to the Combine output or other nodes like Clamp and bring it down manually.

## Properties

#### Ratio

The ratio applied to the selected method.

#### Mode

- **Blend**: Simply overlays the source on top of the base without extra math. Direct normal blend; top layer replaces base with alpha control.
- **Add**: Brightens by adding the source to the base. Linear addition of source and base channels, clamped at max.
- **Screen**: Lightens by blending as if projected on a screen. Inverts both, multiplies, then inverts; 1 - (1 - A) × (1 - B).
- **Subtract**: Darkens by subtracting the source from the base. Base minus source per channel, clamps at zero.
- **Difference**: Emphasizes contrast by subtracting lower from higher values. Absolute difference |A - B| for high-contrast edge-like results.
- **Multiply**: Darkens by multiplying source and base colors. Channel-wise multiplication; A × B, darkens with stacking.
- **Divide**: Lightens by dividing base by source. Base divided by source; A / B with normalization/scaling.
- **Divide2**: Alternative division with different scaling for smoother results. Alternate divide with different scaling/offset to reduce clipping.
- **Max**: Keeps the brighter value at each pixel. Channel-wise maximum; keeps highest of A and B.
- **Min**: Keeps the darker value at each pixel. Channel-wise minimum; keeps lowest of A and B.
- **Hypotenuse**: Combines channels using geometric distance for soft blending. sqrt(A² + B²); Euclidean combination for smooth, bright blend.
- **Overlay**: Combines multiply and screen for vivid contrast. Combines Multiply and Screen based on base brightness.
- **Power**: Raises base to the power of source for nonlinear effects. Raises base to source power; A ^ B per channel.
- **Exclusion**: Low-contrast version of Difference for softer inversion. A + B - 2AB; softer Difference with reduced contrast.
- **Dodge**: Brightens highlights by dividing inverted base by inverted source. A / (1 - B); brightens highlights aggressively.
- **Burn**: Darkens shadows by inverting, dividing, and inverting again. 1 - (1 - A) / B; intensifies shadows via division.
- **SoftLight**: Gently lightens or darkens based on source brightness. Conditional blend approximating diffused light on surface.
- **HardLight**: Strong contrast blend combining multiply and screen. Screen or Multiply based on source brightness threshold.
- **PinLight**: Replaces pixels selectively with lighter or darker values. Replaces mid-range with source; keeps extremes from base.
- **GrainMerge**: Adds source to base with mid-gray treated as transparent. A + B - 0.5; mid-gray = no change; good for texture merge.
- **GrainExtract**: Subtracts source from base, centering results on mid-gray. A - B + 0.5; isolates texture/detail difference.
- **Reflect**: Creates intense highlights by reflecting colors around white. A² / (1 - B); creates sharp highlight "reflection" effects.
- **Glow**: Emphasizes bright regions for a glowing effect. Inverse Dodge-like; emphasizes light bloom by brightening source areas.
- **Phoenix**: Produces a fiery inversion by subtracting and swapping values. A - B + max(A, B); surreal color inversions with swap-like logic.

#### Output

How to process the data range.

- **None**: Retain pure, unclamped data. Less safer, but useful in advanced usage.
- **Clamp**: Clamp the data to 0..1 range. Slightly slower, but recommended for safety.
- **Extend**: Extend to fill the range.

#### Swap Inputs

Switches the two inputs.

#### Add Input

#### Enhance

Post-process the output.

- **None**: No post-processing. Recommended.
- **Autolevel**: Autolevel the output.
- **Equalize**: Equalize the output.
