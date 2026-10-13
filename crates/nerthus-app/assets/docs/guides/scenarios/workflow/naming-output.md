# Name Outputs

Most "pipeline pain" isn’t about file formats - it’s about *file paths*.

If your DCC links keep breaking, your builds feel chaotic, or your team can’t tell which output belongs to what, this is usually a naming and destination strategy problem.

## Know How Nerthus Names Files

By default, exported files follow this pattern:

`<Name or NodeName>_<PortName>.<extension>`

That’s good news: it’s predictable and scales well if you keep names disciplined.

> [!TIP]
> If you don’t want the primary port to add `_Out`, you can disable that suffix in Build Settings.

## A Practical Naming Strategy

### Node naming
Use short, meaningful node names that read well in a folder:

- `Mountain_Base`
- `Volcano_Crater`
- `Erosion_Primary`
- `Masks_Slope`

Avoid:

- `final_final2`
- `test123`
- `lol`

### Port selection
Only export what you will actually use downstream.

- Use **Primary Only** when you don’t need every diagnostic port.
- Export wear/flow/deposits only when your material setup depends on them.

### Tiles (if applicable)
If you’re doing tiled builds, set tile naming so it plays nicely with your tools:

- **Tile Suffix Pattern** like `_y%Y%_x%X%`
- **Add Leading Zeroes** for consistent sorting
- **Start Numbering from 1** if your pipeline expects 1-based indexing
- **Organization**: consider "folders for each node" to prevent collisions

## Avoid Roundtrips

A "roundtrip" is when you rebuild and your DCC links break because the output folder changed.

If you use an incremental destination pattern like `[+++]`, you keep build history - but you also change the folder every build.

The best practice is:

1. Keep the incremental build folders (history).
2. Enable **Maintain a static folder with the latest copy**.
3. Point your DCC to the `Latest` folder.

Now you get:

- stable links in Maya/Houdini/Unreal/etc,
- full build archives for rollback.

> [!TIP]
> If you also enable "Copy the .terrain file to the build folder", each build becomes a fully traceable snapshot.

## Recommended Output Layout

- `_builds/Final/001/ ...`
- `_builds/Final/002/ ...`
- `_builds/Final/Latest/ ...`

And your DCC always references:

`_builds/Final/Latest`

That’s it. No relinking, no guessing, no lost history.
