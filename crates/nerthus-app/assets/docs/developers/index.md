# Developers

This section is for technical users building pipelines, automation, or reusable graph systems around Nerthus. It covers command-line builds, bridge workflows, macros, expressions, and variables.

If you are looking for day-to-day terrain creation guidance, start with [Using Nerthus](../using/index.md) instead. Use Developer docs when you need to make Nerthus repeatable, scriptable, or extensible.

---

## Start Here

> [!NOTE]
> Use Developer docs when:
>
> - you need command-line builds or automated output
> - you want to pass variables into a build pipeline
> - you are preparing custom bridge workflows
> - you are building macros or reusable graph tools
> - you are working with expressions, variables, or the Math node

## Main Areas

### Automation

Start with [Automation](automation/index.md) when you want Nerthus to run as part of a pipeline instead of only through the UI.

Useful starting points:

- [Command-line Interface](automation/cli/index.md) for the available `nerthus` and `nerthus-build` commands
- [Command Line Automation](automation/cli/command-line-automation.md) for automated builds with `nerthus-build`
- [Building Bridges](automation/building-bridges/index.md) for bridge-oriented automation patterns
- [Building a Bridge with the CLI](automation/building-bridges/building-a-bridge-with-the-cli.md) for CLI bridge setup notes

### Extensibility

Start with [Extensibility](extensibility/index.md) when you want reusable controls, graph packaging, or expressions.

Useful starting points:

- [Macros](extensibility/macros/index.md) for packaging graph logic into reusable nodes
- [Scripting and Expressions](extensibility/scripting-and-expressions/index.md) for variables, expressions, and Math node workflows

## Suggested Paths

If you need repeatable exports, start with [Command Line Automation](automation/cli/command-line-automation.md), then add variables once the basic build works.

If you need reusable artist-facing tools, start with [Macros](extensibility/macros/index.md).

## In this section

- [Extensibility](extensibility/index.md): Choose the right Nerthus extensibility path for reusable graph tools, parameterized projects, and scripting
- [Automation](automation/index.md): Automate Nerthus builds with `nerthus-build`, variables, generated command lines, and bridge-oriented file workflows
