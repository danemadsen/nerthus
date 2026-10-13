# Nerthus 0.1.0

Nerthus is a node-based terrain generator. It opens and saves `.terrain` projects and builds them into heightfields, masks and colour maps.

***

## In this version

* 190 nodes and 19 modifiers.
* Projects with variables, bindings and expressions, macros, and mesh import (FBX, Collada, glTF and OBJ).
* The node graph and toolbox, the property area with its parameter editors, graph notes, undo, the 3D viewport and regions.
* Build and Export from the editor or the command line (`nerthus-build`): single images, tiles, Hybrid and Region builds, post-build scripts, and every export format, including meshes, point clouds and baked textures.
* Build profiles, batch builds and mutations, "Maintain a static folder with the latest copy", and the build report (`report.txt` and `report.json`).
* Baking, caching previews to disk (when a project closes, when the editor is idle, and Smart Cache), and the Optimize Memory dialog.
* Compute on the GPU (Metal, Vulkan and DirectX 12) for the nodes that run as compute kernels.
* Tools › Options, the first-run setup, the Console (F12), Verbose Logging and log files, the Artifact Explorer and Run Diagnostics.

See the [README](https://github.com/danemadsen/nerthus#readme) for what is not done yet.
