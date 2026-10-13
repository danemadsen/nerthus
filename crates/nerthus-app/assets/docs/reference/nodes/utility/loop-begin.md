# LoopBegin

LoopBegin begins a loop: the nodes from LoopBegin to LoopEnd run several times over, each pass starting from the last one's result.

The graph builds once as usual; then LoopEnd runs the remaining passes, each feeding its input back out of LoopBegin. How many passes, and whether the loop runs at all, are set on the [LoopEnd](loop-end.md) node.

## When to Use It
Use LoopBegin, with LoopEnd, when a chain of nodes should run several times over, each pass building on the last.
