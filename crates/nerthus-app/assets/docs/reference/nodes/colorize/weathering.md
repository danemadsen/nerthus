# Weathering

Weathering adds subtle tonal variation based on surface form: it brightens protrusions and exposed features, and darkens crevices and sheltered areas. It is typically layered on top of an existing color map as a delicate final touch, or used earlier to reveal erosion and weather shapes before adding finer material detail.

Think of it as a controlled "ageing pass" that helps terrain read better by reinforcing micro-contrast and form cues.

## When to Use It
Use Weathering when a color map needs subtle age, wear, and crevice emphasis without repainting the whole terrain.

## What It's Used For

* Adding a final "lived-in" finish to color textures without repainting.
* Bringing out erosion ridges, talus lines, and carved channels so the terrain reads at a distance.
* Introducing subtle dirt buildup in cracks, nooks, and recesses.
* Unifying material transitions by gently compressing overly clean or flat areas.

## How It Works

Weathering evaluates terrain shape at a chosen scale and produces a shading-like effect: highlights on exposed edges and raised areas, and darkening in recesses. The result is meant to be blended over your base color map, usually at low strength.

The effect can be tuned toward:

* Fine, gritty detail or broad feature definition, depending on scale.
* Softer spread and blending via creep, instead of hard, localized accents.
* More pronounced dirt accumulation in sheltered regions.

It also includes simple post adjustments (hue, saturation, lightness) to steer the final look toward a climate or biome.

## Usage Tips

* Use low amounts first. Weathering is most convincing when you can barely notice it until you toggle it off.
* Match scale to the detail you want to emphasize. Smaller scale will pick up fine erosion texture and small cracks; larger scale will emphasize ridges, terraces, and major creases.
* If the effect looks too procedural, increase creep and reduce amount. That tends to produce more natural blending.
* For finishing passes, apply Weathering after you have the base color map mostly resolved, then blend it lightly on top. For look-dev, apply it earlier to make forms readable, then layer additional detail textures afterward.
* If your base color is already high-contrast, Weathering can quickly become heavy-handed. Reduce amount or limit the node's contribution in the blend stage.

## Dirt and Crevice Definition

Weathering can push darker tones into recesses to simulate accumulated dirt, moisture staining, organic matter, or general grime. This is useful for:

* Making gullies and cracks feel deeper.
* Adding sheltered staining under overhang-like forms.
* Reducing the "freshly generated" look in clean terrains.

If dirt becomes too dominant, lower dirt strength before lowering overall amount. That keeps highlights present while dialing back the crevice darkening.

## Inversion and Washed-Out Looks

* Inverse flips the logic, emphasizing areas that would typically be protected rather than exposed. This can be useful for stylized looks, snow-like residue in recesses, or when your terrain's "exposed vs sheltered" assumption is reversed by your lighting or material intent.
* Washed Out fades or desaturates the result, mimicking prolonged exposure and bleaching. It can help unify colors when a texture set feels too saturated or too clean, but should be used carefully to avoid flattening the palette.

> [!TIP]
> In most workflows, keep these changes small and do any major color grading outside the weathering pass.

## Properties

### Primary

#### Scale

Sets the scale at which weathering effects are applied. Lower values target fine details, while higher values affect larger terrain features.

#### Creep

Simulates gradual movement and spreading of weathering effects across the terrain, softening transitions and blending weathered areas.

#### Amount

Adjusts the overall intensity of the weathering effect. Higher values produce more pronounced weathering.

#### Washed out

Toggles if colors are faded or desaturated, mimicking the effect of prolonged exposure to the elements.

#### Inverse

Inverts the weathering effect, applying it to areas that would normally be less affected, such as recesses instead of protrusions.

#### Dirt

Adds darker coloration to crevices, nooks, and sheltered areas of the terrain, simulating the accumulation of soil, organic matter, or moisture.

#### Darker

Intensifies the darkness of the dirt effect, making crevices and nooks appear deeper and more shadowed.

### Post Processing

#### Hue

Adjusts the overall hue of the weathered terrain, allowing for color shifts to match desired environmental conditions.

#### Saturation

Controls the intensity of colors in the weathered terrain, from muted to vibrant.

#### Lightness

Sets the overall brightness of the weathered terrain, affecting how light or dark the final result appears.
