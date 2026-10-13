# Develop Regions Separately

A lot of people coming from older World Machine workflows are used to the idea of "zooming in" after they’ve established a terrain: build a big landscape, pick an interesting region, then crank the resolution and refine the closeup.

That can work with **pure fractal detail**, because fractals can add detail at any scale. But once you introduce **physically-inspired processes** (especially erosion) this "zoom later" approach starts to break down.

## Why zooming in breaks realism

Fractals don’t care about world scale. Erosion does.

When you zoom into a region of a finished terrain and rebuild it at higher resolution, you often get:

* **Erosion fidelity mismatch**: gullies, channels, and sediment patterns were computed at the original scale. Zooming changes your "meters per pixel", so the same erosion settings no longer represent the same physical size.
* **Feature scale drift**: what used to be a valley becomes a canyon, or what used to be believable rills become oversized grooves because the terrain’s context changed.
* **Loss of context**: erosion depends on upstream drainage, slope continuity, and catchment area. Cropping/zooming can change flow behavior and produce different results.
* **Detail that looks pasted-on**: you compensate with noise/fractals, but it can clash with the erosion language of the larger terrain.

> [!TIP]
> If your closeup needs to hold up in a hero shot, treat it like its own asset with its own scale, erosion, and build budget.

## The modern Nerthus approach

With Nerthus, it’s usually better to build the "zoomed-in" element **first**, at the resolution and fidelity you need, and then place it into a larger landscape.

Instead of "zooming into a world", you "compose a world around a hero asset".

The key advantage: **both the hero element and the surrounding landscape can live in the same graph**, but they can be built independently at high resolution as needed.

## Example workflow: a volcano and its surroundings

Imagine you need:

* A hero 8K asset of the volcano crater and upper slopes for close shots
* A broader 8K (or lower) landscape that includes the volcano in context for wide shots

### Build the hero volcano first

1. Create the volcano at the intended real-world scale (crater size, slope angles, caldera shape).
2. Run your erosion and detailing where it matters: rim breakup, ash flows, gullies, deposits.
3. Keep this branch clean and focused so it can be built at 8K efficiently.
4. Bake or export the volcano heightfield (and any masks you care about) so it becomes a stable "module".

### Compose it into a larger landscape

1. Build the surrounding terrain as its own branch (mountain ranges, plains, foothills).
2. Use **Transform** to position and scale the volcano module correctly.
3. Use **Embed**/**Combine**/**Transpose** as needed to seat the volcano into the landscape (clean integration beats brute-force blending).

Now you have one graph that contains both:

* A high-fidelity volcano asset branch
* A larger world branch that provides context

And you can build what you need, when you need it.

> [!TIP]
> Think "modules" instead of "zoom levels". Your volcano is a module with its own resolution target and erosion language, and the world is another module that supports it.

## Why this is better long-term than the old zoom workflow

* **Safer iteration**: you can change the surrounding world without risking the hero volcano (and vice versa).
* **Resolution where it matters**: build the volcano at 8K for hero shots, while keeping the broader landscape cheaper until you truly need more.
* **Consistent scale**: erosion, deposits, and micro-forms remain physically coherent because they were authored at the correct scale from the start.
* **Multiple deliverables**: the same graph can output a hero asset for closeups and a wide environment for shots - without forcing one to compromise for the other.

> [!TIP]
> If you try to "fix" a zoomed-in closeup by piling on fractal detail, it may look sharper, but it often won’t match the erosion scale language of the larger terrain. Viewers feel that mismatch immediately.

## The only real cost is planning

Yes, this approach asks you to plan a little earlier:

* Decide what is a hero element
* Decide what needs 8K fidelity
* Decide how it will integrate into the wider world

But once you adopt it, you’ll find it’s **easier, more predictable, and far less fragile** than the old "zoom in after the fact" workflow - especially when erosion and realism matter.
