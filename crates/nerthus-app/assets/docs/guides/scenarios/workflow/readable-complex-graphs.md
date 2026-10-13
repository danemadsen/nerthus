# Keep Complex Graphs Readable as They Grow

A graph can still build while becoming impossible to understand.

The risk is not visual mess by itself. The real risk is that nobody can find the approved shape, the active masks, the output branch, or the experiment that should have been deleted.

> [!TIP]
> Name the main stages and outputs while the graph is still small. Cleanup is much cheaper before handoff pressure arrives.

## Make the graph tell the terrain story

Most production graphs have a natural order:

1. inputs
2. macro shape
3. erosion and simulation
4. surface/detail
5. masks and data maps
6. color or presentation
7. exports

Group or separate the graph around those stages. The layout should help someone understand the terrain without previewing every node.

Use Groups when a stage needs a visible boundary. Use annotations when a branch has an unusual purpose. Use Tabs when the graph has genuinely outgrown one surface. Use Portals and Chokepoints when wires travel too far or many branches depend on one result. Use [Data View](../../../using/managing-graphs/data-view.md) and [Graph Hygiene - Orphan Nodes](hygiene-orphans.md) before handoff to catch abandoned work.

## A two-minute cleanup

Before a final build or handoff, do a small pass:

* rename important outputs with `F2`
* group one messy stage
* add one annotation for any non-obvious branch
* move experiments away from the main chain
* check Data View for orphans and export-marked nodes

This does not need to become a redesign. A small cleanup often removes the worst ambiguity.
