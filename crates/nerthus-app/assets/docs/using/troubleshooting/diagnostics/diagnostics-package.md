# Diagnostics Package

Nerthus Diagnostics can create a Diagnostics Package that helps examine crashes or incorrect behavior in Nerthus, for example when you report a problem.

It takes a few moments for the package to be created. The resulting diagnostics package is a `.zip` file named `Nerthus_Diagnostics_<date>_<time>.zip`, placed in your Nerthus documents folder (Options › Paths › User Files), whose window opens once it is ready.

## Package Contents

The diagnostics package contains:

* `summary.json`: the Nerthus version, the operating system, the processor's architecture and core count, and how much memory Nerthus and its cache were using.
* `Devices.txt`: the compute devices Nerthus found, and which of them are turned on.
* `SessionLog.txt`: the log of the current session so far.
* `Logs/`: the ten newest log files from the data folder, crash logs included. These are detailed chronological entries of events inside the program, especially technical readouts and error traces.
* `Settings/`: a copy of your settings and benchmark scores, so the same environment can be recreated.

### Privacy

The package stays on your computer: Nerthus sends it nowhere. Almost all of its data is plain text that you can read before sharing it.

### Editable Logs

The zip file is editable. If you find any information you prefer not to share (a user name, or a file name that describes a private project), you may remove it before sharing the package.
