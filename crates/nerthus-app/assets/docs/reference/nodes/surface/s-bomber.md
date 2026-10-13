# SBomber

SBomber scatters copies of other terrains or shapes over its main input, on a grid that tiles seamlessly. Each copy gets its own size, rotation and height within the ranges you set.

## When to Use It
Use SBomber when you need many scattered features, such as boulders, mounds or craters, over a terrain that has to tile. Connect the terrain to the first input and a shape to the second; once every input is connected, another appears, so you can scatter several shapes. Use [Bomber](bomber.md) when the result does not need to tile.

## Properties

#### Density

How many copies are scattered.

#### Size

The range of the copies' sizes.

#### Size distribution

How sizes spread across that range: at 0 evenly, higher values favor small copies.

#### Rotation

The range of the copies' rotations, as a share of a full turn.

#### Maintain proportions

Scales each copy's height with its size, so smaller copies are lower too.

#### Variable height

Gives each copy a height within the **Height** range.

#### Height

The range of the copies' heights, when **Variable height** is on.

#### Fade edges

Softens the copies' edges so they blend into the terrain.

#### Seed

Randomization seed for the copies' placement, sizes and rotations.

#### Blend mode

How overlapping copies combine: None, Blend, Add, Subtract, Difference, Multiply, Screen, Max or Min.

#### Quality

The quality of the copies' resampling: Draft, Medium or High.

#### Global scale

Scales the whole scatter grid.

#### Offset x

Moves the scatter grid across the terrain.

#### Offset y

Moves the scatter grid along the terrain.
