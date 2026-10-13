# Managing Graphs

As a terrain becomes more detailed, the graph becomes part of the design problem. Good graph management makes it easier to understand what each branch is doing, reuse important outputs, find expensive nodes, and make changes without breaking downstream work.

This section focuses on practical habits and tools for keeping production graphs readable.

## Key Workflows

[Using Modifiers](using-modifiers.md) shows how to apply common adjustments directly on a node without adding a chain of extra nodes. Modifiers are useful for height remaps, masks, warps, and quick local corrections.

[Portals and Chokepoints](portals-and-chokepoints.md) explains how portals and chokepoints reduce long-distance connections and create safer handoff points between graph stages.

[Accumulators](accumulators.md) can collect masks from multiple simulation nodes, such as snow, water, vegetation, or debris, without building a dense web of Combine nodes.

[Data View](data-view.md) gives you a structured way to inspect the graph, exports, variables, statistics, heavy nodes, orphan nodes, and other project-level information.

[Navigating Complex Graphs](navigating-complex-graphs.md) covers navigation habits for larger projects, including how to move through dependencies, use landmarks, and keep related work grouped.

[Visualizing Scale](visualizing-scale.md) helps you judge whether the terrain reads correctly at the intended real-world scale.

## Practical Habits

* Name important nodes and keep major graph stages visually separated.
* Use portals for long connections and repeated references, especially across distant graph regions.
* Use chokepoints before branches that feed many downstream systems.
* Prefer modifiers for small local adjustments that do not need to become standalone graph logic.
* Use the Data View to find exports, heavy nodes, orphan nodes, and other maintenance targets.
* Bake stable sections when the graph becomes expensive to preview or build.
