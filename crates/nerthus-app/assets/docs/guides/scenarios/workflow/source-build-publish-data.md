# Keep Source, Builds, and Publish Data Separate

Nerthus projects can produce a lot of files: `.terrain` sources, File node inputs, builds, masks, previews, notes, source snapshots, and old deliveries. If all of that lives in one folder, nobody knows what is editable, what is generated, what is current, or what is safe to ignore.

The fix is not an elaborate pipeline. It is a small folder contract that separates source, generated builds, and clean publish data.

> [!TIP]
> Keep downstream tools pointed at a stable `Latest` or publish folder. Keep numbered build folders as history, not as live links.

## Give every folder one job

A simple project can look like this:

```text
Project_Terrain/
  _notes/
  _inputs/
  _nerthus/
  _builds/
    preview/
    final/
    final/Latest/
  _publish/
    Latest/
  _archive/
```

Use `_nerthus/` for editable `.terrain` files. Use `_inputs/` for files read by File nodes. Use `_builds/` for generated output history. Use `_publish/` for the clean set of files that another tool or artist should consume. Use `_notes/` for scale, preview, and delivery notes.

The exact names can change, but the roles should not blur.

## Set Nerthus to build into the structure

Use [Build](../../../ui/interface/build-options/build.md) to set the destination, and use [Build Path Tokens](../../../ui/interface/build-options/build-tokens.md) when you want predictable dated or numbered folders. If a DCC or engine needs a stable link, enable the static latest folder workflow described in [Avoid Roundtrips](../helpful-info/roundtrip.md).

When reproducibility matters, copy the `.terrain` file into the build folder. When the publish package needs clean names, use [Name Outputs](naming-output.md) before the build instead of renaming files after the fact.

## What goes wrong without this

The common failure is linking a downstream tool directly to a random numbered build folder. A week later, a newer build exists, but the DCC still points at the older one. Or the opposite happens: someone deletes a build folder that was quietly acting as the source of truth.

Generated build folders are history. A `Latest` or `_publish/Latest` folder is a contract.
