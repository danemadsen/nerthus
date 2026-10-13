# Creating Mutations

Mutations are a great way to create multiple variations of the same terrain with minimal effort.

In its most basic form, mutations update the seeds used by all nodes in the terrain unless a node is baked. When you use the `--seed <number>` command line argument, it increments each seed in every node by that amount.

This provides a clean and predictable way to generate multiple mutations.

> [!NOTE]
> To prevent specific nodes from being affected, you can try [Baking Nodes](../../baking-nodes/index.md).

## Using the Batch Build Manager

The fastest way to set up mutations is to go to Project › Execute Batch Build… and use the `Add Mutation` button to queue up as many mutations as you want.

If you find yourself repeating these builds, use **Save Batch File** to save the batch as a script: a `.bat` file on Windows, a shell script elsewhere.

> [!NOTE]
> Use the `[Mutation]` token in the [Build Destination](../../../ui/interface/build-options/build.md#build-destination-and-file-handling) to add the current mutation as part of a folder name or subfolder.
>
> For example:
>
> `<Builds>\[Filename]\[+++]\[Mutation]` 
>
> or 
>
> `<Builds>\[Filename]\[Region]_[Mutation]`
