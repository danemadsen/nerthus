# Crumble

Crumble runs a different kind of simulation that feeds off of existing crevices, erosive information, and other subtle details to collapse the terrain. Just as the name implies, the terrain starts crumbling from the edges.

Crumble works with just about any Primitive, Erosion, and Surface node. While the controls may seem somewhat simplistic, they can create a wide range of effects depending on the combination.

## When to Use It
Use Crumble when a terrain should break down from its edges, its crevices and erosion details collapsing.

## Controlling the Edge

Horizontal bias shifts the erosion either towards the beginning of flow structures or towards the deposits at the end. Vertical bias shifts the erosion towards the higher altitude or lower altitudes.

While both may look simple, combining them in unique ways can give you a variety of different-looking terrains.

While Crumble does "eat away" the edges, the control over the edges was limited. With the new Edge slider, you can reduce or expand the effect the erosion process has on sharp edges. You can combine this with the Rock Hardness slider for absolute control.

## Properties

#### Duration

Controls how long the simulation runs. Longer durations produce stronger and more extensive crumbling effects.

#### Strength

Defines how intensely the rock or terrain breaks apart from the main body. Higher values result in more aggressive fragmentation.

#### Coverage

Sets how far the crumbling effect spreads inward from exposed edges, determining the affected area.

#### Horizontal

Biases the crumble effect towards flow origins or towards deposits, influencing the lateral direction of collapse.

#### Vertical

Biases the crumble effect towards higher or lower altitudes, controlling whether upper or lower terrain is more affected.

#### Rock hardness

Controls material resistance, reducing how much terrain collapses. Higher values simulate harder, more resilient rock.

#### Edge

Expands or reduces the impact of crumbling specifically on sharp terrain edges, allowing for more pronounced or subdued edge effects.

#### Downcutting

Controls how deeply the simulation carves downward into the terrain during crumbling, affecting the vertical extent of collapse.

#### Depth

Adjusts the overall depth of the collapsed areas, determining how far the terrain sinks or breaks away.

#### Direction

Specifies the primary direction in which the crumbling effect propagates across the terrain.

- **X**: Directs the crumbling effect along the X (horizontal) axis.
- **N**: Directs the crumbling effect towards the north (upward in world space).
- **E**: Directs the crumbling effect towards the east (right in world space).
- **S**: Directs the crumbling effect towards the south (downward in world space).
- **W**: Directs the crumbling effect towards the west (left in world space).
