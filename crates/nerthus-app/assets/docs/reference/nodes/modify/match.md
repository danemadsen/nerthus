# Match

The Match node is a low-level lets you provide a Reference input that is used to adapt/match the heights of the main Input.

This is useful when you need terrains to be a precise height, but lose some heights after a particular effect, erosion, or other processes.

> [!TIP]
>  See [Match or Correct Heights](../../../guides/scenarios/helpful-info/match-height.md) for a practical example of how Match can be utilized.

## When to Use It
Use Match when one terrain needs to inherit the height range or elevation character of a reference input.

## Properties

#### Mode

Specifies the method used to match the height of the current terrain to a reference terrain. Determines how the height adjustment is performed.

- **Height**: Directly matches the height values of the current terrain to the reference terrain, aligning elevation point-for-point.
- **Histogram**: Matches the height distribution (histogram) of the current terrain to that of the reference terrain, preserving relative features while adjusting overall elevation statistics.
