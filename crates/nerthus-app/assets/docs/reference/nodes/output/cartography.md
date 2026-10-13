# Cartography

Cartography creates a topographical overlay for a map.

## When to Use It
Use Cartography when you need a map-style rendering of the terrain, with topographic lines over it.

## Properties

### Map Style

#### Accuracy

Controls the precision of map features such as contours and gradients. Higher values produce more detailed and accurate representations of terrain.

#### Gradient style

Specifies the style of gradient used to represent elevation or terrain features on the map.

- **Smooth**: Applies a continuous, smooth gradient for a natural transition between elevation bands.
- **LargeContours**: Emphasizes major elevation bands with broad, distinct contour regions.
- **AllContours**: Displays all contour lines, including minor and major bands, for maximum detail.

#### Equalize gradient

Balances the gradient distribution to ensure even representation of elevation changes across the map.

#### Reverse

Inverts the color or gradient mapping, swapping high and low elevation colors for a different visual effect.

#### Erode

Applies an erosion effect to the map, softening features and simulating natural terrain wear.

### Land

#### Land palette

Selects the color palette used for land areas on the map.

- **Classic**: Uses a traditional cartographic color scheme for land.
- **Minimal**: Applies a minimalistic color palette for a clean, modern look.
- **FertileRed**: Highlights fertile areas with red tones.
- **FertileGreen**: Highlights fertile areas with green tones.
- **RedBlueSpectrum**: Uses a spectrum from red to blue to represent elevation or land type.
- **MineralsYellow**: Emphasizes mineral-rich areas with yellow hues.
- **FertileCool**: Applies cool tones to fertile regions for a subdued effect.
- **FertileMurky**: Uses murky, earthy tones to represent fertile land.

#### Land contours

Controls the visibility and density of contour lines on land areas, highlighting elevation changes.

#### Land contour intenstity

Adjusts the brightness or opacity of land contour lines, making them more or less prominent.

#### Land bias

Applies a bias to the land color or contour mapping, shifting emphasis to certain elevation ranges or features.

#### Water palette

Selects the color palette used for water areas on the map.

- **Classic**: Uses a traditional blue color scheme for water.
- **BlackSea**: Applies a dark, deep blue palette reminiscent of the Black Sea.
- **Caribbean**: Uses bright, turquoise tones inspired by Caribbean waters.

#### Water contours

Controls the visibility and density of contour lines in water areas, indicating depth or underwater features.

#### Water contour intenstity

Adjusts the brightness or opacity of water contour lines, making them more or less prominent.

#### Water bias

Applies a bias to the water color or contour mapping, shifting emphasis to certain depth ranges or features.

#### Trim

Removes or crops map features outside a specified boundary, focusing the map on a particular region.

### Line Art

#### Grid count

Sets the number of grid divisions overlaid on the map, controlling the density of the grid.

#### Grid intenstity

Adjusts the brightness or opacity of the grid lines, making them more or less visible.

#### Grid style

Specifies where grid lines are drawn on the map.

- **None**: No grid lines are displayed.
- **OverWater**: Grid lines are drawn only over water areas.
- **OverLand**: Grid lines are drawn only over land areas.
- **Everywhere**: Grid lines are drawn over both land and water areas.

#### Boundary thickness

Sets the thickness of boundary lines, such as map borders or region outlines.

#### Contour thickness

Specifies the thickness of contour lines, affecting their visual prominence.

#### Grid thickness

Controls the thickness of grid lines overlaid on the map.
