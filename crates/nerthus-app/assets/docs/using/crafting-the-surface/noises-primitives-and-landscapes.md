# Noises, Primitives, and Landscapes

Nerthus provides three types of building blocks:

* Basic Primitives: Noises, Constants, File Inputs, Shapes.
* Landform Primitives: Standalone Mountains, Hills, Craters, etc.
* Landscapes: Mountain Ranges, Fields, Ridge Lines, Rolling Hills, etc.

## Building with Landform Primitives

The Landform Primitives and Landscapes are designed to provide fairly rudimentary yet clearly defined geological shapes to help you quickly establish a terrain. While you have the ability to wrangle Noises and Gradients/Shapes into various terrain-like shapes, these primitives provide a more efficient and friendly way of crafting terrains.

Landscapes help create the "ground", while the Primitives help create standalone features that can be placed on the ground, or sometimes be used by themselves to create free-standing assets.

Both Landscapes and Primitives will have some level of natural processing built-in, such as Erosion, and will mostly have the option to turn it off as well. However, these are meant only as a starting point. You will need to add proper [Erosion](../../reference/nodes/simulate/erosion.md) and possibly other [Surface Nodes](surface-nodes.md) to turn it into a more realistic environment.

## Basic Primitives

Basic Primitives, while being deceptively simple, are foundational components of Nerthus. They provide low-level access to shape generation that can be used to create both precise shapes - whether geometric or naturalistic - or create randomness that can mimic nature.

Advanced users will be able to craft complex outputs from these primitives.
