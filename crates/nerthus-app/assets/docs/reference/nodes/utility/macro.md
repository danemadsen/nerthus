# Macro

A Macro node is an instance of a packaged graph. It lets you reuse a saved `.macro` as a normal node, with the Macro's exposed inputs, outputs, and variables shown as the node's ports and properties.

For authoring and export workflow, see [Building Macros](../../../developers/extensibility/macros/building-macros.md).

## When to Use It
Use Macro when you want to reuse an installed Macro inside the current graph instead of rebuilding the same node chain by hand.

## Properties

Macro node properties are defined by the Macro package:

- exposed variables become user-facing properties
- input and output ports come from the Macro's [MacroPort](macro-port.md) nodes
- defaults, ranges, and labels come from the Macro authoring file

If the expected controls are missing, reopen the original Macro source `.terrain`, check the exposed variables and [MacroPort](macro-port.md) definitions, then export and reinstall the Macro.
