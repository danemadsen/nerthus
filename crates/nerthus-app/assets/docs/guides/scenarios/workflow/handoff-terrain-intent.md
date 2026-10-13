# Hand Off a Terrain to Another Artist Without Losing Intent

The next artist does not only need the `.terrain` file. They need to know what the terrain is supposed to be, which parts are approved, which outputs matter, and which branches are experiments.

A graph can open correctly and still be a bad handoff. If every branch looks equally important, the receiver has to guess where to continue.

> [!TIP]
> Mark the approved path through the graph before you send the file. The handoff should be visible before anyone reads the notes.

## Make the graph carry the intent

Use Nerthus's graph tools to show decisions, not just node structure.

Group the main stages: base shape, erosion, surface detail, masks, color, and outputs. Add short annotations where a branch has a specific purpose. Bake or Gate stable checkpoints so it is clear which parts are foundation and which parts are still open for editing.

Name production masks and output nodes in plain language. If `Mask_Slope_Rock` leaves the graph, the next artist should not have to inspect ten nodes to learn what it controls.

Data View is useful before handoff. Check export-marked nodes, orphan nodes, errors, and required baking so the receiver does not inherit hidden cleanup work.

## Say what is safe to change

A useful handoff describes the terrain contract:

* **Goal**: dry badlands hero terrain for Unreal, alpine vista for a shot, playable canyon landscape, etc.
* **Scale**: world size, height range, intended build resolution.
* **Approved stages**: base shape, erosion pass, masks, output branch.
* **Safe edits**: color pass, local detail, material masks, closeup branch.
* **Do not touch**: baked foundation, shared output names, contract masks, source File nodes.

The important part is not the length of the note. It is whether the next artist can make a change without accidentally breaking the part that was already approved.

## A short handoff note

```text
Terrain purpose:
Target app or shot:
World size / height range:
Approved path:
Safe to edit:
Do not change:
Production outputs:
Known issues:
Next step:
```

Keep this note near the terrain file or inside the package. It should match what the graph shows.

## Example: handing off mask work

If the next artist is taking over material masks, the graph should make the terrain structure easy to ignore. The base shape and erosion can be grouped and baked. The mask branches should be named by purpose: slope rock, deposit soil, flow wetness, snow height, biome regions.

The note can say: "Base shape and erosion are approved. Continue from the mask group. Do not rename existing mask outputs because the Unreal material expects those names."

That is enough context to start work without a meeting.

## A good handoff is obvious

The receiving artist should be able to open the file, identify the approved chain, understand the terrain goal, find the production outputs, and know where to continue.

If they have to ask whether a branch is old, final, experimental, or safe to edit, the handoff is not finished yet.
