# CutNoise

CutNoise makes a noise of sharp, cut-like creases: three Perlin noises of different scales, where only the lowest of them shows at each point. The result is a low-lying field (up to a quarter of the full height) of crisp ridges and valleys.

## When to Use It
Use CutNoise when you need a creased, faceted noise for breakup or detail, rather than the soft rolling shapes of plain Perlin noise. Connect a terrain or mask to its input to multiply the noise by it, which keeps the creases where the input is high.

## Properties

#### Scale

The size of the noise's features. Higher values make larger creases.

#### Seed

Randomization seed for generating unique variations of the noise.
