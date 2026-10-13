# Adjust

The Adjust node is pretty much the same as the Modifier Stack, but in node form. It is often a process of trial and error where am adjustment node would need to be replaced with another. Sometimes the node would need to be chained with another.

This node makes that process easier by combining all the common adjustments inside a single node and allowing you to toggle them on and off. Additionally, it has its own [Modifier Stack](../../../ui/interface/property-editor/modifier-stack.md) so you can apply further adjustments.

## When to Use It
Use Adjust when you want modifier-stack style height remapping in the graph without committing to a baked upstream result.

## Properties

#### Multiply

Multiplies each value by a specified factor.

#### Shaper

Applies a shaping function to modify the curve of values.

#### Clamp

Restricts values to a specified minimum and maximum range.

#### Clip

Limits values by cutting off those outside a given range.

#### Drop

Removes values below or above a certain threshold.

#### Autolevel

Automatically adjusts values to span the full output range.

#### Equalize

Redistributes values to achieve a more uniform distribution.

#### Strong

Enhances the contrast or intensity of values.

#### Invert

Flips values to their opposite within the range.
