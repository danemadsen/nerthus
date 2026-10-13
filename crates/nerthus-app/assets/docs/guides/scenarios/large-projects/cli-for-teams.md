# Command Line for Teams

The command line is how you turn a "personal" terrain file into a *repeatable team asset*.

It enables:

- consistent builds across machines,
- quick automation (nightly builds, CI, farm rendering),
- reproducible variations (variables + seed).

Nerthus provides two command line programs:

- `nerthus` (the editor, with a few start-up options)
- `nerthus-build` (builds without the editor)

## Use "Show Build Command Line" First

In the editor, use **Show Build Command Line** in the Data View's Variables tab to get the exact command for the current file, with fully qualified paths and every variable's value.

That gives your team a known-good baseline command to version and reuse.

## `nerthus` Basics

`nerthus` opens the editor. Its options are mostly for troubleshooting:

- `nerthus <file.terrain>` opens a project.
- `--cpu-only` computes on the CPU only for the session.
- `--verbose` turns on Verbose Logging for the session.
- `--diagnostics` opens Diagnostics at once.

> [!TIP]
> Verbose logging is great for debugging, but don’t leave it on permanently unless you need it.

## `nerthus-build` Basics (Build Automation)

`nerthus-build` is what you use to build terrains headlessly.

Minimum viable build:

```sh
nerthus-build build "D:\Project\Terrain_Final.terrain"
```

### Build a specific profile

```sh
nerthus-build build "D:\Project\Terrain_Final.terrain" --profile "Final_8K"
```

### Build a specific region

```sh
nerthus-build build "D:\Project\Terrain_Final.terrain" --profile "Final_8K" --region "Volcano_Closeup"
```

### Ignore baked cache (clean build)

```sh
nerthus-build build "D:\Project\Terrain_Final.terrain" --ignorecache
```

### Set a seed (repeatable variation)

```sh
nerthus-build build "D:\Project\Terrain_Final.terrain" --seed 1337
```

## Working with Variables and Expressions

See [Variables](../../../developers/extensibility/scripting-and-expressions/variables.md) and [Expressions](../../../developers/extensibility/scripting-and-expressions/expressions.md) on how to enable variable driven properties that can be manipulated through the command line.

### Set variables (great for team presets)

Single variable:

```sh
nerthus-build build "D:\Project\Terrain_Final.terrain" -v ErosionStrength=0.63
```

Multiple variables:

```sh
nerthus-build build "D:\Project\Terrain_Final.terrain" -v ErosionStrength=0.63 -v Snowline=0.42
```

Variables from file:

```sh
nerthus-build build "D:\Project\Terrain_Final.terrain" --vars "D:\Project\vars_final.json"
```

## Team Tips That Actually Matter

1. Use **Build Profiles** to standardize output sets (Shape, Detail, Final).
2. Store variable presets in version control (`vars_final.json`, `vars_game.json`).
3. Use `--seed` to make "random" variation repeatable.
4. Prefer `nerthus-build` for heavy builds: an open editor competes with it for memory. For many builds in a row, `nerthus-build host` keeps a build host loaded between them.

---

For full details on accepted command line arguments see [Command Line Automation](../../../developers/automation/cli/command-line-automation.md).
