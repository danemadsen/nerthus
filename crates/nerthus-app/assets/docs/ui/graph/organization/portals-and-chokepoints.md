# Portals and Chokepoints

## Portals

Portals are a unique concept in graph-based applications. 

Like a "wormhole", Portals allow you to connect nodes across vast gaps (or short ones) without visible connections. They not only improve your quality of life, but also dramatically change how you think of and use graphs.

When using the [Organizing with Tabs](organizing-with-tabs.md), Portals can be used to connect nodes across different graphs.

### Creating Portals

Any output port in a node can be converted to a portal. You can create a portal in many ways:

Right-click a connection to bring up the Connection menu and select `Convert to Portal`.

Right-click an existing connection and select `Convert to Portal`.

Right-click a node and select `Manage Portals` (shortcut key `P`). Then select the port you wish to convert to a Portal. If the Port has existing connections, they will be converted to portal connections.

> [!NOTE]
> If you press `Shift` while Converting a Port to a Portal in the Manage Portals menu, a Chokepoint node will be created first and then its output will be turned into a Portal.

When you select "Connect to Portal" in the Portal menu for any incoming ports, you will get a Search popup with a list of available Portals.

## Chokepoints

The Chokepoint node is a special node whose sole purpose is to help avoid re-connecting many nodes, while adding no memory overhead.

Often a graph ends up with natural chokepoints - a single node that is referenced by several other nodes. If at a later point you want to modify that important node, you would need to reconnect all the connections. This can be awkward, time consuming, and dangerous in some situations.

To add one, right-click the important node, choose **Insert chokepoint**, and pick its output: a Chokepoint takes over all of that output's connections. You can also right-click a connection and choose **Insert chokepoint** to put one into that connection alone. See [Chokepoint](../../../reference/nodes/utility/chokepoint.md).
