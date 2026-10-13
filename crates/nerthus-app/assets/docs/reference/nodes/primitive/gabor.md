# Gabor

Gabor shapes are dramatically different when compared to the common Perlin and Voronoi nodes. It can be used to create sand-like patterns, modulated hills, and more. Along with Perlin and Voronoi, we consider Gabor to be one of the foundation nodes of Nerthus.

## When to Use It
Use Gabor when you want a patterned noise, such as sand-like ripples or modulated hills, that Perlin and Voronoi do not give.

## Properties

### Gabor

#### Size

Controls the scale of the Gabor noise features. Larger values produce broader, more widely spaced patterns, while smaller values create finer details.

#### Entropy

Adjusts the randomness or complexity of the Gabor noise. Higher entropy values result in more chaotic and less predictable patterns.

#### Anisotropy

Determines the degree of directionality in the noise. Higher values stretch the pattern along a specific axis, creating elongated or striated features.

#### Azimuth

Sets the orientation angle (in degrees) for the anisotropic direction of the Gabor noise, controlling the alignment of elongated features.

#### Gain

Controls the amplitude or intensity of the Gabor noise, affecting the contrast and prominence of the generated features.

#### Seed

The randomization pattern or seed for the node's process. Changing the seed produces different variations of the Gabor noise.
