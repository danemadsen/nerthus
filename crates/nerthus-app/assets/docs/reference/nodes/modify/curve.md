# Curve

Curve adjusts the height curve of a terrain.

## When to Use It
Use Curve when you want precise control over how heights are remapped, for example to flatten valleys or sharpen peaks along a curve you draw.

## Properties

#### Curve spline

Defines the shape of the curve or spline used to remap or adjust values. The curve controls how input values are transformed, allowing for custom response profiles or non-linear adjustments.

#### Mode

Specifies how the curve is applied to the input values.

- **Absolute**: Applies the curve using absolute input values, mapping the entire range directly according to the curve shape.
- **Relative**: Applies the curve relative to the input's current range or context, allowing for adaptive or proportional remapping.

#### Fit Curve

Automatically fits or scales the curve to match the input range, ensuring the curve covers the full extent of the data.
