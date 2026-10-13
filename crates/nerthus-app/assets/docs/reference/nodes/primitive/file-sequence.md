# FileSequence

FileSequence reads a folder of images and outputs one of them, chosen by its frame number. Each image is resized to the build resolution.

## When to Use It
Use FileSequence when a terrain needs one of many prepared heightfields or masks, for example a series of scans or frames, chosen by a number that a [variable](../../../developers/extensibility/scripting-and-expressions/variables.md) can set for each build.

The images are taken in the order the folder lists them, and are read once and kept until **Purge cache** is turned on. The first build after the node is created or the project opened reads the folder; if it reports an error, build again.

## Properties

#### Folder

The folder holding the images.

#### Frame

Which image to output, counting from 0.

#### Is RGB

Reads the images in color instead of as heightfields.

### Caching

#### Purge cache

Forgets the images read, so the folder is read again. The node outputs nothing while it is on.
