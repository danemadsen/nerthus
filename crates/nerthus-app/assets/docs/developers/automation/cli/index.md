# Command-line Interface

Nerthus has two programs: `nerthus`, the editor, and `nerthus-build`, which builds projects without the editor. Both sit in the folder Nerthus is installed in.

## nerthus

```sh
nerthus [<project>] [--cpu-only] [--verbose] [--onboard] [--skip-onboard]
        [--diagnostics] [--mcp] [--mcp-port <port>]
```

| Option | Description |
| --- | --- |
| `<project>` | A `.terrain` file to open. |
| `--cpu-only` | Computes on the CPU only for this session, whatever devices the preferences name. See [CPU Only Mode](../../../using/troubleshooting/diagnostics/cpu-only-mode.md). |
| `--verbose` | Verbose Logging for this session: the log keeps the debugging messages too. |
| `--onboard` | Shows the first-run setup. |
| `--skip-onboard` | Marks the first-run setup as done without showing it. |
| `--diagnostics` | Opens [Diagnostics](../../../using/troubleshooting/diagnostics/index.md) at once. |
| `--mcp` | Starts the editor's MCP server for this session, so AI assistants can work on the open project. |
| `--mcp-port <port>` | Starts the MCP server on this port instead of 47110. |

The proxy for network connections is set in [Options › General](../../../ui/interface/options/general.md).

## nerthus-build

```sh
nerthus-build <command> [options]
```

| Command | Description |
| --- | --- |
| `build <project>` | Builds a project and exports its marked nodes: a Standard build, a Hybrid or Tiled one a bucket at a time from the project's bakes, or a Region build, as the project's build settings say. |
| `host [<project>]` | Builds in the build host, which stays loaded between builds. The first run starts it; later runs hand their project and options to it and print its build's output. `--exit` closes it. |
| `bake <project>` | Bakes nodes as Graph › Bake does. Without `--node`, bakes the nodes that need baking. |

`nerthus-build --help` lists every command, and `nerthus-build <command> --help` its options. See [Command Line Automation](command-line-automation.md) for the options of `build`.

Each run writes a log to the `Logs` folder of the [data folder](../../../using/troubleshooting/diagnostics/data-folder.md), and unless `--silent`, leaves a [build report](../../../using/advanced-topics/command-line-builds/build-reports.md) in the build folder.
