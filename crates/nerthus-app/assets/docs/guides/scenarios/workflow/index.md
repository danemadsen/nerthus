# Workflow

- [Bake and Offload Data](bake-and-offload.md): At high resolutions, node previews can consume a lot of memory - and they’ll often take longer to process than the same graph at 1K–4K
- [Bake Only What Matters](bake-required.md): Baking is one of the simplest ways to keep large projects fast and memory-friendly - especially when you’re working with Regions, tiled builds, or heavy
- [Graph Hygiene - Orphan Nodes](hygiene-orphans.md): Large graphs tend to accumulate \\"just-in-case\\" experiments: half-finished branches, abandoned masks, and one-off tests that never make it into the final
- [Name Outputs](naming-output.md): Most "pipeline pain" isn’t about file formats - it’s about file paths
- [Create Clean Masks for Materials, Biomes, and Shaders](clean-production-masks.md): Turn Nerthus masks into named production data that materials, biomes, shaders, and engine layers can actually use
- [Deliver a Terrain Package That Others Can Use](deliver-terrain-package.md): Package terrain height, masks, previews, scale notes, and source context so the next artist or tool can use the result
- [Explore Variations Without Breaking Your Main Graph](explore-variations-safely.md): Test terrain variants, seeds, profiles, and art-direction branches without losing the approved baseline
- [Keep Complex Graphs Readable as They Grow](readable-complex-graphs.md): Use Nerthus graph organization tools so large terrain files remain understandable, editable, and safe to hand off
- [Start New Projects with a Reusable Terrain Template](reusable-terrain-template.md): Create a practical starter graph for scale notes, graph stages, common masks, profiles, and export checks
- [Hand Off a Terrain to Another Artist Without Losing Intent](handoff-terrain-intent.md): Prepare a Nerthus graph, notes, outputs, and checkpoints so another artist can continue the work without guessing
- [Build the Right Version Before You Export](build-right-version.md): Avoid sending stale, preview, wrong-profile, or incomplete Nerthus builds by checking what is actually being exported
- [Keep File Nodes Portable with Relative Paths](portable-file-node-paths.md): Keep Nerthus File node inputs working when projects move between folders, artists, machines, or automation
- [Work Faster with Keyboard-First Navigation](keyboard-first-workflow.md): Learn the small set of Nerthus keyboard habits that remove friction from graph work, export checks, and debugging
- [Audit a Graph Before Build with Data View](audit-graph-before-build.md): Use Data View to catch missing exports, required baking, heavy nodes, errors, and orphaned graph work before a build
- [Keep Source, Builds, and Publish Data Separate](source-build-publish-data.md): Organize Nerthus project folders so source files, external inputs, generated builds, and published packages do not get mixed together
- [Debug Macros with Test Nodes Before Export](debug-macros-with-test-nodes.md): Create a small Macro test harness so MacroPorts, exposed controls, masks, and outputs can be validated before export
