# Command-Line Builds

`nerthus-build` builds the terrain recipe created in the Nerthus editor without the editor, so builds can run from a terminal, a script, or another application.

It is a separate command-line program, so it runs without a UI burdening it. For many builds in a row, `nerthus-build host` keeps a build host loaded between them, so each build starts faster.

## Options

All options for a build of a specific file are set in its [Build Settings](../../../ui/interface/build-options/index.md).

Some additional options can be set via [Command Line Automation](../../../developers/automation/cli/command-line-automation.md).

## Automation

See [Command Line Automation](../../../developers/automation/cli/command-line-automation.md) and [Variables](../../../developers/extensibility/scripting-and-expressions/variables.md) for automation scenarios.

## Post Action Report

Nerthus saves a post-action report as both human readable and machine readable files. You can use them in your automation workflow or use them to gather vital intelligence about your builds. See [Build Reports](build-reports.md).
