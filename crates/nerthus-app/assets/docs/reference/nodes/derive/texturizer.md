# Texturizer

Texturizer works like **TextureBase** but uses pre-defined, curated profiles to quickly generate natural-looking texture masks. It's designed for fast, consistent results with less manual tuning. **Factor** is the main strength or intensity control for the chosen style, while **Secondary** adjusts the impact of supporting details and effects.

## When to Use It
Use Texturizer when you want ready-made terrain texture masks with controllable breakup instead of building every selector from scratch.

## Properties

### Texturizer

#### Style

Select from A, B, C, D, E, F, G, H, I, J, K, or L. Each option represents a distinct pre-defined texture style, allowing you to experiment and choose the look that best fits your terrain.

#### Factor

Primary driver controlling the overall strength or intensity of the selected style. Higher values increase the prominence of the texture effect.

#### Secondary

Adjusts the strength of secondary effects such as soil accumulation, flow streaks, and breakup patterns. Use to fine-tune the complexity and realism of the texture.

#### Seed

Randomizes the pattern while keeping other settings the same, allowing for variation without the need to re-tune parameters.

### Components

#### Flows

Simulates natural erosion channels and directional streaks, adding realism by mimicking the effects of water or wind movement across the terrain.

#### Slope

Adds slope-based weighting to the texture, favoring either steeper or flatter areas to enhance natural variation.

#### Soil

Controls soil-like accumulation in flatter regions, adding natural buildup and enhancing the realism of the terrain surface.
