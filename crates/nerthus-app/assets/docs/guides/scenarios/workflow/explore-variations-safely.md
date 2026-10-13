# Explore Variations Without Breaking Your Main Graph

Variation work should create choices, not confusion.

If three new versions leave you unsure which file is approved, which graph changed, or which output belongs to which idea, the exploration is already costing too much. The first job is to protect the baseline. The second job is to make the variants comparable.

> [!TIP]
> Before testing a new direction, make the current approved state obvious: bake it, duplicate it, save it, or mark it in the graph.

## Choose the right kind of variation

Not every idea needs a duplicated project file.

Use **Mutations** when you want natural seed differences while keeping the same terrain logic. Use **Build Profiles** when the terrain is the same but the build target changes. Use **Variables** when a small set of exposed values needs to be tested quickly.

When the art direction changes in a bigger way, duplicate the file or split the project into a separate detail file. A new erosion language, different biome logic, or a completely different closeup treatment should not be hidden inside the main graph as "just a quick test."

For protected foundations, bake a checkpoint or place a Gate before you start experimenting downstream.

## A safe variation pass

Start by deciding what is approved now. It might be the base shape, the erosion pass, the whole graph, or only the latest output package.

Then:

1. Save or bake that baseline.
2. Name the variation by what it is trying to prove: `Var_SharperCliffs`, `Var_DryBadlands`, `Var_SofterErosion`.
3. Change one class of thing at a time.
4. Build variants with the same camera, resolution, lighting, and output list.
5. Compare them beside the baseline.
6. Promote the winner or archive the branch.

> [!WARNING]
> If you cannot restore the baseline in one clear step, stop and make the baseline explicit before exploring.

## Compare like with like

Changing every review condition at once produces new terrain files, not meaningful alternatives.

Keep the same terrain definition, review camera, lighting or presentation mode, build resolution, output names, and mask checks. If the only thing that changed is the erosion pass, you can judge the erosion pass. If the camera, resolution, color, and masks all changed too, the comparison is muddy.

## Example: trying a drier erosion style

Suppose the current terrain is approved for its silhouette, but the erosion feels too soft.

Do not edit the main erosion chain directly. Bake or Gate the approved shape, duplicate the erosion branch, and label the new branch `Var_DryErosion`. Keep the output profile and camera the same as the baseline. Build both and review them side by side.

If the dry version wins, promote that branch into the main path. If it loses, delete or archive the branch without untangling the production graph.

## Exploration should end cleanly

At the end, you should be able to show the baseline and variants side by side, explain what changed, choose a winner, and remove the rejected work without damaging the production graph.

That is the difference between exploration and drift.
