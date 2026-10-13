# ColorThreshold

ColorThreshold creates a grayscale mask from a color input by selecting pixels close to a chosen color. It is useful when you want to isolate specific parts of a color map, such as vegetation, rock, or snow, and turn that selection into a mask for further processing.

The Color property sets the color to match. Falloff controls how wide the match is. A low Falloff selects only colors very close to the chosen value, while a higher Falloff includes a broader range of similar shades and creates a softer result.

## When to Use It
Use ColorThreshold when you have a color map and need a mask of the areas close to one of its colors.

## Properties

#### Start

The color to mask from the given input.

#### Falloff

Range of similar colors to be masked as well.
