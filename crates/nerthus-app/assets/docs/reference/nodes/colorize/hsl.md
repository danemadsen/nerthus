# HSL

The HSL node adjusts Hue, Saturation, and Lightness of a color input. It is used to shift color palettes, intensify or mute colors, and brighten or darken a texture without changing its structure.

HSL is most commonly used when preparing or tuning color textures for export, or when standardizing the look of multiple texture sources so they match.

## When to Use It
Use HSL when you want to shift hue, saturation, or lightness on a color texture without repainting the texture from scratch.

## What It's Used For

* Recoloring an albedo or basecolor map to fit a different biome or art direction.
* Reducing saturation to make a texture less "gamey" or less noisy.
* Subtle lightness changes to match value ranges across a texture set.
* Quick variations from a single source texture without reauthoring.

## Practical Tips

* Prefer small hue shifts. Large hue changes tend to produce unnatural results unless the source is already close to the target palette.
* If you are using a texture as data (masks, roughness, metalness, AO, normals), avoid HSL. Use scalar remapping tools instead and keep the map linear.
* When exporting to a renderer or engine, confirm the texture is treated as color (sRGB) if it is albedo or basecolor. If it is imported as linear by mistake, saturation and lightness will not match what you saw in Nerthus.
* If you are trying to fix "too dark" or "washed out," check gamma or import settings first. HSL can correct appearance, but it won't fix a broken color-management pipeline.

## Properties

#### Hue

Adjusts the hue of the input, shifting colors around the color wheel. Changing this value alters the overall color tone.

#### Saturation

Controls the intensity or vividness of colors. Higher values produce more vibrant colors, while lower values result in muted or grayscale tones.

#### Lightness

Sets the brightness of the colors. Increasing lightness makes the image lighter, while decreasing it darkens the image.
