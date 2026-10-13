# Property Editor Toolbar

The Property Editor Toolbar sits on top of the Property Editor. It provides various options related to the selected node in the graph.

**Replace Node**: Click the node's name to list the nodes related to it. Selecting one replaces the current node with the chosen one, keeping its connections. The "Other node..." option lets you search for any other node to replace it with.

**Presets**: Create and apply presets for the current node. Presets can be managed and selected from the menu to quickly configure node settings. See [Presets](../property-editor/presets.md).

**Node menu**: The menu at the right offers:

* **Node State**:
    * **Revert Changes**: Undo recent modifications and return the properties to their previous state.
    * **Reset to Defaults**: Restore the default settings for the properties.
    * **Save State**: Saves the current settings of the properties for later retrieval. Once saved, **Load State** reloads them.
* **Bind all properties**: Binds every property of the node to a new variable. See [Binding Variables](../property-editor/binding-variables.md).
* **Render Intent (Override)**: Customize how the node's output is shown in the viewport:
    * **Default**: Utilizes the node's standard rendering.
    * **Heightfield**: Interprets the output as a height map, ideal for terrain visualization.
    * **Mask**: Renders the output as a mask.
* **Show Help for this Node**: Opens the selected node's page in this documentation.

**Commands**: Node-specific commands show as buttons below the toolbar. A Combine node, for example, has "Swap Inputs" for swapping its inputs and "Add Input" for adding another input. A node that has a gizmo also gets **Toggle Gizmo**, which shows the gizmo for the node in the viewport.
