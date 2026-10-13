# Utility

Utility nodes help organize, route, compare, mask, and control data in a graph. Use this family when the problem is not the terrain shape itself, but how data moves, branches, repeats, or stays manageable.

Start here when you need to:

- route or switch between graph branches
- compare inputs or build helper masks
- manage layers, loops, variables, or reusable graph logic
- keep a complex graph readable and easier to control

Useful starting points:

- [Compare](compare.md), [Mask](mask.md), [Edge](edge.md), and [Accumulator](accumulator.md) for mask and data helpers
- [Layers](layers.md), [Mixer](mixer.md), and [Combine](combine.md) for organizing multiple inputs
- [Gate](gate.md), [Switch](switch.md), [Route](route.md), [LoopBegin](loop-begin.md), and [LoopEnd](loop-end.md) for flow control
- [Repeat](repeat.md), [Seamless](seamless.md), and [Reseed](reseed.md) for pattern and variation workflows
- [Macro](macro.md) and [MacroPort](macro-port.md) for reusable graph logic and Macro authoring
- [Math](math.md), [Var](var.md), and [DataExtractor](data-extractor.md) for technical graph logic

## In this section

- [Accumulator](accumulator.md): One of the common ways a graph gets clutters with overlapping connections is when multiple Snow, Lake, or Debris nodes are used and their respective masks
- [Chokepoint](chokepoint.md): The Chokepoint node is a special node whose sole purpose is to help avoid re-connecting many nodes, while adding no memory overhead
- [Combine](combine.md): The Combine node is one of the most important nodes in Nerthus
- [Compare](compare.md): Compare two heightfields to inspect differences and validate terrain data in Nerthus
- [Construction](construction.md): Places other terrains onto a base terrain, each moved, scaled, rotated and blended into place
- [DataExtractor](data-extractor.md): Extracts specific derived data such as boundaries, differences, or Modifier maps from an input
- [Edge](edge.md): Edge softly fades the edges of a terrain to nothing, similar to a vignette in paint applications. Useful with Bomber or other effects where a tall edge would be a problem
- [Gate](gate.md): Defines a baking boundary for the graph by forcing all ancestor nodes to be baked
- [Layers](layers.md): Combines multiple layers using blend operations in a stacked workflow
- [LoopBegin](loop-begin.md): LoopBegin begins a loop: the nodes from LoopBegin to LoopEnd run several times over, each pass starting from the last one's result
- [LoopEnd](loop-end.md): LoopEnd ends a loop: the nodes from LoopBegin to LoopEnd run several times over, each pass starting from the last one's result
- [Macro](macro.md): Creates an instance of an installed Macro so a packaged graph can be reused as a node
- [MacroDefinition](macro-definition.md): Holds a macro's name, author and description
- [MacroPort](macro-port.md): Defines an input or output port inside a macro for passing height, mask, or color data
- [Mask](mask.md): The Mask node allows you to mask the node/effect after it has been created
- [Math](math.md): Evaluates mathematical expressions to generate or modify terrain data procedurally
- [Mixer](mixer.md): Mixer allows layer-based manipulation of color textures and contains in-built masking
- [Repeat](repeat.md): This simple filter repeats or tiles a given terrain
- [Reseed](reseed.md): Re-randomizes an upstream procedural node by overriding or refreshing its seed behavior
- [Route](route.md): Route chooses the flow of the graph based on the Input/Output ports chosen, and relies on integer input for automation
- [Seamless](seamless.md): Seamless can transform any input terrain or color map into a tileable/repeating pattern
- [Switch](switch.md): Switch chooses between two different heightfield inputs and relies on a boolean (true/false) input
- [Var](var.md): Reads or modifies named graph variables for procedural control and automation
