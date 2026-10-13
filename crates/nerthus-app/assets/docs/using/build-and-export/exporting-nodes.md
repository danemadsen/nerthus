# Exporting Nodes

Nerthus is primarily an asset generator designed for creating and exporting terrains to be used in other applications. It supports all major heightfield and mesh formats, ensuring compatibility with various platforms and rendering tools.

## **Output Nodes**

* **Node Types**: Utilize specific output nodes, such as Mesher for mesh generation or Point Cloud for simple XY point cloud export, to export your assets.
* **Customization**: Depending on the output node type, you can adjust settings such as file naming conventions, or mesh topology and vertex count for Mesher.
* **Naming Conventions**: The name of the exported file matches the name of the Output node, streamlining asset management and integration into other projects.

### **Mark for Export**

* **Shortcut Key**: Right-click a node and select `Mark for Export` or press `F3` on any node to mark it for export. This flexibility allows you to export multiple node outputs simultaneously.
* **Build Tab and Settings**: The Build tab and Build Settings window display all nodes marked for export. Here, you can change the output format, select which ports to export, and rename the export files.
* **File Naming**: Exported files are named in the format `<Name or NodeName>_<PortName>.<extension>`, ensuring clarity and order in file management.

> [!NOTE]
> You can disable `_Out` suffix for the Primary Port in [Build Settings](../../ui/interface/build-options/index.md).

### Export Settings and Options

**File name.** The node name is used by default for the export filename, however, you change the name in the Export Items list in the Build Tab in [Data Editor](../../ui/interface/data-editor/index.md) or in the [Build Settings](../../ui/interface/build-options/index.md).

**File Formats.** Nerthus supports a wide range of file formats, allowing you to choose the best option based on the requirements of the receiving application or your workflow needs.

**Port Selection.** From the export menu, you can select specific ports (such as Enabled, Out, Flow, Wear, or Deposits) to export, providing control over the data being exported and optimizing resource usage.

**Additional Options**

* **Primary Only.** Export only the primary or most important outputs of a node, reducing the number of files and focusing on key data.
* **Mark All/None.** Quickly select or deselect all available export options for efficiency and convenience.

> [!NOTE]
> For automation scenarios where you wish to explicitly name the output file, see [Managing Input and Output](../../developers/automation/building-bridges/managing-input-and-output.md) in [Automation](../../developers/automation/index.md)
