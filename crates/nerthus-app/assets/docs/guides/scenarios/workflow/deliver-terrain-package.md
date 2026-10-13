# Deliver a Terrain Package That Others Can Use

Sending a heightmap is not the same as delivering a terrain.

The person receiving it still needs to know the scale, height range, mask meanings, final filenames, intended look, and whether the source can be rebuilt. Without that, the terrain may import, but the next step becomes guesswork.

> [!TIP]
> Before copying files, write the package note. It forces you to name what the delivery actually contains.

## What a useful package contains

A minimal terrain package usually needs:

* the final height output
* production masks for materials, biomes, shaders, or layers
* a preview image that shows the intended result
* scale and height notes
* the source `.terrain` file or a build snapshot when reproducibility matters
* a short output list explaining which file does what

If any of those are missing, someone downstream has to infer the answer. That is how good terrain data gets imported at the wrong size, masks get inverted, or old test files become "final."

## Keep the package clean

Use a folder that separates source, notes, and final files. For example:

```text
Terrain_Package/
  notes/
    scale-and-outputs.md
    preview.jpg
  source/
    Terrain_Final.terrain
  final/
    height/
    masks/
    color/
    mesh/
```

The exact folder names can change, but the idea should stay simple: `final/` is for files the receiver should use. Experiments, rejected builds, and old tests do not belong there.

## Set it up in Nerthus

Name the output nodes before the final build. A file called `Mask_Slope_Rock` is already more useful than `Export_12`.

In Build Settings, choose a destination that matches the package structure. If a downstream DCC or engine needs stable links, use a static latest folder as described in [Avoid Roundtrips](../helpful-info/roundtrip.md). When the build needs to be audited later, copy the `.terrain` file to the build folder and keep the [Build Reports](../../../using/advanced-topics/command-line-builds/build-reports.md) output with the package.

For masks, write down what white means. A mask named `Mask_Deposit_Soil` is much safer when the note says "white = sediment and soft soil accumulation."

## A package note that is enough

```text
Terrain:
Target app or shot:
World size:
Height range:
Build resolution:

Final height:
Masks:
- Mask_Slope_Rock: white = exposed steep rock
- Mask_Deposit_Soil: white = sediment / soft soil

Preview:
Source file:
Known issues:
Rebuild notes:
```

This does not need to become a large document. It only needs to answer the questions that would otherwise interrupt the next person.

## Stop before sending if

Do one final pass before handoff. Stop if the final file is named `final2`, masks do not explain black and white meaning, the package has no preview, source inputs are missing, scale has to be guessed, or old and final outputs are mixed together.

The package is ready when the receiver can import the terrain at the right scale, use the masks correctly, compare against the preview, and know whether the source can be rebuilt.
