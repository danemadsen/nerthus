# Build the Right Version Before You Export

Many export problems are not format problems. The wrong profile was active, the wrong folder was inspected, a mask was not marked for export, or the final build was actually yesterday's file.

Before sending terrain data downstream, check the build itself.

> [!TIP]
> Open the Build tab and confirm the active profile, resolution, destination, and export-marked nodes before you run the final build.

## The pre-send check

Check the active profile in [Profiles](../../../ui/interface/build-options/profiles.md) so a final output is not built with preview settings. Check [Resolution](../../../ui/interface/build-options/resolution.md) so the intended detail level is being built. Check [Build](../../../ui/interface/build-options/build.md) so you know which folder will receive the files.

Then check [Nodes](../../../ui/interface/build-options/nodes.md) or Data View for the nodes and ports that will actually export. If automation is involved, compare the command in [Commands](../../../ui/interface/build-options/commands.md) with the build you just validated in the UI. After the build, use [Build Reports](../../../using/advanced-topics/command-line-builds/build-reports.md) and the files on disk to confirm what happened.

## Inspect the files, not the memory of the build

After building, look at the output folder by timestamp, count, and name. Make sure old files are not being mistaken for new ones. Make sure the `Latest` folder, if used, is the folder your downstream tool is reading.

Common finds include:

* final output built from `Preview_2K`
* old files left behind after a failed build
* a DCC linked to a numbered archive instead of `Latest`
* a renamed node changing a filename unexpectedly
* masks visible in the graph but not marked for export
