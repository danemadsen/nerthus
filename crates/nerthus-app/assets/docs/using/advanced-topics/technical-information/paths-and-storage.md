# Paths and Storage

## Data

Nerthus keeps its settings and everything it makes for itself (logs, autosaves, thumbnails, downloaded updates) in one folder, the data folder:

| System | Data folder |
| --- | --- |
| macOS and Linux | `~/.nerthus` |
| Windows | `%LOCALAPPDATA%\Nerthus` (usually `C:\Users\<you>\AppData\Local\Nerthus`) |

Options › General has a button that opens it. See [Data Folder](../../troubleshooting/diagnostics/data-folder.md) for what is inside.

## Your Nerthus Files

The documents and projects folders can be changed in [Options](../../../ui/interface/options/index.md) › [Paths](../../../ui/interface/options/paths.md).

### Documents

This is where all Presets, Screenshots, and other Nerthus documents are stored.

Default location: `~/Documents/Nerthus` (on Windows, `C:\Users\<you>\Documents\Nerthus`).

### Projects

This is where Nerthus project files are stored by default.

Default location: the `Projects` folder in the documents folder.

### Builds

This is where output files from builds are stored. It is denoted by the token `<Builds>` in [Build Destination](../../../ui/interface/build-options/build.md#build-destination-and-file-handling).

Default location: the `Builds` folder in the documents folder.
