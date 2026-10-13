# General

The **General** tab provides essential application-wide preferences, controlling how Nerthus launches, handles logs, updates and its network connection, and its diagnostic tools. These settings affect global behavior rather than project-specific options.

## **Startup Settings**

* **Start Nerthus on:** Select which monitor Nerthus will open on at launch. Useful for multi-monitor setups.
  * _Monitor 0_ – Opens on the first detected monitor.
  * _Monitor 1 (Primary)_ – Opens on the system’s primary display.

## **Interface Behavior**

* **Move cursor to target when using the Lazy Menu:** Automatically moves the mouse cursor to the selected target in the Lazy Menu for faster interaction.
* **Show Expert Tools:** Enables advanced tools intended for experienced users. May expose complex features not needed for beginners.
* **Show warning for orphan variables:** Warns when a project has variables that no property is bound to.

## **Update and Reporting**

* **Check for updates on startup:** Always on: Nerthus checks for a newer version every time it launches, and shows an **Update** button in the title bar when there is one. Help › Check for updates asks again.
* **Enable Verbose Logging:** Creates more detailed log files for troubleshooting. This can generate large files. Requires a restart to take effect.
* **Report crash reports anonymously** and **Report anonymous usage statistics:** Always off. Nerthus sends no crash reports or usage statistics anywhere.

## **Network**

* **Use Web Proxy:** Enable and configure a proxy server for network communications (the update check and update downloads) if your environment restricts direct connections. With it off, Nerthus connects directly; with it on and no proxy named, it uses the system's.

## **Utilities**

* **Open Data Folder:** Opens Nerthus’s [data folder](../../../using/troubleshooting/diagnostics/data-folder.md), which holds its logs, autosaves and settings.
* **Run Diagnostics:** Opens [Diagnostics](../../../using/troubleshooting/diagnostics/index.md), which detects and troubleshoots common issues.
* **Factory Reset:** Opens Diagnostics, where **Reset to factory defaults** puts all preferences and settings back to their original defaults.

> [!WARNING]
> Verbose logging should only be enabled when troubleshooting to avoid excessive disk usage.
