# Perlin

Perlin is the most basic and essential noise. In Nerthus, we've made a "geo-variant" by modifying the base Perlin shape to fit the needs of terrain design from the beginning.

## When to Use It
Use Perlin when you need a dependable all-purpose base noise for blocking out terrain, testing erosion, or blending with other primitives.

## Properties

### Noise

#### Type

Specifies the style of Perlin noise to generate, affecting the overall appearance and structure of the noise pattern.

- **FBM**: Fractal Brownian Motion. Produces classic Perlin noise with layered octaves for natural, smooth terrain.
- **Ridged**: Generates ridged noise, emphasizing sharp crests and valleys for more dramatic, mountainous features.
- **Billowy**: Creates billowy, cloud-like noise with soft, rounded shapes and less pronounced valleys.

#### Scale

The scale of the noise. Higher values create larger, less detailed shapes. Lower values create noisier, highly detailed shapes.

#### Octaves

The number of noise octaves. Higher values create more complexity by layering additional detail at smaller scales.

#### Gain

Controls the amplitude reduction between octaves, affecting the persistence and contrast of the noise.

#### Height

Sets the maximum elevation or intensity of the noise, scaling the output vertically.

#### Seed

The randomization pattern or seed for the node's process. Changing the seed produces different noise patterns.

### Warp

#### Type

Specifies the style of Perlin noise to generate, affecting the overall appearance and structure of the noise pattern.

- **FBM**: Fractal Brownian Motion. Produces classic Perlin noise with layered octaves for natural, smooth terrain.
- **Ridged**: Generates ridged noise, emphasizing sharp crests and valleys for more dramatic, mountainous features.
- **Billowy**: Creates billowy, cloud-like noise with soft, rounded shapes and less pronounced valleys.

#### Frequency

Controls the frequency of the warp, determining how often the pattern is distorted across the terrain.

#### Amplitude

Sets the strength of the warp effect, controlling how far the noise is displaced.

#### Octaves

The number of noise octaves. Higher values create more complexity by layering additional detail at smaller scales.

### Transform

#### Scale x

The non-uniform scale of the X axis, stretching or compressing the noise horizontally.

#### Scale y

The non-uniform scale of the Y axis, stretching or compressing the noise vertically.

#### X

Offsets the Perlin noise along the X axis, shifting its position horizontally.

#### Y

Offsets the Perlin noise along the Y axis, shifting its position vertically.
