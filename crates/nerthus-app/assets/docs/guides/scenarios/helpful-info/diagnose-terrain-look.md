# Diagnose Why a Terrain Does Not Match the Look You Wanted

When a terrain looks wrong, adding another node is often the slowest fix.

First find where the problem lives: scale, macro shape, erosion, surface detail, mask logic, color, output settings, or camera distance. A specific diagnosis is easier to fix than a vague feeling that the terrain needs "more detail."

> [!TIP]
> Bypass late detail and judge the macro shape before adding more surface work.

## Work from early to late

Start with Terrain Definition and practical scale. If the width, height range, or output target is wrong, later nodes will fight the setup.

Then inspect the base shape without surface decoration. If the silhouette is weak, micro detail will not save it. Check erosion as a support stage, not a magic fix. After that, inspect surface detail, masks, and color.

Finally, check the exported file in the target context. A terrain can look right in Nerthus and still be wrong in engine if scale, normalization, or file interpretation changes.

## Common diagnoses

If the terrain feels too flat, check height range and silhouette. If it feels too noisy, bypass surface detail and inspect masks. If it feels synthetic, look for repeated patterns or weak erosion support. If the final image is muddy, view masks as data before judging color. If it is wrong in engine, check output settings and target-app scale.

> [!WARNING]
> Do not use micro detail to solve a macro shape problem. It usually makes the terrain slower and less readable.

## Useful Nerthus moves

Bypass suspect nodes with `B`. Lock preview to a downstream result while editing upstream. View masks directly before judging color. Use Data View to find the active output branch. Build a small test output before rebuilding the final package.
