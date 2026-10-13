# Clamp

Clamp allows you to control the height of the terrain. You can use it to squash the terrain, clip it at certain heights, or make it taller using the Extend operation.

## When to Use It
Use Clamp when you need to cap, crop, or extend elevation values without rebuilding the larger landform.

## Properties

#### Mode

Specifies the clamping method used to restrict values within a certain range.

- **Standard**: Applies a fixed minimum and maximum clamp to the input values, preserving their original scale.
- **Normalized**: Clamps and remaps the input values to a normalized 0–1 range, ensuring consistent output regardless of the original scale.

#### Value

Sets the clamp threshold or range. Values outside this range are limited to the specified minimum or maximum.

#### Drop

Lowers the overall shape to lowest value, ie 'dropping' the entire shape to the 'floor'.
