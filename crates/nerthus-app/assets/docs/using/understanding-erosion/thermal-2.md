# Thermal2

**Erosion**

* **Duration:** Controls the duration of the thermal erosion process. A higher value means the erosion will act over a longer period, leading to more significant changes in the terrain.
* **Strength:** Determines the intensity of the thermal erosion. Higher values will result in more pronounced smoothing and displacement of materials, particularly on steeper slopes.
* **Anisotropy:** Governs how erosion and the resulting rock deposits are shaped. This parameter influences the directional bias of the erosion, affecting both the smoothing of the terrain and the formation of talus slopes.
  * **Low values** keep the original terrain largely intact, resulting in minimal smoothing and erosion.
  * **High values** intensify the erosion, especially on sharp peaks, leading to the creation of stronger talus deposits and more heavily eroded terrain. This is useful for simulating environments where the erosion is more aggressive in specific directions, such as under strong wind or gravity influences.

**Talus**

* **Angle:** Sets the maximum angle of the slopes before materials start to move downward, forming a talus. Lower angles result in more gradual slopes, while higher angles allow for steeper terrains before material begins to shift.
* **Sediment Removal:** Controls the amount of sediment that is removed from the terrain during the talus formation. Higher values will result in cleaner, sharper features as excess material is removed, whereas lower values allow more sediment to accumulate, creating smoother transitions.

**Scale**

* **Feature Scale:** Adjusts the scale of the thermal erosion features in meters. Larger values will produce broader, more pronounced features, while smaller values will result in finer details.

These parameters in the Thermal2 node give you control over how thermal erosion shapes your terrain, from broad, sweeping changes to fine, detailed adjustments, with the Anisotropy parameter providing nuanced control over the directionality and intensity of the erosion effects.
