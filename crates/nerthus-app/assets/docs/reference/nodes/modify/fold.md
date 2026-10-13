# Fold

The Fold node simulates terrain folding by introducing slants, breaks, and folds to the terrain. It will change the shape of the terrain significantly and should be used early in the graph.

## When to Use It
Use Fold when you want to introduce strong bent, slanted, or folded structure early in the graph.

## Properties

#### Waveform

Specifies the type of waveform used to apply the fold or wave transformation to the terrain. The waveform determines the shape and style of the folds.

- **Sine**: Applies a smooth, sinusoidal wave pattern, creating gentle, rolling folds.
- **Triangle**: Applies a triangular wave pattern, resulting in sharp, angular folds with linear transitions.
- **Sawtooth**: Applies a sawtooth wave pattern, producing repeating, ramp-like folds with abrupt drops.

#### Folds

Controls the number of folds or wave repetitions applied across the terrain. Higher values create more frequent, closely spaced folds.

#### Symmetric

Determines whether the folds are mirrored symmetrically across the center, producing balanced, repeating patterns.
