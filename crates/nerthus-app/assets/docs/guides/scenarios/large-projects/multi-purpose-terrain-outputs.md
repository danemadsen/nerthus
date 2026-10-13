# Build One Terrain for Game, Cinematic, and Closeup Outputs

The mistake is trying to make one export do every job.

A gameplay landscape, trailer vista, hero closeup, and material package can all come from the same terrain idea. But they usually should not come from the same build settings. Gameplay needs scale and import discipline. A cinematic wide shot needs silhouette and read. A closeup needs local detail. A material package needs clean masks, not a pretty viewport.

The graph can stay shared. The outputs should be allowed to differ.

> [!TIP]
> List the deliverables before you choose resolution. "One terrain" does not have to mean "one final file."

## Think in output lanes

Most large terrains have a shared foundation: macro shape, main erosion language, major masks, and art direction. After that, the work branches into lanes.

Useful lanes might be:

* **Gameplay**: height and masks for an engine landscape.
* **Cinematic wide**: a build tuned for camera distance, silhouette, and atmosphere.
* **Hero closeup**: a Region or separate detail pass where local density matters.
* **Material package**: named masks and utility outputs for shaders or biomes.
* **Archive**: source snapshot, notes, and a reproducible build folder.

These lanes can share the same terrain identity without sharing every setting. That is the point.

## A simple production shape

Start with the source terrain: the broad landform, major erosion, and the masks that define the terrain's intent. Keep this part stable and readable.

Then create output lanes from that source:

1. Save a **Build Profile** for the gameplay or engine build.
2. Save another profile for the final cinematic or presentation build.
3. Use **Regions** when one area needs closeup detail instead of raising the entire world to that density.
4. Use **Tiled Builds** when the world is too large for a single output.
5. Mark only the nodes that belong to each output lane.
6. Name files so the lane is visible in the result: `Gameplay_Height`, `Trailer_Wide_Color`, `Hero_Closeup_Normal`, `Mask_Slope_Rock`.

The important habit is to stop treating the graph as if it has one universal "final." It may have several finals, each honest about what it is for.

## Example: one mountain, three deliverables

Imagine a mountain terrain for a game reveal.

For gameplay, you build a clean engine heightfield and masks that match material layer names. You review it from traversal height and check the engine import.

For the trailer wide shot, you build a cinematic version with the same mountain identity but a different output profile. The review is about silhouette, valleys, snow line, and how the terrain reads from the shot camera.

For a closeup near the summit, you use a Region or a separate detail file. You spend resolution on rock breakup, snow pockets, and surface masks only where the camera will see them.

All three outputs belong to the same terrain. None of them needs to compromise for the other two.

## Common wrong turns

Building the whole world at closeup density is the expensive one. It slows the project and still may not give the closeup the art direction it needs.

Changing the shared terrain source for a one-off trailer shot is another common trap. If the shot needs special treatment, make a lane for it. Do not quietly bend the source graph until gameplay, masks, and archive builds inherit a cinematic-only decision.

And do not judge every output from the same view. Gameplay terrain should be checked like gameplay terrain. A trailer build should be checked from the shot. A mask package should be checked as data.

## Keep the source shared, not overloaded

The goal is a terrain with one clear creative foundation and several deliberate outputs. A gameplay build imports cleanly, a cinematic build serves the camera, a closeup build spends detail locally, and a material package contains masks someone can actually use outside Nerthus.
