# Automation

Automation in Nerthus starts with a `.terrain` file that already builds correctly in the UI, then makes that build repeatable from another process. The supported workflow in this section is command-line and file based: expose the values that should change, generate or write the command that should run, and let `nerthus-build` execute the build.

Use [Command-Line Builds](../../using/advanced-topics/command-line-builds/index.md) to understand the builder itself. Use [Command-line Interface](cli/index.md) for the available `nerthus` and `nerthus-build` commands, then use [Command Line Automation](cli/command-line-automation.md) for the build options used in production automation.

## What You Can Automate

- Build a terrain through `nerthus-build` without opening the full UI.
- Select a Build Profile or Region for a specific output target.
- Override exposed variables from the command line or from a variable file.
- Set a mutation seed for repeatable variations.
- Copy a known-good command line from the [Commands](../../ui/interface/build-options/commands.md) tab.
- Run a post-build command from the [Script](../../ui/interface/build-options/script.md) tab.
- Build bridge-style workflows that pass files into Nerthus and collect exported files afterward.

## Recommended Workflow

1. Build the terrain once in the UI so the graph, exports, resolution, and output folders are known to work.
2. Expose the values that the pipeline needs to control as [Variables](../extensibility/scripting-and-expressions/variables.md). Use file-friendly variable names when those values will be set from another tool.
3. Configure [Build Settings](../../ui/interface/build-options/index.md), including Profiles or Regions if the same terrain needs multiple repeatable build targets.
4. Use **Show Build Command Line** in the Data View's Variables tab to capture the exact command for the current file, with its variables.
5. Replace the copied variable values with pipeline values, or move them into a `.json` or `.txt` variable file for easier automation.
6. Run the build through `nerthus-build`, then consume the exported files or post-action report in the next tool.

## Automation Building Blocks

| Need | Use |
| --- | --- |
| Launch options, diagnostics, or CPU-only startup | [Command-line Interface](cli/index.md) |
| Headless terrain builds | [Command Line Automation](cli/command-line-automation.md) |
| External control over node properties | [Variables](../extensibility/scripting-and-expressions/variables.md) |
| Derived or constrained parameter values | [Expressions](../extensibility/scripting-and-expressions/expressions.md) |
| Standardized preview, final, or engine-specific output sets | [Profiles](../../ui/interface/build-options/profiles.md) |
| Focused builds for part of a terrain | [Regions](../../ui/interface/build-options/regions.md) |
| Post-build file moves or downstream commands | [Script](../../ui/interface/build-options/script.md) |
| Bridge workflows between Nerthus and another application | [Building Bridges](building-bridges/index.md) |

## Keep Automation Stable

- Start from a command copied from the UI before hand-writing automation commands.
- Quote file paths, especially paths coming from another application or build system.
- Prefer absolute paths for bridge workflows unless the caller controls the working directory.
- Use variables for values that must change between runs; do not edit the `.terrain` file for every build.
- Use `--vars` with a variable file when the command line would become long or hard to inspect.
- Decide intentionally whether cached data should be reused or whether the build should ignore cache.
- Keep variable preset files with the same project or pipeline configuration that launches the build.

## In this section

- [Command Line Interface](cli/index.md): The editor's and the command-line builder's options
- [Building Bridges](building-bridges/index.md): Build bridge workflows around nerthus-build by exposing inputs, parameters, and output paths as variables
