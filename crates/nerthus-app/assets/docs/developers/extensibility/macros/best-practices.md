# Macro design best practices

Macros are easiest to adopt when they behave like native nodes: one clear purpose, sensible defaults, and controls that match how people actually work. Use the guidelines below to keep Macros maintainable for you and usable for everyone else.

## Keep it simple

Design the graph and the UI so someone can understand the Macro quickly.

* Prefer a short, obvious node chain over clever branching.
* Avoid "do-everything" graphs. If you need multiple distinct behaviors, split into multiple Macros.
* Keep defaults conservative: the Macro should produce a reasonable result immediately when dropped into a graph.

## Follow Single Responsibility Principle (SRP)

A Macro should do one primary job.

* Define the Macro’s purpose in one sentence ("adds terrace shaping", "builds a ridge mask", "softens talus breakup").
* Secondary uses are fine, but they should be natural extensions of the primary purpose, not unrelated features.
* If you find yourself adding controls for a second "mode" that’s often a sign you should make a second Macro.

## Don’t create too many controls

A large control surface often confuses users more than it helps.

* Expose only the parameters that materially change the output in predictable ways.
* Avoid exposing multiple internal controls that do the same thing at different stages.
* If a control is rarely touched, keep it internal and set a good default.

## Separate Standard vs Advanced controls

If you must provide many controls, organize them so most users never need to touch the complex ones.

* **Standard** controls: the handful of sliders/toggles needed for good results.
* **Advanced** controls: optional tuning for edge cases, specialist workflows, or style matching.

Rules of thumb:

* Advanced controls should never be required to get a strong output.
* Advanced controls should have safe defaults that preserve expected behavior.
* Group advanced controls together and keep their naming explicit (e.g., "Advanced: Falloff Bias", "Advanced: Clamp Range").

## Favor meaningful controls over technical controls

Expose controls in terms of outcomes, not implementation details.

* Prefer "Intensity", "Scale", "Detail", "Falloff" over internal node names or raw values.
* If the Macro uses multiple steps internally, avoid exposing each step’s knob unless the user benefit is clear.
* When exposing a parameter, choose the single control that best represents the user’s intent.

## Avoid heavy macros

Overly expensive Macros slow down graphs and can hurt large tiled builds.

* Minimize expensive node stacks inside one Macro.
* Avoid duplicating high-cost branches when a shared result can be reused internally.
* Treat RAM usage as a first-class constraint: excessive memory in one Macro can cause slowdowns and instability in large builds.
* If your Macro is inherently expensive, make it explicit in the name/description and keep the control surface small.

## Prefer resolution-independent behavior

A Macro that looks good only at one build size is fragile.

* Test at multiple resolutions (low, mid, high) and confirm the look scales sensibly.
* Avoid settings that are unintentionally "pixel-sized" unless that is the point of the Macro.
* If the Macro depends on feature size, make feature size a user-facing parameter so it can be retuned per project.

## Test like a user will

Validate the Macro under real usage conditions, not only on your ideal test terrain.

* Test against different types of inputs (flat, noisy, steep, smooth).
* Try extreme inputs and parameter ranges to catch artifacts (banding, clipping, unexpected inversions).
* Drop the Macro into a larger graph and confirm it remains predictable when upstream changes.

## Make the macro graph maintainable

Macros get revised. Make revisions easy.

* Keep the internal graph readable: align nodes, keep wiring clean, avoid unnecessary crossovers.
* Use clear internal node naming where it helps (especially for non-trivial graphs).
* Keep the path from `In` to `Out` obvious so future edits don’t accidentally change semantics.

## Version and compatibility discipline

Users will rely on your Macro in existing graphs.

* Avoid breaking changes to exposed parameters once the Macro is in use.
* If you must change behavior significantly, consider publishing a new Macro variant rather than silently changing semantics.
* Re-export after edits and verify existing graphs still load and behave as expected.

## Validate export and reload

Your distribution workflow should be repeatable.

* After exporting, restart Nerthus and confirm the Macro appears and instantiates correctly.
* Create a quick "smoke test" graph that you use to validate every revision.
* If you share Macros with others, test a clean install path (import into an environment that doesn’t have your source file open).

## Give meaningful names

A Macro name should describe the result, not the vibe.

* Name it for what it produces or changes: terrace, ridge mask, sediment slide, cliff breakup, etc.
* Prefer clear, searchable words over novelty names. "UltraTerra9000" is unsearchable and gives no intent.
* Use consistent naming patterns so a toolbox list reads cleanly:
  * Verb + Noun: `MakeTerraces`, `BreakTerraces`, `GenerateRidgeMask`
  * Noun + Descriptor: `Terraces_Broken`, `Sediments_Sliding`, `Mask_Ridges`
* Avoid baking versions into the name (`_v2`, `_final`). Use Macro metadata/versioning for that; keep the node name stable.
* If the Macro is specific (and that matters), include the qualifier: `Terraces_Broken_Soft`, `Mask_Ridges_Wide`, `ErosionAssist_Talus`.

Examples:

* Good: `BrokenTerrace`, `SlidingSediments`, `TerraceMaker`, `RidgeMask`
* Bad: `UltraTerra9000`, `MyMacro`, `Test2`, `CoolShaperThing`
