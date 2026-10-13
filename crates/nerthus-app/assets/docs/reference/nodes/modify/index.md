# Modify

Modify nodes reshape, filter, remap, warp, or clean up terrain and mask data. Use this family when you already have an input and need to adjust its form, range, sharpness, direction, or spatial behavior.

Start here when you need to:

- clean up unwanted artifacts or fragments
- clamp, normalize, sharpen, blur, or remap height data
- warp, transform, rotate, fold, or reshape an existing terrain
- add controlled post-process changes without replacing the source terrain

Useful starting points:

- [Adjust](adjust.md), [Autolevel](autolevel.md), [Clamp](clamp.md), [Curve](curve.md), and [Threshold](threshold.md) for value and range control
- [Blur](blur.md), [Denoise](denoise.md), [Median](median.md), [Sharpen](sharpen.md), and [SoftClip](soft-clip.md) for cleanup and filtering
- [Warp](warp.md), [SlopeWarp](slope-warp.md), [DirectionalWarp](directional-warp.md), [Transform](transform.md), and [Transform3D](transform-3d.md) for spatial changes
- [Distance](distance.md), [Fold](fold.md), [GraphicEQ](graphic-eq.md), [Shaper](shaper.md), and [ThermalShaper](thermal-shaper.md) for stronger shape design

## In this section

- [Adjust](adjust.md): The Adjust node is pretty much the same as the Modifier Stack, but in node form
- [Aperture](aperture.md): Aperture expands or compacts every feature on the terrain, similar to a camera's bokeh
- [Autolevel](autolevel.md): AutoLevel is a leveling node that allows you to level your terrain
- [BlobRemover](blob-remover.md): Remove small blobs or disconnected shapes from masks and heightfields in Nerthus
- [Blur](blur.md): The Blur node diffuses sharp shapes and softens the terrain
- [Clamp](clamp.md): Clamp controls the height of the terrain. You can squash the terrain, clip it at certain heights, or make it taller using the Extend operation
- [Clip](clip.md): Cuts the terrain against a threshold, removing values above or below a specified level
- [Curve](curve.md): Curve adjusts the height curve of a terrain
- [Deflate](deflate.md): Deflate takes away the bulk of the terrain leaving only finer details
- [Denoise](denoise.md): Denoise is a general purpose noise removal filter
- [Dilate](dilate.md): Dilate grows the bright pixels of a heightfield or mask outward
- [DirectionalWarp](directional-warp.md): Warp terrain using a guide input to create controlled directional deformation effects in Nerthus
- [Distance](distance.md): Generates a distance field from the input, measuring how far each point is from the nearest boundary or feature
- [Equalize](equalize.md): Redistributes the input values to balance the histogram and reveal more detail across the range
- [Extend](extend.md): Expands the terrain range beyond its current limits, effectively stretching elevation values
- [Filter](filter.md): Applies profile-based filtering to reshape terrain values and refine feature response
- [Flip](flip.md): Flip flips the terrain horizontally, vertically, or both
- [Fold](fold.md): Fold simulates terrain folding by introducing slants, breaks, and folds to the terrain. It significantly changes the terrain shape and should be used early in the graph
- [GraphicEQ](graphic-eq.md): Applies a multi-band equalizer-style filter to shape terrain features across different scales
- [Heal](heal.md): The Heal node can reconstruct damaged, low res, or 8-bit data with 16-bit, high resolution fidelity
- [Invert](invert.md): Turns its input upside down: high values become low and low values high
- [Match](match.md): The Match node is a low-level lets you provide a Reference input that is used to adapt/match the heights of the main Input
- [Median](median.md): Applies a median filter to remove spikes, speckles, and small noise while preserving edges better than blur
- [Meshify](meshify.md): Converts terrain into a faceted, mesh-like surface with controllable vertex density and detail sensitivity
- [Origami](origami.md): Origami creates a diagonal multifold pattern, and can produce a highly stylized terrain
- [Pixelate](pixelate.md): Converts the input into a blocky, pixelated version
- [Recurve](recurve.md): Recurve is a curvature-based expander node
- [Shaper](shaper.md): The Shaper node can bulk up, or bulk down, a terrain
- [Sharpen](sharpen.md): Sharpen enhances the edges and small structures of a terrain by making them more prominent
- [SlopeBlur](slope-blur.md): SlopeBlur is a versatile, low-level node that adds directional blurring based on the slopes of a Guide terrain or mask. It is one of our deceptively simple nodes that can create a wide array of shapes
- [SlopeWarp](slope-warp.md): SlopeWarp is a versatile, low-level node that warps a terrain along the slopes of a Guide terrain or mask. It behaves similarly to SlopeBlur
- [SoftClip](soft-clip.md): SoftClip a low-level utility that lets you softly reduce the intensity of shapes above a specified threshold. It can be comparable to masking a shape with a Height node and then clamping that portion
- [Swirl](swirl.md): Swirl creates a vortex warp on the terrain. Very useful for adding a fluid look
- [ThermalShaper](thermal-shaper.md): ThermalShaper is a fast and effective way of condensing and coalescing shapes similar to what Shaper does, but in a way where it is in line with what
- [Threshold](threshold.md): Abs can modify the terrain or mask it by eliminating values higher or lower than the exact value specified
- [Transform](transform.md): Transform allows you to move, rotate, and scale any terrain
- [Transform3D](transform-3d.md): Applies full 3D transformation to the terrain, including rotation and spatial adjustment
- [Transpose](transpose.md): Transpose takes the character of the Reference terrain and applies it to the Input terrain
- [TriplanarDisplacement](triplanar-displacement.md): TriplanarDisplacement applies displacement using triplanar projection across a surface
- [VariableBlur](variable-blur.md): Blurs the input by varying the blur amount across the surface using local data or a guide
- [Warp](warp.md): Warp can take the terrain and "warp" the shape creating more organic shapes
- [Whorl](whorl.md): Whorl applies multiple vortex-like distortions across the terrain to create large scale warping
