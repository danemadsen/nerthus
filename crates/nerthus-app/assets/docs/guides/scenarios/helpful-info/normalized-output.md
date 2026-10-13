# Using Normalized Output

When you export a heightfield, you’re not really exporting "mountains"  -  you’re exporting numbers. And the *single biggest* quality win you can get (especially for 16-bit) is making sure those numbers use as much of the available range as possible before you save. That’s exactly what normalizing / autoleveling does: it remaps your current minimum height to 0 and maximum height to 1 (or to the full integer span), so you’re not wasting precision on empty headroom. 

## Why "use the whole range" matters

A stored heightfield has discrete steps:

* 16-bit integer heightmaps have 65,536 possible values (0–65535).
* 32-bit outputs are often 32-bit float (EXR) or sometimes 32-bit integer; either way, they can represent far more detail  -  *but* many pipelines still quantize, filter, or bake down later, so good range usage still helps.

The key idea: the vertical "step size" equals (height range) / (number of code values).

So if your terrain’s meaningful heights only occupy, say, 10% of the saved range (because you left lots of unused headroom above peaks or below sea level), you effectively throw away most of your bits. In 16-bit, using only 10% of the range gives you ~6,553 usable levels instead of 65,536  -  that’s roughly 12.7 bits of effective precision, not 16. Result: more visible terracing/banding, noisier erosion micro-detail, and worse downstream filtering.

Normalizing/autoleveling fixes that by stretching your actual min→max to the full span, minimizing quantization error everywhere.

## Why it’s especially important for 16-bit

16-bit is the common "game-friendly" choice, and it’s the most fragile:

* Banding/terracing shows up first in gentle slopes and flats (dunes, plains, snowfields).
* Erosion detail and small sediment variations can collapse into the same height code when the range is underused.
* Any later processing (blur, resample, mip generation, engine import) amplifies those errors.

In practice, if you’re exporting 16-bit, normalization is one of those "free quality" steps that costs nothing and pays back everywhere.

## "But I need real-world meters!"  -  normalize now, scale later

Normalization does *change absolute elevation* (it’s a remap), so the right workflow is:

1. Normalize/autolevel before saving to maximize precision.
2. In your final DCC / engine (Houdini, Unreal, Unity, etc.), apply a vertical scale (height multiplier / displacement scale) to restore the intended meter range.

Conceptually:

* Exported value:
  `v = (h − hmin) / (hmax − hmin)`
* Reconstructed height in DCC:
  `h = v · (hmax − hmin) + hmin`

That means you can keep your terrain *numerically optimal* on disk, while still ending up with the correct real-world heights in the final scene.

### Practical tip: save the metadata

If you want perfect reconstruction, keep track of (or export alongside) your terrain’s original min/max elevation (or your desired total relief in meters). Then your "Height Scale" in the DCC becomes deterministic instead of eyeballed.

## A note on tiles: avoid per-tile normalization seams

One caveat: if you normalize each tile independently, each tile gets its *own* min/max, which can cause seams where values don’t match across borders.

Best practice:

* Normalize before tiling (one global min/max), or
* Force a consistent min/max across all tiles.

## The bottom line

Normalizing (autolevel) before saving is great because it:

* Uses the entire bit depth you’re paying for (critical in 16-bit).
* Preserves subtle slopes and micro-relief by minimizing quantization error.
* Keeps exports robust through resampling, filtering, and engine import.
* Lets you do the "artistically meaningful" part  -  real-world vertical scaling  -  later, non-destructively, in the final DCC app.
