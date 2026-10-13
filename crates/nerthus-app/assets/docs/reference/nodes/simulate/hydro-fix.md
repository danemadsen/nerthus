# HydroFix

HydroFix is a low level utility that can help create unbroken flows. It does not change the landscape completely, but rather creates small adjustments along flow zones to promote long and better looking flows.

For example, the Flow map of a simple Perlin noise breaks into short, scattered pieces. With HydroFix applied to the Perlin, the same Flow map shows longer and more coherent structures.

> [!NOTE]
> You do not need to use HydroFix with Rivers as it has a special version of it built into its algorithm.

## When to Use It
Use HydroFix when a flow map breaks up into short pieces: it adjusts the terrain slightly along flow zones so that flows run longer and unbroken.

## Properties

### HydroFix

#### Downcutting

The amount of downcutting to apply while modifying the terrain for hydrological flow.
