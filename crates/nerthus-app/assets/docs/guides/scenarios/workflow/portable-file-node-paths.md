# Keep File Nodes Portable with Relative Paths

File nodes are useful until their paths break.

A terrain that depends on `D:\Downloads\height.exr` may work perfectly for the author and fail for everyone else. It may also fail on the same machine later, once the file is cleaned up or moved.

> [!TIP]
> Put external inputs beside the project before the graph depends on them.

## Keep inputs with the terrain

A portable project shape can be very simple:

```text
Project_Terrain/
  source/
    Terrain_Main.terrain
  inputs/
    base_height.exr
    region_mask.png
  builds/
  publish/
```

The goal is that the `.terrain` file and the files it reads travel together.

For a shared folder or client handoff, relative project inputs are usually the safest choice. For automation, bind the File node filename to a variable and pass the path at build time. For a one-machine experiment, an absolute path can be acceptable, but document it before anyone else inherits the file.

## Test portability early

Move external inputs into a stable `inputs` folder. Point File nodes there. Decide whether the project will use relative paths or automation variables. If a command-line build will supply the input path, test that exact command path, not just the UI.

Then copy the project folder to a temporary location and open it. If File nodes resolve there, the project is much safer to share.

> [!WARNING]
> Relative paths must be tested in the workflow that will run the build. UI use and command-line automation can have different assumptions about where a path starts.
