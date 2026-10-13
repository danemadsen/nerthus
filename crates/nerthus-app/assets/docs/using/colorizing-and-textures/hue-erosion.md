# HueErosion

## Sedimentary Flows with HueErosion

HueErosion takes the principles of sedimentary erosion and applies it to color maps.

Nerthus's Erosion algorithm creates powerful, realistic sedimentary flows and the HueErosion nodes takes advantage of that to create realistic color flows that mimic minerals and other sediments as they shift and flow across a landscape.

HueErosion is extremely powerful, yet very simple to use. In most cases, the default settings are more than enough to get the precise effect. It requires only two inputs, the color map you wish to erode and your terrain heightfield.

### Using Directly

You can apply a SatelliteMap directly a terrain, with or without a TextureBase or Texturizer node in between, and simply apply HueErosion to get decent layered sedimentary flows.

### Using with Mixer

The Mixer node is very helpful in preparing the color map before applying HueErosion as you can change different color strata before eroding, and choose exactly how and where they may flow.

### Understanding How Color Moves

HueErosion can be a very creative component whether you are creating something realistic or stylistic. The core understanding required is how color placed high up flows with existing sediment flows.

Experiment with various color layers, whether you use SatelliteMap's Bias slider or various layers with Mixer, and see how the erosion reacts.
