# Autosave and Recovery

## Autosave Menu

The Autosave menu, in the title bar beside the file name, controls Nerthus's automatic saves. Nerthus saves a safety copy of your project in the data folder's `Autosaves` folder at regular intervals. This ensures that your work is periodically backed up, minimizing data loss in the event of unexpected software shutdowns or system failures.

**Remind Me to Save Every.** This option allows you to set the frequency of the Autosave reminders. You can choose from the following intervals **5 minutes, 10 minutes, 15 minutes, 30 minutes**, or **Never**. When the interval passes with unsaved changes, the file name in the title bar turns red until you save.

> [!WARNING]
> Selecting one of these options will configure the application to remind you to save your work at the chosen interval. Nerthus autosaves every 10 minutes by default, and saves a Disaster Recovery file every 5 seconds (see below).

**Reset Timer.** Selecting 'Reset Timer' will restart the countdown of the current Autosave interval. This is useful when you want to make sure that a save happens at a specific point during your work session.

### Revert Options

**Revert to.** Here you can find a list of this session's five most recent Autosave points. Each entry is labeled with a sequential number and the time of the save (e.g., Autosave 4 @ 03:16:46). This feature allows you to revert your project to a previous state, which is extremely helpful if you need to go back to an earlier version of your work due to errors or changes that did not produce the desired outcome.

**Autosave Entry.** Until five autosaves have been made, the remaining places in the list show a disabled 'Autosave Entry'.

**Open Autosave...** Opens any autosave from the `Autosaves` folder, including those of earlier sessions.

**Revert to Saved.** The 'Revert to Saved' button allows you to quickly revert your project to the last manually saved state. This is used to discard any changes made after the last manual save if they are no longer needed or desired.

### Usage Tips

* **Regular Saves**: In addition to relying on Autosave, regularly save your work manually to capture significant changes.
* **Adjust Frequency**: Adjust the Autosave frequency based on the complexity and duration of your tasks. More frequent saves are recommended for larger, more complex projects.
* **Check Autosave Files**: Periodically check the Autosave files to confirm they are being created as expected. This verification helps to avoid potential data loss.

## Disaster Recovery

Sometimes your Nerthus instance might crash, or a hardware failure may cause your computer to crash or reboot. Every 5 seconds, if the project changed, Nerthus saves it on a background thread.

In your [Nerthus Data Folder](../../using/advanced-topics/technical-information/paths-and-storage.md) you will find an `Autosaves/Recovery.terrain` file. When Nerthus starts, it keeps the previous session's copy as `Recovery<date>_<time>.terrain` in the same folder. If Nerthus itself fails, it also saves the project there as `DISASTER_RECOVERY_<name>_<time>.terrain`.

> [!CAUTION]
> It is important to check both the `Recovery` and autosave files which are also in the same folder. If the 5-second-time that saves recovery information has not fired before the crash, then you may need to use the most recent autosave as the latest recovery file may not have saved. This does not happen often, but it is a possibility.

### Version Migration Backup

When Nerthus opens a file saved in an older version, it will save a copy of the original in the Autosave folder with the name convention: `Migration_<originalName>_<time>.terrain`

If a new version is not backward compatible, you can use such migration copies to recover older data in a disaster scenario.
