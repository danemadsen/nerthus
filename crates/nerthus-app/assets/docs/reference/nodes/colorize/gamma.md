# Gamma

The Gamma node applies gamma adjustment to heightfield or mask values. It behaves like gamma in an image editor: it remaps midtones more than extreme blacks or whites, changing perceived contrast without being a simple linear gain.

Gamma is most useful when you want to bias a terrain or mask toward lighter or darker mid-values without crushing lows or highs as aggressively as a Levels-style operation.

## When to Use It
Use Gamma when you want to brighten or darken terrain or mask mid-values without remapping the full range linearly.

## Practical Usage Tips

* Think "mid-value bias" rather than "contrast". For min or max shaping, normalize first with Levels or Remap, then use Gamma to bend the curve.
* Make small moves. Values like 0.85, 0.95, 1.05, and 1.15 are often enough.
* Use on masks deliberately. Gamma is a fast way to tune erosion, flow, snow, or vegetation masks without reauthoring them.
* Preview masks solo. A subtle change on terrain preview can be dramatic when used as a selector downstream.

## Gamma and Color Textures

Gamma mistakes are a common cause of "washed out", "too dark", or "mushy mask" results. The key is distinguishing color textures from data textures.

### Color vs Data

Color (usually sRGB or gamma-encoded):

* Albedo/BaseColor/Diffuse

Data (must remain linear):

* Roughness/Metalness/AO
* Normal maps
* Height/Displacement
* Masks/weightmaps/splatmaps

Rule of thumb: if it represents *visible color*, it's usually sRGB. If it represents *numbers*, it should be linear.

### Import Tips

* Import color textures as sRGB. If treated as linear, they will usually appear too dark and respond oddly to edits.
* Import masks or data textures as linear. If sRGB or gamma decoding is applied, your masks will get "soft" and thresholds will not match what you expect.
* If a mask behaves unexpectedly, verify it has not been gamma-treated by the file format or exporter. This is common with image tools that assume sRGB.

### Export Tips

* Export color textures as sRGB unless your target explicitly expects linear color.
* Export data textures as linear, and in the target tool or engine ensure they are imported as linear, often with "sRGB off".
* Avoid "fixing" pipeline mismatches by baking gamma into maps unless you control the entire pipeline. It is easy to double-apply later.

### Common Symptoms

* Washed out color: gamma applied twice (double sRGB).
* Too dark color: missing sRGB decode (treated as linear).
* Weak or muddy masks: mask imported or exported as sRGB instead of linear.

## Properties

#### Gamma

The Gamma range is 0.0 to 2.5, with 1.0 being neutral or the original incoming gamma. &lt; 1.0 lifts mid-values (brighter), expands lighter detail, broadens features. &gt; 1.0 darkens mid-values, compresses lighter detail, tightens features.

#### Automatic

Tries to automatically set the correct gamma based on existing value.
