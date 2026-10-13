# Extensibility

Extensibility in Nerthus covers the tools that let you turn a graph or project into something reusable, controllable, or pipeline-friendly. Start here when you want to package graph logic, expose artist-facing controls, or drive builds through variables.

For automation-only workflows, such as command-line builds and bridge execution, start with [Automation](../automation/index.md) instead. Use this section when the structure of the graph itself needs to become more reusable or programmable.

## Choose a Path

| Goal | Start with |
| --- | --- |
| Package graph logic into reusable nodes | [Macros](macros/index.md) |
| Expose controls that can drive one or more node properties | [Variables](scripting-and-expressions/variables.md) |
| Transform or constrain incoming property values | [Expressions](scripting-and-expressions/expressions.md) |
| Build procedural shapes or calculations inside the graph | [Math Node](scripting-and-expressions/math-node.md) |
| Review available expression and Math node functions | [Allowed Functions](scripting-and-expressions/functions.md) |

## Macros

Use [Macros](macros/index.md) when you want to turn a graph into a reusable node. A macro can expose selected parameters, provide optional inputs and outputs, and let teams share terrain logic without copying the full source graph into every project.

Macros are the best first step for reusable artist-facing tools because they work inside Nerthus's existing graph workflow. Read [Building Macros](macros/building-macros.md) for the authoring process, then [Best Practices](macros/best-practices.md) before sharing macros with a team.

## Scripting and Expressions

Use [Overview](scripting-and-expressions/index.md) when you need a project to respond to exposed values or computed logic.

- [Variables](scripting-and-expressions/variables.md) create reusable values that can be bound to node properties and controlled through the UI or automation.
- [Expressions](scripting-and-expressions/expressions.md) modify a bound value at the property level, which is useful for scaling, clamping, or deriving one control from another.
- [Math Node](scripting-and-expressions/math-node.md) lets you write procedural calculations directly inside the graph.

These features are often the bridge between artist controls and automated builds. A project can expose variables for a pipeline, use expressions to keep controls stable, and use the Math node for custom procedural behavior.

## In this section

- [Macros](macros/index.md): Packaging graph logic into reusable nodes
- [Scripting and Expressions](scripting-and-expressions/index.md): Variables, expressions and the Math node
