# Gate

Defines a baking boundary for the graph by forcing all ancestor nodes to be baked.

Gate passes its input on unchanged. It is marked as requiring baking, so Graph › Bake › Bake Required Nodes bakes it, and the nodes after it read the baked result instead of rebuilding everything before it.

## When to Use It
Use Gate to mark a point in the graph that should be baked, so that the nodes after it build quickly.
