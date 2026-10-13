# Curvature

The **Curvature** node creates a mask where convex areas, such as sharp edges, ridges, and protrusions, are selected. It's a very useful layer in texture creation, often for adding highlights or edge wear on top of an existing texture map.

## When to Use It
Use Curvature when you need a mask for exposed edges, ridges, and convex forms to drive highlights, rock breakup, or wear.

## Properties

#### Range

Sets the minimum and maximum curvature values to include in the mask. Only terrain features within this curvature range are selected.

#### Type

Specifies the type of curvature to evaluate for the mask, determining which terrain features are highlighted.

- **Horizontal**: Selects areas with horizontal (planform) curvature, highlighting ridges and valleys based on their plan shape.
- **Vertical**: Selects areas with vertical (profile) curvature, emphasizing convex and concave slopes along the direction of steepest descent.
- **Average**: Uses the average of horizontal and vertical curvature, providing a balanced mask that highlights both plan and profile features.

#### Falloff

Controls the softness of the mask transition at the range boundaries. Higher values create a smoother, more gradual blend from selected to unselected areas.
