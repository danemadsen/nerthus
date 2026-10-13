# Distance

Distance is a simple Distance Transform node that allows you to create a sloping shape from a flat shape: it measures how far each point is from the nearest boundary. It works very well in conjunction with nodes such as Mask, Island, and anything else that produces hard, flat shapes.

## When to Use It
Use Distance when you need a distance field from a shape or mask boundary for falloffs, spacing, bands, or growth zones.

## Properties

### Distance

#### Method

Specifies the algorithm used to calculate distance. Different methods may offer varying performance or accuracy.

- **Classic**: Uses the standard distance calculation method, suitable for most general purposes.
- **RT**: Uses a real-time optimized distance calculation, providing faster results for interactive workflows.

#### Mode

Determines the pattern or structure of the distance calculation, affecting how distance propagates from the source.

- **Asterisk**: Calculates distance in a star-shaped (asterisk) pattern, radiating out in multiple directions.
- **Pyramid**: Calculates distance in a pyramid or diamond pattern, emphasizing diagonal and orthogonal propagation.

#### Directions

Sets the number of directions in which distance is calculated, influencing the angular resolution and smoothness of the result.

#### Skew

Applies a skew or bias to the distance field, distorting the propagation direction for creative or technical effects.

#### Angle

Specifies the primary angle or orientation for directional distance calculations, allowing for anisotropic effects.

#### Angular jitter

Introduces randomness to the angle of distance propagation, creating more organic and less uniform distance fields.

#### Falloff

Controls how quickly the distance value decreases with increasing distance from the source, shaping the gradient of the field.

#### Threshold

Sets a cutoff value for the distance field. Values beyond this threshold are clamped or ignored.

#### Falloff jitter

Adds randomness to the falloff rate, producing irregular or natural-looking distance gradients.

### Options

#### Invert input

Inverts the input mask or field before calculating distance, reversing the source and background.

#### Invert output

Inverts the resulting distance field, swapping near and far values.

#### Multiply by input

Multiplies the final distance field by the original input, masking or modulating the result.

#### Seed

Randomization seed for any stochastic elements in the distance calculation, ensuring reproducible or varied results.
