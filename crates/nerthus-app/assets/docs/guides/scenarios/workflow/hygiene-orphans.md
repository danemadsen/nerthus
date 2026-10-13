# Graph Hygiene - Orphan Nodes

Large graphs tend to accumulate "just-in-case" experiments: half-finished branches, abandoned masks, and one-off tests that never make it into the final output.

Those loose ends are called **orphan nodes** - nodes that aren’t connected (or don’t contribute to any final output). Keeping them around adds clutter, increases mental load, and can waste resources.

## Find Orphans Fast

The easiest way to locate orphans is via the Data View.

1. Open **Data View → Terrain tab**.
2. Use the **Filter** dropdown.
3. Choose **Orphan Nodes**.

Nerthus will isolate the nodes that aren’t participating in the graph’s outcome, so you can deal with them intentionally.

> [!TIP]
> While you’re here, also check:
> - **Nodes with Error** (to catch broken links and invalid setups)
> - **Heavy Nodes** (to find performance bottlenecks)

## Decide What to Do with Them

Once you have the orphan list, pick one of these outcomes:

### Delete
If you’re confident the branch is dead, delete it. This keeps the graph readable and reduces "false complexity".

### Park It
If you want to keep an experiment for reference:

1. Create a dedicated **Scratch** tab or group.
2. Move orphans into it.
3. Rename the group clearly: `SCRATCH - do not ship`.

This keeps your production graph clean while preserving R&D work.

> [!WARNING]
> Don’t let "maybe later" nodes live in the main graph area. If it’s worth keeping, it’s worth parking.

## Keep the Graph Future-Proof

A clean graph scales better when:

- someone else opens it months later,
- you need to split work across multiple files,
- you build at extreme resolutions (8K/16K or tiled).

Treat orphan cleanup like sweeping the workshop floor: small habit, huge payoff.
