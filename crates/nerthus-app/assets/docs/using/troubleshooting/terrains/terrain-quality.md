# Terrain Quality

## Map

### There are banding/stairstep patterns in my displacement

Open the exported heightfield in a good quality image viewer and check whether it is 32-bit or not. If the file has been exported as 16-bit for any reason, then you may find some stairsteps or quantization banding.

Export the file as 32-bit to remove the banding and avoid lossy formats such as PNG.

## Mesh

### Terrain looks faceted

The Normals were not imported properly in your mesh. You can use a Smooth / Autosmooth modifier in your app to fix it, or re-import the mesh and ensure Normals are imported. 

### Terrain is inside out

Your Normals are inverted. This can happen during import in some importers with certain settings. You can fix this with a Normals or Flip Normals modifier in your app. Or re-import and make sure the right settings are set in the Importer.

### Mesh size is tiny/huge

If you have exported the mesh at `Normalized` scale in Nerthus, then it will be exported as a `0..1` scale mesh. Use the `Multiplier` or scaling available in your OBJ importer if available to multiply it by the size you want. Alternatively, you can import it and them scale it up to the size defined in your Nerthus world.

You can also export the terrain in Meters or Kilometers and then import it at that scale.
