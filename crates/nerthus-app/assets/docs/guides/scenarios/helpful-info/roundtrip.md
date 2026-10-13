# Avoid Roundtrips

A "roundtrip" is when you rebuild in Nerthus and then have to re-import (or relink) the terrain, masks, bitmaps, and other outputs in your DCC because the file paths changed.

## The problem

If your Build Destination uses an incremental pattern like `[+++]`, Nerthus creates a new output folder for every build. That preserves history, but it also changes the asset paths every time - so your materials and file references in your CG application break and must be updated.

If you use a fixed (static) output path instead, your links stay valid - but each build overwrites the last one, so you lose per-build history.

## The solution

Nerthus has an option in [Build Settings](../../../ui/interface/build-options/index.md) (Build tab) called `Maintain a static folder with the latest copy`.

Enable it *while still using* an incremental destination pattern like `[+++]`:

* Nerthus continues to write each build into its own numbered folder (so you keep every build).
* Nerthus also copies the newest build into a sibling folder named `Latest`.

Point your DCC file links to the `Latest` folder. Your references stay stable across rebuilds, while you still retain archived outputs per build.

Optional: enable `Copy the .terrain file to the build folder` to store an exact snapshot of the terrain file used for each build. This is useful for tracking changes across iterations and rolling back to a specific version.

```

This is what a typical folder structure would look like in this scenario.

c:\Nerthus\Builds\Sample\
├── 001\
│   ├── ErosionPro.exr
│   ├── ErosionPro_Deposits.exr
│   ├── ErosionPro_Flow.exr
│   ├── ErosionPro_Wear.exr
│   ├── report.json
│   ├── report.txt
│   └── Sample.terrain
├── 002\
│   ├── ErosionPro.exr
│   ├── ErosionPro_Deposits.exr
│   ├── ErosionPro_Flow.exr
│   ├── ErosionPro_Wear.exr
│   ├── report.json
│   ├── report.txt
│   └── Sample.terrain
├── 003\
│   ├── ErosionPro.exr
│   ├── ErosionPro_Deposits.exr
│   ├── ErosionPro_Flow.exr
│   ├── ErosionPro_Wear.exr
│   ├── report.json
│   ├── report.txt
│   └── Sample.terrain
└── Latest\
    ├── ErosionPro.exr
    ├── ErosionPro_Deposits.exr
    ├── ErosionPro_Flow.exr
    ├── ErosionPro_Wear.exr
    └── Sample.terrain

```
