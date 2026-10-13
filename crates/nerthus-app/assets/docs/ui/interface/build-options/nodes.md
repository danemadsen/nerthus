# Nodes

The **Nodes** tab lists all nodes marked for export (**F3**), grouped by graph tab, and lets you adjust their output settings before building.

**Change formats for all nodes** sets one file format for every listed node. When two nodes would write files of the same name, a warning appears: click it to rename them apart.

## Node Export Settings

* **Port Selection**: From the export menu, you can choose specific ports, such as `Enabled`, `Out`, `Flow`, `Wear`, or `Deposits`, to control exactly which data is exported.
* **Primary Only**: Export only the primary or most important output of a node.
* **Mark All/None**: Quickly select or clear all available export options.
* **Filename**: The name specified here becomes the base filename used during export. Files are written in the format `<NameOrNodeName>_<PortName>.<extension>`.
* **File Formats**: Choose the format that best matches the requirements of the receiving application or workflow.

> [!NOTE]
> You can disable `_Out` suffix for the Primary Port in the [Build](build.md) section.
