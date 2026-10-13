# LoopEnd

LoopEnd ends a loop: the nodes from LoopBegin to LoopEnd run several times over, each pass starting from the last one's result.

The graph builds once as usual; then LoopEnd runs **Loops** − 1 more passes, each feeding its input back out of [LoopBegin](loop-begin.md). A loop only runs once it is primed: **Design Time** runs it in the editor, **Build Time** in command line builds.

## When to Use It
Use LoopEnd, with LoopBegin, when a chain of nodes should run several times over, each pass building on the last.

## Properties

### Primed

#### Design Time

#### Build Time

### Loop

#### Loops

Number of loops to perform

#### Mutate seeds

When enabled, the seeds of all nodes within the loop are mutated.

#### Seed

The seed for mutation if enabled above.
