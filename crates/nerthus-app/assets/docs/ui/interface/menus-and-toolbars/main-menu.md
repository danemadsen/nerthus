# Main Menu

## File Menu

* **New**: Start a new project.
* **Open**: Open an existing project file.
* **Open Recent**: Access recently opened files.
* **Open Examples**: Open one of the example projects.
* **Start Dialog**: Opens the start dialog.
* **Save**: Save current project.
* **Save As**: Save project under a new name.
* **Save Incremental**: Save a versioned copy of the project.
* **Repair File**: Attempt to fix corrupted files.
* **Import**: Bring a macro, another project's graph, a TORLET graph fragment, or an image into the project.
* **Export**: Save a macro, or a graph tab as a TORLET graph fragment.
* **Exit**: Close Nerthus.

## Edit Menu

* **Undo**: Revert the last action.
* **Redo**: Reapply the last undone action.
* **Cut**: Remove selected nodes and copy to clipboard.
* **Copy**: Copy selected nodes to clipboard.
* **Paste**: Insert nodes from clipboard.
* **Copy Properties**: Copy properties of selected node.
* **Paste Properties**: Apply copied properties to another node.
* **Rename**: Rename the selected node.
* **Duplicate**: Create a copy of the selected node(s).
* **Delete**: Remove selected item(s) from the graph.

## Graph Menu

* **New Graph Tab**: Adds a graph tab. See [Organizing with Tabs](../../graph/organization/organizing-with-tabs.md).
* **Add Note**: Adds a note object to graph surface. See [Annotations](../../graph/organization/annotations.md).
* **Zoom**:
  * **Extents**: Zooms to display all nodes.
  * **Reset Zoom**: Resets the zoom level to the default view.
  * **Selection**: Zooms to fit the selected nodes within the view.
* **Graph Surface**:
  * **Show Grid**: Toggle the visibility of the grid overlay on the graph.
  * **Snap to Grid**: Enable or disable alignment of items to the grid lines.
  * **Snap to Items**: Toggle whether items snap to align with other items on the graph.
* **Bake** (see [Baking Nodes](../../../using/baking-nodes/index.md)):
  * **Bake**: Bakes the selected node.
  * **Bake Required Nodes**: Bakes every node that must be baked before a Hybrid or Tiled build.
  * **Force load baked cache**: Loads the baked nodes' data again.
  * **Unbake Selected**: Unbakes the selected node.
  * **Unbake All**: Unbakes every node.
  * **Resolution**: The bake resolution: 1024x1024, 2048x2048, 4096x4096, or 8192x8192.
  * **Reload SmartCache to RAM**: Loads cached previews back into memory.
  * **Cache to Disk**: Whether previews are cached to disk **When closing file** and **When Idle**, and **Delete Cache** to remove the cache on disk.
* **Bookmarks**: Bookmarks the current node, and lists the bookmarked nodes. Clicking one navigates directly to that node in the graph. See [Node Bookmarks](../navigation/node-bookmarks.md).

## Node Menu

* **Refresh**: Rebuild the node.
* **Bypass**: Temporarily disable a node.
* **Lock Preview to Node**: Set preview exclusively to selected node.
* **Mark for Export**: Designate node for export.
* **Reset**: Reset node to default state.
* **Mutate**: Mutate the current node.
* **Locked 2D Viewport**: Opens a 2D viewport locked to the node. See [2D Viewport](../viewport/2d-viewport.md).
* **Select Upstream** and **Select Downstream**: Select all nodes feeding the node, or fed by it.
* **Group / Ungroup**: Create a group for selected Nodes. If a group is selected, it ungroups the nodes.
* **Combine nodes**: Combines the selected nodes.
* **Convert port to portals**: Select an output port from the list to convert it into a portal.
* **Move to Tab**: Select from a list of graph tabs to move the current node to your chosen tab.
* **Presets**: Access saved node presets.

## Preview Menu

* **Resolution**: Set the preview resolution (512x512 to 4096x4096).
* **Regions**: Preview one of the terrain's regions.
* **Engine Active** / **Engine Suspended**: Suspend or resume the engine. See [Suspending Engine](../../graph/basic-workflow/suspending-engine.md).
* **Clean Memory**: Free up allocated memory.
* **Clear Cache**: Clear cached data.
* **Rebuild All**: Rebuild the project from scratch.
* **Save Screenshot**: Save a screenshot of the viewport (**F9**).
* **Screenshot Options**: **Save Graph Screenshot as...**, **Save Viewport Screenshot as...** (**Ctrl + F9**), **Open Screenshots Folder**, and the screenshot's resolution (1x, 2x or 4x the viewport's size).
* **Record Turntable Animation**: Create a turntable animation by setting the frames and resolution.
* **Preview Renderer**: The viewport's [render mode](../viewport/render-modes.md).

## Project Menu

* **Build and Export**: Build project and export assets.
* **Execute Batch Build**: Execute multiple builds in a queue using build profiles.
* **Build Settings**: Configure build parameters.
* **Open Build Folder**: Open directory with build files.
* **Statistics**: Each node's build time and the memory its cache takes.
* **Recent Files** and **Open Autosave**: Reopen a recent project, or an autosaved copy.

## Tools Menu

* **Interface Scale**: Adjust interface scaling.
* **Run Diagnostics**: Execute system and application checks. See [Diagnostics](../../../using/troubleshooting/diagnostics/index.md).
* **Artifact Explorer**: The folders Nerthus leaves files in (caches, logs, autosaves and more), each with its size, a link that opens it, and **Delete…** to clean up all its files or those older than 45 days.
* **Console**: Open console for debugging.
* **Verbose Logging**: Enables detailed logging for enhanced troubleshooting and analysis.
* **MCP Server**: Lets AI assistants (MCP clients such as Claude Code) edit and build the open project. Stays on for later starts until turned off.
* **Options**: Access application settings.

## Help Menu

* **User Guide**: Open this documentation.
* **Node Reference**: Open the [Node Reference](../../../reference/index.md).
* **Shortcut Reference**: List of keyboard shortcuts.
* **What's New**: View recent updates in the software.
* **Check for Updates**: Manually check for updates.
* **Diagnostics**: Open diagnostics tools.
* **About Nerthus**: Display version and credits for Nerthus.
