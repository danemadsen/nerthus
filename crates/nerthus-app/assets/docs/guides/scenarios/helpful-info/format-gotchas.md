# File Format Gotchas

Here are practical do’s and don’ts for the formats people most often feed into Nerthus, tuned for the workflow:

See [Match or Correct Heights](match-height.md) for a useful workflow using Constant (meters) → File (unclamped) → Autolevel → MATCH (Constant as Reference)

# File Format Gotchas

## GeoTIFF DEM

### Do

* Prefer 32-bit float GeoTIFF when available (best fidelity, handles negatives cleanly).
* Keep File = Unclamped, especially if the DEM includes below sea level or "no-data" borders.
* Pre-handle NoData if your exporter encodes it as a huge negative (e.g., -32768) or huge positive - mask/crop it before Nerthus if possible, or you’ll Autolevel against garbage.
* Use your Constant in meters as the "intended peak altitude" and MATCH to it.

### Don’t

* Don’t export with "visualization scaling" (some GIS apps rescale for display); export raw elevation values.
* Don’t clamp on import; it can silently flatten valid extremes.

---

## 16-bit PNG

### Do

* Use 16-bit grayscale PNG (not 8-bit).
* Ensure the PNG is linear data:
  * If your tool has a color management option, pick linear / data / non-color.
* Treat 16-bit PNG as *precision-limited*: Autolevel is strongly recommended before any further math.
* Keep the File unclamped, then Autolevel, then MATCH to Constant.

### Don’t

* Don’t use 8-bit PNG for real terrain unless it’s purely stylized; banding will be obvious.
* Don’t let apps apply sRGB gamma to the heightmap (this bends elevations and ruins slopes).
* Don’t roundtrip through image editors that "helpfully" color manage or dither unless you’re sure it’s data-safe.

---

## 16-bit TIFF

### Do

* Prefer 16-bit integer TIFF if the source is already quantized (like a generator exporting 16-bit).
* Confirm it’s single-channel grayscale (or at least that the height is in one channel consistently).
* Use linear / non-color handling in any app that touches it.
* Autolevel is still valuable: if the terrain only uses part of the range, you’re wasting bits.

### Don’t

* Don’t assume "TIFF" means "high fidelity". Many TIFFs are 8-bit, RGB, or color-managed.
* Don’t apply compression workflows that alter values (some pipelines do processing on save).

---

## 32-bit EXR

### Do

* Prefer 32-bit float EXR for maximum fidelity and safe negatives.
* Use EXR when you’ll do lots of downstream processing (erosion, remaps, reprojection, tiling).
* Still consider Autolevel if your data occupies a small numeric range or you’re going to bake down later.

### Don’t

* Don’t treat EXR like an image: avoid color transforms, tone mapping, or display LUT baking.
* Don’t export "beauty" EXRs; export data EXRs (single channel if possible).

---

## RAW / R16 / .raw

### Do

* Use RAW when you control the pipeline end-to-end (e.g., Unreal landscape import).
* Match bit-depth and endianness exactly (common: 16-bit, little-endian).
* Keep a record of:
  * resolution
  * bit depth
  * endian
  * expected min/max elevation mapping
* Use Autolevel + MATCH in Nerthus so the RAW uses the full 0–65535 span before export.

### Don’t

* Don’t use RAW if you aren’t 100% sure of its format metadata - RAW has no header, so it’s easy to misread.
* Don’t change resolution midstream without a deliberate resample strategy.

---

## 8-bit formats (PNG/JPG)

### Do

* Use 8-bit for masks, splatmaps, selectors, and weight maps.
* Dither or blur thoughtfully if banding in masks becomes visible.

### Don’t

* Don’t use JPG for heights (lossy compression creates block artifacts and slope noise).
* Don’t use 8-bit PNG for final displacement unless it’s intentionally low-fi.

---

## Common gotchas (format-agnostic)

### Do

* Keep NoData out of your normalization range (crop/mask before Autolevel if needed).
* Work in linear data space end-to-end.
* For tiled terrains: normalize/match consistently (global intent), not per-tile randomness.

### Don’t

* Don’t per-tile Autolevel with different min/max unless you *want* seams.
* Don’t let any stage "optimize" or "enhance" your heightmap like it’s a photo.
