# ErosionPro

The **ErosionPro** node is a powerful erosion algorithm designed to enhance terrain shaping capabilities beyond the traditional [Erosion](erosion.md).

Despite its complexity, ErosionPro remains user-friendly and delivers deterministic results with up to 10x faster performance, even on the CPU.

It simulates advanced hydraulic erosion with realistic sediment transport and terrain reshaping. It offers fine control over downcutting, sediment deposition, and orographic (rainfall-driven) effects for highly detailed or stylized terrains.

## When to Use It
Use ErosionPro when you want a more advanced hydraulic erosion pass with strong control over downcutting, sediment behavior, and rainfall-driven shaping.

## Properties

### General

#### Duration

Controls how long erosion runs. Longer durations produce more elaborate terrain reshaping, while shorter durations help preserve original forms.

#### Downcutting

Governs how deeply water cuts into the terrain, carving valleys and channels.

#### Erosion scale

Sets the overall size of erosion features, from fine gullies to massive ravines.

#### Seed

Randomizes the erosion pattern while keeping other settings the same.

#### No erosion

### Sediment Discharge

#### Suspended Load

Fine, light sediments that travel far and fill gullies with subtle detail.

#### Discharge Angle

Sets the slope at which suspended load settles.

#### Bed Load

Heavier sediments like gravel that fill and shape gullies and slopes.

#### Discharge Angle

Sets the slope at which suspended load settles.

#### Coarse Sediments

The heaviest materials (boulders, scree) that deposit on steep slopes.

#### Discharge Angle

Sets the slope at which suspended load settles.

#### Deposition Boost

### Shape

#### Shape

Enables artistic control over how erosion reshapes terrain.

#### Shape sharpness

Defines how sharp or concave ridges become after erosion.

#### Shape detail scale

Sets the scale of simulated details, from broad ravines to fine gullies.

### Orographic Influence

#### Enable

Toggles orographic influence on or off.

#### Directional precipitation

Simulates rain falling more intensely from one direction to create rain shadows.

#### Direction

Sets the direction from which rainfall comes most strongly.

#### Rain shadow

Controls how strongly terrain blocks rainfall from certain directions.

#### Slope

Limits rainfall effects to specific elevation ranges.

#### Altitude

Limits rainfall effects to specific slope ranges.

#### Reverse

Inverts the influence mask for creative control.
