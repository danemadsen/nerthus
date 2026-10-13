# Splat

Splat creates a single heightfield by combining the channels of an RGBA color input. Each channel can be weighted, then blended into one scalar output. This is useful when you have multiple masks packed into a single texture and you want to derive one composite mask, or when you want to treat channel-packed data as a multi-source influence map.

## When to Use It
Use Splat when you want to combine several packed RGBA channels into one composite mask or influence map.

## What It's Used For

* Combining multiple packed masks into one selector mask.
* Building a composite distribution map from several influences stored in RGBA.
* Quick extraction-and-combine workflows when you do not need four separate outputs (use RGBSplit if you do).
* Creating a unified "coverage" mask from different authored regions.

## How It Works

* Splat reads the incoming R, G, B, and A channels.
* Each channel is multiplied by its corresponding amount control.
* The weighted channels are combined into a single heightfield output.
* Optional post-processing can normalize the result and keep it within valid limits.

The output is a scalar map in the same role as any other heightfield or mask, suitable for remapping, clamping, erosion masking, blending, and selectors.

## Practical Tips

* Use channel amounts as mix weights. Set unused channels to 0 to ignore them.
* If you are combining masks that already represent probabilities or coverage, keep weights modest to avoid saturating the output too early.
* If you need predictable thresholds downstream, consider clamping and remapping after Splat rather than relying only on automatic adjustments.
* Treat channel-packed masks as linear data end-to-end. In downstream tools, ensure the texture is not imported as sRGB, otherwise channel values will be gamma-altered.

> [!NOTE]
> on autolevel and clamping
>
> * Autolevel can be helpful when the combined output only occupies a narrow range and you want to recover contrast quickly. It is not ideal when absolute values carry meaning.
> * Clamp Product is useful when your combination can overshoot 1.0 and you want to guarantee a valid 0..1 output. This is especially relevant if you are multiplying influences implicitly through strong weights or tightly correlated channels.

## Properties

#### R

Amount of R input channel.

#### G

Amount of G input channel.

#### B

Amount of B input channel.

#### A

Amount of alpha input channel.

#### Autolevel

Automatically adjusts the levels of the combined channels to maximize the output range.

#### Clamp product

Clamps the product of the R, G, and B channels to the 0..1 range, ensuring the output does not exceed valid limits.
