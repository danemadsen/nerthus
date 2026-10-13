# Debug Macros with Test Nodes Before Export

When you edit a Macro, the MacroPort input represents data that will arrive later. During authoring, that input may be empty. The Macro can look broken even when the problem is simply that no test terrain is flowing through it.

Before exporting a Macro, give it a small test harness.

> [!TIP]
> Test the Macro with several simple inputs before installing it in a clean graph.

## Build a temporary test harness

Place a few temporary inputs near the Macro source graph and use them to feed the route that normally starts at the input MacroPort.

Useful test inputs include:

* a Constant to reveal unwanted change on flat terrain
* Noise or Perlin to check normal shaping behavior
* a Mountain or ridge to check terrain-scale behavior
* a steep shape to find clipping and mask edge cases
* a smooth shape to catch artifacts on gentle terrain
* a test mask to check polarity and range

Disconnect or clearly isolate those nodes before export if they are not part of the Macro itself.

## What to check before export

Make sure the Macro has one clear job. Expose only controls that change the output in useful ways. Test low and high resolutions. Try more than the ideal input. If the Macro accepts masks, check black and white behavior deliberately.

After export, restart Nerthus and test the installed Macro in a clean graph. That catches missing assumptions from the source file.

> [!WARNING]
> Do not ship private debug branches inside a shared Macro unless they are intentionally exposed and documented.
