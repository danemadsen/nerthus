# LightX

Creates a fully lit render using controllable sun, shadow, and ambient lighting.

## When to Use It
Use LightX when you need a fully lit render of the terrain, with control over the sun, shadows, and ambient light.

## Properties

### LightX

#### Direction selection

Specifies how the sun direction is determined for lighting calculations.

- **Manual**: Allows manual input of sun direction parameters such as azimuth and elevation.
- **Geographic**: Calculates sun direction based on geographic coordinates, date, and time for physically accurate lighting.

#### North

Sets the orientation of the north direction in the scene, affecting geographic sun calculations.

#### Latitude

Specifies the latitude for geographic sun calculations, influencing sun position and lighting angle.

#### Day of year

Sets the day of the year for geographic sun calculations, affecting seasonal sun position and lighting.

#### Solar hour

Specifies the time of day for geographic sun calculations, determining the sun's position in the sky.

#### Sun azimuth

Sets the horizontal angle of the sun, determining the direction from which sunlight is cast.

#### Sun elevation

Controls the vertical angle of the sun above the horizon, affecting the length and intensity of shadows.

#### Angular diameter

Specifies the apparent size of the sun in the sky, influencing the softness of shadows and highlights.

#### Direct light

Adjusts the intensity of direct sunlight in the scene, affecting brightness and shadow contrast.

#### Shadows

Enables or adjusts the rendering of shadows cast by terrain features, enhancing depth and realism.

#### Ambient light

Controls the overall ambient light level, simulating indirect illumination from the sky and environment.

#### Ambient occlusion

Enables or adjusts ambient occlusion, which darkens crevices and areas shielded from ambient light for added realism.

#### Quality

Sets the rendering quality for lighting and shadow calculations.

- **Low**: Fastest rendering with basic lighting and shadow detail.
- **Medium**: Balanced quality and performance with moderate lighting and shadow detail.
- **High**: High-quality rendering with detailed lighting and shadows.
- **Ultra**: Maximum quality with the most accurate lighting and shadow effects.

### Light

#### Exposure

Adjusts the exposure level, controlling the overall brightness and dynamic range of the rendered image.

#### Sun intensity

Sets the strength of the sun's light, affecting the brightness of direct illumination.

#### Ambient intensity

Controls the strength of ambient light, influencing the brightness of areas not directly lit by the sun.

#### Air density

Simulates atmospheric density, affecting the scattering of light and the appearance of haze.

#### Haze

Adds atmospheric haze to the scene, softening distant terrain and simulating light diffusion.

#### Ozone

Adjusts the ozone level in the atmosphere, influencing the color and intensity of sunlight.

### Colors

#### Automatic colors

Enables automatic adjustment of sun and ambient colors based on sun position and atmospheric conditions.

#### Sun color

Sets the color of direct sunlight, allowing for warm, cool, or custom lighting effects.

#### Ambient color

Specifies the color of ambient light, affecting the overall tone of shaded areas.

#### Ground brightness

Controls the brightness of the ground plane, influencing the overall illumination and visual balance of the scene.
