# Building Macros

A Macro packages a small node graph and lets you reuse it as a single node. The node used to define Macro inputs and outputs is `MacroPort`.

## Macro structure

A Macro graph is typically:

* **In**: a `MacroPort` node set to `Direction = Input`
* **Out**: a `MacroPort` node set to `Direction = Output`
* One or more processing nodes in between (for example, `Shaper`)

Rename the ports to something clear such as `In` and `Out` so the source graph stays easy to read and maintain.

## Quick Start

1. Create a new Nerthus file.
2. Add a `MacroPort` node, rename it to `In`, and set `Direction` to `Input`.
3. Add a second `MacroPort`, rename it to `Out`, and set `Direction` to `Output`.
4. Set the `Display` type for each port as needed, such as `Heightfield`, `Mask`, or `Color`.
5. Place your processing nodes between them (for example, add a `Shaper`).
6. Wire the graph so the flow is `In > (your nodes) > Out` (for example, `In > Shaper > Out`).
7. Enable `Is Optional` on any `MacroPort` that should not be required.

## Expose parameters (bind internal settings)

To expose a setting from a node inside the Macro:

1. Select the internal node (for example, `Shaper`).
2. In its Properties panel, click the diamond Property Editor for the setting you want to expose.
3. Choose `Bind to > New variable` to create a new exposed control, or bind to an existing variable.
4. Give the variable a clear name and sensible range.

Repeat for any other settings you want to control from outside the Macro. These exposed variables become the user-facing controls for the Macro and can also be driven through automation. For more background, see [Variables](../scripting-and-expressions/variables.md) and [Expressions](../scripting-and-expressions/expressions.md).

## Export the Macro

1. Go to `File > Export > Macro...`
2. Fill in the metadata such as author name, Macro name, version, and description.
3. Choose whether to save the `.macro` file, install it in Nerthus, or both.
4. Export the Macro.
5. **Restart Nerthus to test the Macro.**

> [!TIP]
> Keep the original Macro source `.terrain` file. The exported `.macro` is the installable package, but the source file is what you should reopen when you want to edit, fix, or improve the Macro later.

## Install and use a Macro

1. Install a Macro by dragging the `.macro` file into Nerthus, or use `File > Import > Macro`.
2. Installed Macros are stored in `\Data\Macros` inside your Nerthus installation and can be accessed quickly through the `Artifact Explorer`.
3. Create the Macro by searching for its Macro name in the toolbox or search like any other node.

## Edit a Macro

1. Open the original Macro source `.terrain` file.
2. Modify nodes, connections, `MacroPort` definitions, and bindings as needed.
3. Export the Macro again (`File > Export > Macro...`).
4. Keep the same Macro ID if you want the installed Macro to update in place, and increase the version or revision number so the update is easy to track.
5. **Restart Nerthus to ensure the updated Macro is loaded.**
