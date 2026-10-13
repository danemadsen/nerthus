# Chokepoint

The Chokepoint node is a special node whose sole purpose is to help avoid re-connecting many nodes, while adding no memory overhead.

Often a graph ends up with natural chokepoints - a single node that is referenced by several other nodes. If at a later point you want to modify that important node, you would need to reconnect all the connections. This can be awkward, time consuming, and dangerous in some situations.

For example, a second Erosion node that feeds several other nodes is a chokepoint: a critical junction in the terrain's creation process, more or less its "last" node.

The easy solution is to insert a Chokepoint node after the important node: right-click the Erosion node, choose **Insert chokepoint**, and pick its output. The Chokepoint takes over all of that output's connections. See [Portals and Chokepoints](../../../ui/graph/organization/portals-and-chokepoints.md) for more.

Now everything is routed through the Chokepoint, allowing you to experiment with the Erosion node later, and only reroute a single connection when needed.

The Chokepoint can be further organized by making its output a portal. You can select all the outgoing connections and press `Ctrl + P` to turn them into portal connections. This is especially useful when working with multiple graphs.

## When to Use It
Use Chokepoint when several downstream nodes depend on the same result and you want a safer place to reroute that shared connection later.
