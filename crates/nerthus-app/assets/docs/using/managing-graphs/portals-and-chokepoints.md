# Portals and Chokepoints

Portals and chokepoints help large graphs stay readable. They do not change the terrain result; they change how information moves through the graph and how safely you can revise it later.

For basic interface steps, see [Portals and Chokepoints](../../ui/graph/organization/portals-and-chokepoints.md).

## Portals

A portal lets an output be consumed elsewhere in the graph without drawing a long visible connection. This is useful when a terrain, mask, or color output needs to be reused far away from its source.

Use portals when:

* a mask is shared by several branches;
* a terrain stage feeds work in another part of the graph;
* a graph has become hard to read because of long crossing wires;
* related work has been separated into different graph areas.

Portals are especially useful with stable intermediate results, such as the terrain after primary erosion, a master slope mask, a reusable flow map, or a final heightfield used by multiple export nodes.

## Chokepoints

A chokepoint is a neutral pass-through node used as a deliberate junction. It is valuable when many downstream nodes depend on one result and you may need to replace or revise that upstream result later.

Instead of connecting many branches directly to a node that may change, route them through a chokepoint. If the upstream terrain changes, you can reconnect the chokepoint once and keep the downstream structure intact.

## Practical Pattern

1. Build a major stage, such as primary terrain shaping or erosion.
2. Add a chokepoint after the stage if several branches will depend on it.
3. Convert the chokepoint output to a portal when the result needs to be reused far away.
4. Name the chokepoint or portal clearly so it remains searchable and understandable later.

This pattern is useful for handoff points such as `Base Terrain`, `Post Erosion`, `Main Flow`, `Rock Mask`, `Water Mask`, and `Export Height`.

## Common Mistakes

* Do not use portals to hide structure that is still changing rapidly. During early exploration, visible wires can make dependencies easier to understand.
* Do not create many similar portal names. Clear names matter more once the graph has multiple reusable outputs.
* Do not skip chokepoints before heavily shared outputs. They make later rerouting much safer.
