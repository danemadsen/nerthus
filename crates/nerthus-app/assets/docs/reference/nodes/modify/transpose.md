# Transpose

Transpose takes the character of the Reference terrain and applies it to the Input terrain.

## When to Use It
Use Transpose when you want one terrain to take on the character of another, a reference.

## Transpose

Transpose maintains the original volume and shape of the Input terrain so you do not lose your silhouette, but still adds the details of the Reference terrain.

In practical scenarios, it is often difficult to create the kind of surface on a specific terrain. So you can create a basic terrain with low or no details, and then create flat (or undulating) surface separately, then Transpose the details onto your basic terrain.

This also works quite well with imported terrains or even meshes.

By combining Mask and Draw, you can paint your effect area.

## Insert

Insert lets you insert a sparse heightfield into another heightfield while preserving the target heightfield's relative heights. A common example is creating rocks on a flat (clipped) surface and then inserting them onto a terrain. Unlike Max or Add combines, Insert preserves the inserted heightfield and does not add artifacts or shapes from the underlying heightfield.

## Properties

#### Mode

How the Reference is applied to the Input.

- **Transpose**: Adds the Reference's detail to the Input, then fits the result back into the Input's height range, so the Input keeps its shape and silhouette.
- **Embed**: Sets the Reference's shapes into the Input, like stones pressed into the ground.
- **Insert**: Flattens the Input's finer detail under the Reference's footprint, then adds the Reference on top.

#### Amount

How strongly the Reference is applied.

#### Extend

Embed only. Values above 0.1 press the Reference deeper into the Input.

#### Flatten

Insert only. How much of the Input's finer detail is flattened under the Reference.

#### Threshold

Insert only. The height above which the Reference counts as part of its footprint.

#### Boundary

Insert only. How far the edge of the Reference's footprint is softened.
