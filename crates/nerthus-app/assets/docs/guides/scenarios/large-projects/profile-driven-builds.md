# Use Profiles for Repeatable Builds and Automation

If you rebuild final settings by hand each time, sooner or later one build will be wrong. The resolution changes, the output folder is different, the wrong nodes are marked, or the command line no longer matches the build that was approved.

**Build Profiles** are the simple fix. Instead of remembering a pile of settings, you save the build as a named target: preview, final, engine import, mask package, tiled world, shot closeup, and so on.

> [!TIP]
> Make a profile as soon as an output becomes something you expect to build more than once.

## Start with the builds you actually need

Most projects do not need a huge profile library. They need a few reliable outputs that everyone understands.

Common profiles include:

* `Preview_2K` for quick reviews
* `Final_8K` for the approved terrain output
* `Unreal_Landscape` for an engine-specific build
* `Masks_Package` for material or biome data
* `Tiles_Final` for the full tiled terrain
* `Shot010_Closeup` for a Region or hero-camera build

The name should tell you what kind of build will happen before you open the settings. Avoid names like `test`, `final2`, `new`, or `profile_a`; those names are cheap when you create them and expensive when you need to trust them later.

## A practical profile workflow

Build the terrain once by hand and inspect the result. Confirm the resolution, destination, output nodes, file formats, and any Region or tiled settings. When that build is correct, save it as a profile in [Profiles](../../../ui/interface/build-options/profiles.md).

From there, use the profile as the source of truth:

1. Run the profile from the UI for normal artist builds.
2. Use [Profiles and Batch Builds](../../../using/advanced-topics/command-line-builds/profiles-and-batch-builds.md) when several outputs need to run together.
3. Use [Commands](../../../ui/interface/build-options/commands.md) to copy a known-good command line for automation.
4. Use variables only for values that genuinely change per run, such as a destination path or exposed parameter.

Profiles should describe the *kind* of build. Variables should describe the few things that change around that build.

> [!WARNING]
> Do not turn every build setting into a variable just because automation is possible. If the output is a known target, put the stable settings in a profile and keep the command line small.

## Where profiles help most

### Preview and final builds

A preview profile can stay light: lower resolution, fewer exports, and a fast destination. A final profile can use the real resolution, final output nodes, and the folder structure expected by the rest of the project.

That separation prevents the common mistake where a quick preview build quietly becomes the file someone sends downstream.

### Engine and material packages

Engine imports often need specific naming, formats, or output nodes. Mask packages usually need different files from the beauty or height build. Keeping those as separate profiles makes the build intent clear.

For example:

* `Unreal_Landscape` builds the engine height and layer data.
* `Masks_Package` builds only the named production masks.
* `Final_8K` builds the main terrain deliverable.

They may all come from the same graph, but they are not the same build.

### Command-line and batch builds

Once a profile is correct, copy the command line from Nerthus and use that command as the automation baseline. This matters because the command came from a build you already validated in the UI, not from a hand-written guess.

## A good profile is boring

You know the setup is working when an artist or build machine can run `Final_8K`, `Unreal_Landscape`, or `Masks_Package` and get the expected output without recreating settings by hand.

At that point, the profile name is part of the project contract. Treat it like any other production output: change it deliberately, and make sure the next person knows what it builds.
