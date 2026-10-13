# Construction

Construction places other terrains onto a base terrain, each moved, scaled, rotated, raised and blended into place through an edge mask. It is a way to assemble a landscape from separately made parts.

## When to Use It
Use Construction when you have built several landforms (a mountain, a volcano, a mesa) and want to arrange them on one base, each where you choose.

Connect the base terrain to the first input and each part to the inputs after it; **Add Input** adds more. Then click **Open Constructor**: a window showing the base and the parts, where each part's placement is set by dragging its gizmo or typing its values:

- position across and along the terrain, rotation and scale;
- height (a multiplier) and a base lift;
- a shaper that strengthens the part's shape;
- how it blends with the base (Max by default), and the shape and softness of the mask around its edges;
- whether it shows at all.

## Properties

#### Quality

The quality of the parts' resampling: Draft, Medium or High.
