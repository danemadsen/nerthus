# Profiles and Batch Builds

Build Profiles let you save all Build Settings in a named preset. If you find yourself creating multiple versions of your terrain, or creating different output types, you can switch between the different build settings swiftly with Profiles.

## Using Profiles

To create a profile:

* Open Build Settings.
* Select the settings you want for a profile
* Go to the Profiles tab and click Save As...

The "Load" button imports the selected build profile into the current build settings.

The "Save As..". button allows you to create a new build profile based on the current build settings.

The "Delete" button removes the currently selected build profile.

> [!NOTE]
> Use ASCII-friendly names for the profile as you may wish to use them in command lines or batch scripts.

## Running Batch Builds

You can execute multiple builds using saved profiles in the Batch Build window. Go to Project › Execute Batch Build… to open it.

Queue the different profiles you want to build (**Add Profile**, or **Add All Profiles**) and execute the batch. You can also save the batch as a script (**Save Batch File**: a `.bat` file on Windows, a shell script elsewhere) for later execution or modification.

The `--profile <name>` or `-p <name>` argument can be used to specify a profile for a [command-line build](index.md). It is recommended that the name be wrapped in `""` quotation marks to prevent incorrect parsing.

See also [Creating Mutations](creating-mutations.md)
