# Command Line Automation

Nerthus's command-line builder, `nerthus-build`, builds a project and exports its marked nodes without opening the editor, so builds can be automated.

See [Command-line Interface](index.md) for both programs and all their commands.

## Arguments

`nerthus-build build <project> [options]` accepts the following options:

| Option | Description |
| --- | --- |
| `<project>` | The path to the `.terrain` file to build. Wrap it in `""` quotes for safety. |
| `--resolution <size>` | The build resolution. Defaults to the project's; a Region build builds at the region's size. |
| `-r`, `--region <name>` | Builds this region of the terrain instead of the one the build settings name. |
| `--output <folder>` | The build folder, instead of the project's destination template. |
| `-p`, `--profile <name>` | Builds with this [build profile](../../../ui/interface/build-options/profiles.md) instead of the saved build settings. |
| `-v`, `--variable <name=value>` | Sets a variable for this build. Repeat it for each variable, for example `-v foo=bar -v value=0.35 -v erosion=1`. |
| `--va <values>` | Every variable's value, comma separated, in the order the project lists its variables. It must include all of them. |
| `--vars <file>` | A `.json` object or a `.txt` file of `name=value` lines setting variables. See below. |
| `--seed <number>` | Builds a [mutation](../../../using/advanced-topics/command-line-builds/creating-mutations.md) of the terrain: every node's seed moved on by this number. |
| `--ignorecache` | Builds the baked nodes instead of reading their baked maps (Standard builds). |
| `--silent` | Prints only what goes wrong, leaves no report in the build folder and never opens it. |
| `--interactive` | Asks which build profile to use when the project has more than one and none is given, and whether to open the build folder once built. Needs a terminal. |
| `--macros <folder>` | The folder holding the project's macros. |
| `--cpu-only` | Computes on the CPU only, whatever devices the preferences name. |
| `--verbose` | Verbose logging: the run's log keeps the debugging messages too. |
| `--safemode` | Builds with the default preferences instead of the saved ones, computing on the default device. For diagnosing problems; not recommended for production builds. |
| `--pause` | Waits for Enter before exiting, unless the build folder opens (for a build in a window of its own). |

The Data View's **Variables** tab has a **Show Build Command Line** button that shows the current file's build command line, including all variables. It is useful to copy this and then modify it as needed. Build Settings' **Copy Command Line** copies the command without the variables.

For many builds in a row, `nerthus-build host` takes the same options but keeps a build host loaded between builds, so each build starts faster.

### Using Files for Variables

You can store variables in a text or JSON file and pass it with `--vars`. This is useful for more complex automation where an external tool saves variables instead of passing them directly on a command line.

#### Text file

The text file holds `name=value` lines.

```ini
variable=value
filename=C:\Path\File.ext
erosion=0.35
switchErosionProcess=1
```

#### JSON File

```json
{
  "variable": "value",
  "filename": "C:\\Path\\File.ext",
  "erosion": 0.35,
  "switchErosionProcess": 1
}
```

> [!CAUTION]
> Variable names cannot be repeated.

To see how variables can be bound to node properties, see [Variables](../../extensibility/scripting-and-expressions/variables.md).

## Examples

```sh
nerthus-build build "D:\Terrains\Canyon.terrain"
nerthus-build build "D:\Terrains\Canyon.terrain" --profile "Final" --resolution 4096
nerthus-build build "D:\Terrains\Canyon.terrain" -v Height=0.63 -v Seed=12
nerthus-build build "D:\Terrains\Canyon.terrain" --vars "D:\Terrains\canyon-vars.json" --output "D:\Out\Canyon"
```
