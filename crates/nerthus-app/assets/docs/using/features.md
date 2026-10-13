# Feature Overview

## Terrain creation and realism

* Node-based Infinity Graph for building complete terrain recipes, from first shape to final outputs.
* Extensive node library covering generators, shaping, masking, filtering, simulations, surfacing, utilities, and exports.
* Real-world landform simulations, built around erosion workflows like [Erosion](understanding-erosion/index.md).
* Surface and material workflows for believable terrain finish, including [Crafting the Surface](crafting-the-surface/index.md).
* Color workflows from procedural to photographic, including [Satellite Maps](colorizing-and-textures/working-with-satellite-maps.md).

## Build, export, and production pipeline

* End-to-end [Build and Export](build-and-export/index.md) workflow for repeatable output generation.
* Detailed build diagnostics and outputs via [Build Reports](advanced-topics/command-line-builds/build-reports.md).
* Large-world production via [Tiled Builds](build-and-export/tiled-builds.md).
* Focused high-resolution production with [Regions](build-and-export/regions/index.md).
* Mesh pipelines for engines and DCCs via [Mesh Exports](build-and-export/mesh-exports.md).
* Element-focused exports for downstream compositing and look-dev via [Exporting Elements](getting-started/basics/exporting-elements.md).

## Speed, iteration, and convenience

* Non-destructive finishing controls with [Modifiers](managing-graphs/using-modifiers.md).
* Rapid variation systems through [Mutations](advanced-topics/command-line-builds/creating-mutations.md).
* Predictive workflow assistance via [Learning and Predictive System](advanced-topics/technical-information/learning-and-predictive-system.md).
* Tools and habits for scaling big projects via [Managing Graphs](managing-graphs/index.md).
* Cleaner complex graph wiring patterns using [Portals](managing-graphs/portals-and-chokepoints.md).

## Automation and pipeline integration

* Headless, high-resolution batch building with [command-line builds](advanced-topics/command-line-builds/index.md).
* Saved, repeatable production presets through [Profiles and Batch Builds](advanced-topics/command-line-builds/profiles-and-batch-builds.md).
* Full CLI automation for farms and pipelines via [Command Line Automation](../developers/automation/cli/command-line-automation.md).
* Command-line options for CPU-only mode, verbose logging and diagnostics via [Command-line Interface](../developers/automation/cli/index.md).
* Automation-friendly filename and path control using [Managing Input and Output](../developers/automation/building-bridges/managing-input-and-output.md).

## Parametric control and power-user extensibility

* Centralized controls that can drive many properties via [Variables](../developers/extensibility/scripting-and-expressions/variables.md).
* Per-control value shaping, remapping, and logic using [Expressions](../developers/extensibility/scripting-and-expressions/expressions.md).
* Scriptable procedural construction inside graphs with [Math Node](../developers/extensibility/scripting-and-expressions/math-node.md).
* A structured entry point for advanced control systems via [Scripting and Expressions](../developers/extensibility/scripting-and-expressions/index.md).

## Reliability, troubleshooting, and production readiness

* Bake and cache strategies for large builds using [Baking Nodes](baking-nodes/index.md).
* Project-wide inspection and cleanup tools via [Data View](managing-graphs/data-view.md).
* Built-in diagnostics and benchmarking via [Diagnostics](troubleshooting/diagnostics/index.md).
* Optional CPU-only operation for compatibility and debugging via [CPU Only Mode](troubleshooting/diagnostics/cpu-only-mode.md).
* Common pitfalls and quick fixes collected in [Common Errors](troubleshooting/terrains/common-errors.md).

## Import, formats, and interoperability

* Broad interchange support for practical pipelines, documented under [File Formats](advanced-topics/technical-information/file-formats.md).
* Real-world terrain ingest workflows starting with [Importing Terrains](importing-terrains/index.md).
* Mesh ingestion workflows for hybrid pipelines via [Importing Meshes](getting-started/basics/importing-meshes.md).

## Reuse and packaging

* Reusable mini-graphs as single nodes with [Macros](../developers/extensibility/macros/index.md).
* Macro creation workflow and packaging through [Building Macros](../developers/extensibility/macros/building-macros.md).
* Guidance for building maintainable, shareable macro libraries via [Macro Best Practices](../developers/extensibility/macros/best-practices.md).
