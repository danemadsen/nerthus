# RGBMerge

RGBMerge combines three heightfields into a single RGB color output. It is the counterpart to RGBSplit: where RGBSplit extracts channel data from a color texture, RGBMerge packs separate scalar maps into one RGB image for export, interchange, or compact storage.

## When to Use It
Use RGBMerge when multiple scalar maps need to be packed into one RGB texture for export, interchange, or compact storage.

## Common Uses

* Packing multiple masks into one file to reduce asset count and I/O overhead.
* Exporting splat or control maps where downstream tools expect channel-packed data.
* Building utility textures for pipelines where R, G, and B each represent a different control field.
* Creating a combined preview texture to quickly visualize how three maps relate spatially.

## How It Works

RGBMerge takes three separate heightfields and assigns them to the red, green, and blue channels of the output image. Each input is treated as a 0-1 scalar range and becomes grayscale data stored into its channel.

If your inputs are not already normalized, you should decide whether to normalize explicitly with Remap, Clamp, or Levels before RGBMerge, or use the per-channel enhancement options to improve utilization of the available channel range.

## Channel Enhancement

RGBMerge can optionally enhance each channel independently before packing. This is useful when the three source maps have very different ranges or distributions.

* None: preserves the channel exactly as provided.
* Autolevel: stretches values to fill the available range, increasing contrast and making better use of 8-bit exports.
* Equalize: redistributes values to a more uniform histogram, often revealing midtone detail and improving balance.

When to use each option:

* None: when channel values are already calibrated or you need numerical fidelity; recommended for data maps that must match exact thresholds.
* Autolevel: when a channel looks flat or low-contrast and you want a quick normalization without adding extra nodes; helpful when exporting to formats where precision is limited.
* Equalize: when detail is present but bunched into a narrow band; useful for revealing structure in masks, but can change the meaning of values more aggressively than Autolevel.

## Workflow Tips

* Decide what each channel means early and keep it consistent. Example packing conventions: R = flow, G = deposition, B = wear. Consistency matters more than which channel is which.
* Prefer explicit preprocessing when values matter. If a downstream shader or tool expects specific numeric ranges, do your Remap or Clamp work before RGBMerge and keep enhancement set to None.
* Be careful with equalization for control masks. It can be great for visual detail, but it may shift thresholds and relative weights in ways that break tuned blends.
* If you must export 8-bit textures, Autolevel can reduce banding by using more of the 0-255 range, but it also changes absolute meaning. For precision-sensitive maps, export higher bit depth where possible and keep enhancement off.
* Treat packed masks as linear data. In the target tool or engine, ensure the texture is imported as non-color or linear, often with "sRGB off", otherwise the channels will be gamma-altered and mask behavior will be wrong.
* Validate round-tripping. If you intend to split later, do a quick RGBSplit test on the merged output to confirm your packing, bit depth, and import settings preserve the data.

## Common Mistakes

* Viewing the packed RGB texture as a color image and trying to "make it look right". A good packed map often looks strange as an image. What matters is channel correctness.
* Letting the engine treat the packed map as sRGB. This is the most common cause of mismatched mask thresholds after import.
* Using enhancement to compensate for upstream range issues you actually want to control. If that behavior matters, normalize explicitly before packing so the result stays predictable.

## Properties

#### Enhance r

Selects the enhancement method for the Red channel before merging. Adjusts the contrast and distribution of red values to improve the final output.

- **None**: No enhancement is applied to the Red channel. The input values are used as-is.
- **Autolevel**: Automatically stretches the Red channel's values to use the full available range, increasing contrast.
- **Equalize**: Redistributes the Red channel's values to achieve a more uniform histogram, enhancing detail and balance.

#### Enhance g

Selects the enhancement method for the Green channel before merging. Adjusts the contrast and distribution of green values to improve the final output.

- **None**: No enhancement is applied to the Green channel. The input values are used as-is.
- **Autolevel**: Automatically stretches the Green channel's values to use the full available range, increasing contrast.
- **Equalize**: Redistributes the Green channel's values to achieve a more uniform histogram, enhancing detail and balance.

#### Enhance b

Selects the enhancement method for the Blue channel before merging. Adjusts the contrast and distribution of blue values to improve the final output.

- **None**: No enhancement is applied to the Blue channel. The input values are used as-is.
- **Autolevel**: Automatically stretches the Blue channel's values to use the full available range, increasing contrast.
- **Equalize**: Redistributes the Blue channel's values to achieve a more uniform histogram, enhancing detail and balance.
