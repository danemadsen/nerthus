# Building Bridges

A Nerthus bridge connects a `.terrain` file to another application, engine, DCC, or pipeline tool. In the current developer workflow, the reliable integration boundary is a prepared terrain file, variables, file paths, and command-line execution through `nerthus-build`.

This means a bridge usually does not need to change the graph at runtime. The bridge prepares input files and variable values, runs Nerthus, then imports or registers the exported files in the target application.

Use this section when you need to design such a workflow around file-based automation.

## Bridge Shape

A practical bridge has five parts:

1. A `.terrain` file that can build without manual UI decisions.
2. Variables for any input filenames, output paths, or exposed node parameters the host application needs to control.
3. Export nodes with predictable formats, names, and output locations.
4. A command-line build step that launches `nerthus-build` with the correct file, profile, region, and variable values.
5. Host-side code that reads the exported files and maps them into the target application.

## Prepare the Terrain File

The terrain file is the contract between Nerthus and the bridge. Keep that contract explicit:

- Bind the `Filename` property of a File node when the bridge needs to provide an input heightfield or mask.
- Bind exposed terrain controls to [Variables](../../extensibility/scripting-and-expressions/variables.md) instead of editing node properties per run.
- Bind Export node paths when the bridge needs to choose the output folder or filename.
- Use **Explicit** output location when the host application provides the full output path. The Export node expects the path without the file extension; the selected export format decides the extension.
- Use Build Profiles when the same graph needs preview, final, engine-specific, or batch output setups.

See [Managing Input and Output](managing-input-and-output.md) for the input and output binding details.

## Choose the Output Contract

Before writing bridge code, decide what the target application will receive. A bridge may consume a heightfield, masks, color maps, tiled outputs, metadata, or a mix of those files. The important part is that filenames, formats, and scale expectations are predictable.

For engine workflows, [Unreal](../../../guides/use-in/engines/unreal.md) and [Unity](../../../guides/use-in/engines/unity.md) describe the formats, resolutions and scale those engines expect.

## Bridge Patterns

| Pattern | Use it when |
| --- | --- |
| File-in, file-out processor | Another tool sends Nerthus an input terrain or mask, waits for the build, then reads the exported output. |
| Export preparation bridge | Nerthus prepares files and metadata in a format that a target application can import, such as the Unreal workflow. |
| Batch or farm bridge | A pipeline tool runs many builds from the same `.terrain` file by changing variables, seeds, profiles, or regions. |
| Artist-facing wrapper | A DCC or engine plugin presents a small UI, writes variable values, and runs `nerthus-build` behind the scenes. |

## Implementation Notes

- Start by copying the command line (see [Commands](../../../ui/interface/build-options/commands.md)) for a working terrain file.
- Prefer a variable file with `--vars` when the bridge needs to pass several values.
- Keep temporary input and output folders unique per run so concurrent builds do not overwrite each other.
- Capture logs and preserve Nerthus's [build reports](../../../using/advanced-topics/command-line-builds/build-reports.md) when diagnosing failed or unexpected builds.
- Treat the generated files as the handoff point. The target application should validate that the expected files exist before importing them.

## In this section

- [Managing Input and Output](managing-input-and-output.md): It is crucial to be able to modify the Input and Output filenames for automating a workflow via command line automation
- [Building a Bridge with the CLI](building-a-bridge-with-the-cli.md): Use nerthus-build, variables, and predictable file paths to build a simple command-line bridge
