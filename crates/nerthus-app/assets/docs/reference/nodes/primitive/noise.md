# Noise

Noise applies controlled single-pixel noise onto the terrain.

## When to Use It
Use Noise when you need a controlled speckle or grain layer to disturb an otherwise clean terrain or mask.

## Properties

#### Noise type

Selects the algorithm used to generate noise. Each type produces a distinct pattern and character, suitable for different terrain or texture effects.

- **Random**: Generates pure random noise, resulting in a highly chaotic and unstructured pattern.
- **Perlin**: Produces smooth, gradient-based noise with natural transitions, ideal for organic terrain features.
- **Gaussian**: Applies a Gaussian distribution to the noise, creating patterns with a bell-curve intensity and softer transitions.
- **Fixed**: Generates a noise pattern based on large, finite shapes.
- **Micro**: Creates very fine, high-frequency noise suitable for adding subtle surface detail or micro-variation.

#### Height

Controls the maximum amplitude or elevation of the noise, affecting how pronounced the noise appears in the output.

#### Density

Adjusts the frequency or scale of the noise pattern. Higher density results in more frequent variations across the terrain.

#### Relative density

When enabled, density is scaled relative to the current resolution or context, maintaining consistent detail at different scales.

#### Blend mode

Determines how the generated noise is combined with the input or base layer, affecting the final appearance.

- **Add**: Adds the noise values to the input, increasing elevation or intensity where noise is present.
- **Multiply**: Multiplies the input by the noise values, modulating the base layer and enhancing contrast.
- **Subtract**: Subtracts the noise values from the input, reducing elevation or intensity in noisy areas.

#### Seed

Sets the randomization seed for the noise generation. Changing the seed produces a different noise pattern while keeping other settings the same.
