# Recurve

Recurve is a curvature-based expander node. It can inflate and/or deflate the terrain to create stronger slopes and larger, softer formations from a given noise or terrain.

> [!NOTE]
> Despite appearances, Recurve's output is very sensitive to subtle changes in the source heightfield. For example, if prior to Recurve you apply MicroErosion or Breaker, then Recurve will create a very specific flow pattern on those grooves and valleys.

## When to Use It
Use Recurve when you need curvature-based expansion or contraction that reshapes existing forms without replacing them.

## Properties

#### Power

The strength of the inflate/deflate operation. Higher values cause drastic shifts in shape formation.

#### Scale

The amount of details to preserve. Higher values cause more prominent shapes to form.

#### Iterations

#### Style
