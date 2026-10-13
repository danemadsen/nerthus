# Peaks

The Peaks node creates a selection mask for peaks and ridgelines using protrusion, also described as inverse curvature. Unlike height-based selectors, Peaks can isolate prominent high points even when they occur at different absolute elevations. This makes it useful for targeting summit features, crest lines, and exposed tips across an uneven terrain.

The output is a grayscale mask where white indicates strong peak or ridge features and black indicates flatter or recessed areas.

## When to Use It
Use Peaks when you need a selection mask for peaks and ridgelines using protrusion, also described as inverse curvature.

## Common uses

* Snow and ice highlights on exposed summits and ridges, independent of overall elevation.
* Weathering accents and lichen staining on protruding rock.
* Placing sharp rock detail, scree sources, or peak-specific textures.
* Driving erosion variations by emphasizing exposed tips and crests.
* Adding a high-frequency highlight mask to improve terrain readability in renders.

## Usage tips

* Use Peaks when Height is insufficient. If your terrain has multiple mountain groups at different elevations, Peaks can still pick out their tips consistently.
* Balance Peaks with Slope. Peaks finds protrusions, not necessarily steep faces. Combining Peaks + Slope can isolate sharp crest rock while ignoring broad domes.
* Falloff controls focus. Lower values spread selection down ridges and into nearby forms; higher values concentrate the mask tightly around the highest protrusions.
* Precise produces a tighter, more faithful ridge and tip detection. It is helpful when you need the mask to track narrow crest lines, but it can also make the result more sensitive to small surface noise.
* If the mask becomes noisy, reduce upstream micro detail or smooth lightly before Peaks, then reintroduce micro detail later in the texturing stage.

## Properties

#### Falloff

Controls how quickly the mask transitions from the peaks and ridges to lower altitudes. Higher values create a sharper, more focused mask around the highest points.

#### Precise

Enables a more accurate detection of peaks and ridges, resulting in a mask that closely follows the true tips of the terrain.
