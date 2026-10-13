# Navigating Complex Graphs

Large terrain graphs are easier to work with when they have clear landmarks. As the graph grows, try to make the structure explain itself: primary shape creation, erosion, masks, color, simulation, baking, and export should each have an obvious place.

## Use Preview Landmarks

Lock Preview is useful when you want to edit upstream nodes while seeing the result at a later stage. This is especially helpful when small changes in a primitive or mask need to be judged against the final eroded or colorized output.

Underlays are useful when previewing masks or color nodes that need terrain context. If the automatic underlay is not the one you need, set the intended terrain as the underlay so masks and color maps remain readable.

For interface details, see [Lock Preview](../../ui/graph/basic-workflow/lock-preview.md) and [Underlays](../../ui/graph/basic-workflow/underlays.md).

## Follow the Graph Intentionally

When a selected node is part of a dense area, use graph navigation tools to move through incoming and outgoing dependencies instead of manually tracing every wire. Selecting upstream or downstream nodes can also help you understand what contributes to a result, or what will be affected by a change.

[Data View](data-view.md) is useful when you need a structured list of graph contents. The Terrain tab can help locate nodes by type or status, while the Build tab shows the nodes currently marked for export.

## Create Stable Handoff Points

Long connections are hard to read and easy to disturb. Use [Portals and Chokepoints](portals-and-chokepoints.md) when a result needs to travel across the graph or be reused in several places.

Use chokepoints before important branches. A chokepoint gives you one place to reroute when you replace or revise the upstream terrain, instead of reconnecting every downstream consumer.

When several simulations produce related masks, [Accumulators](accumulators.md) can collect them into a single output so the graph stays cleaner.

## Keep the Graph Scannable

* Place related nodes close together and leave space between major stages.
* Use names for important outputs, especially anything marked for export.
* Keep reusable masks near the systems they control, or expose them through portals.
* Check for orphan nodes when a graph has gone through several rounds of experimentation.
* Bake stable sections with [Baking Nodes](../baking-nodes/index.md) when navigation is being slowed down by repeated processing.
