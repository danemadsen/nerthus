# Automation View

The Vars tab of the Data View holds the project's [variables](../../../developers/extensibility/scripting-and-expressions/variables.md): one editor per variable, to change its value, in the order they are listed.

Its toolbar has:

- **Create Variable** – a new variable of a chosen type (Float, Int, Range, Choice, Bool, String, Color, or Undefined), or a Separator or Group to organize the list. The new variable opens in Edit Variable.
- **Toggle Edit Mode** – shows each variable as a row to rename, edit, delete, or drag by its grip to another place in the list.
- **Group by Nodes** – lists the variables under the nodes they are bound to.
- **Show Build Command Line** – the command that builds the project from a terminal, with every variable's value. See [Command Line Automation](../../../developers/automation/cli/command-line-automation.md).
- **Create Connected Nodes** – for macros: macro ports for all of the selected nodes' ports, or only their inputs or outputs.
- **Delete all variables**.

To bind a node's property to a variable, use its parameter menu in the property panel. See [Binding Variables](../property-editor/binding-variables.md).
