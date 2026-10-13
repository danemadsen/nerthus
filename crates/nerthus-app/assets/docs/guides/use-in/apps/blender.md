# Blender

Assumptions:

* Terrain footprint = **5000 m × 5000 m**
* Heightmap is **0..1**, where **black = 0 m** and **white = 2500 m**
* Blender units = meters

> [!TIP]
> These are Nerthus defaults. Please adjust the steps if your terrain size is different.

---

# Import as a displacement map (heightfield)

## 1. Export from Nerthus

* Export a **Height** map (prefer **32-bit EXR**, otherwise **16-bit PNG**).

## 2. Set Blender units to meters

* **Scene Properties → Units**

  * Unit System: **Metric**
  * Length: **Meters**

## 3. Create a 5000 m plane

1. **Shift+A → Mesh → Plane**
2. Press **N → Item → Dimensions**
   * Set **X = 5000**
   * Set **Y = 5000**
3. `Ctrl+A → Scale`

## 4. Add geometry

* Add **Subdivision Surface** (or Adaptive Subdivision in Cycles) until you have enough detail.

## 5. Displace using the heightmap (material-based)

1. Create a material → Shader Editor:
   * **Image Texture** → load heightmap → set **Color Space = Non-Color**
   * Connect: **Image Texture → Displacement (Height) → Material Output (Displacement)**
2. Material Properties → Settings:
   * Displacement: **Displacement and Bump** (Cycles)

## 6. Set height exactly to 2500 m

* On the **Displacement** node:
  * **Midlevel = 0.0**
  * **Scale = 2500**

Result: black stays at 0 m, white becomes 2500 m.

---

# Import as a mesh (OBJ/FBX)

## 1. Import

* **File → Import → OBJ/FBX**

## 2. Scale to 5000 m footprint

1. Select the imported terrain.
2. Press **N → Item → Dimensions**
3. If X/Y are ~1.0 (the typical 0..1 mesh):
   * Set **Scale X = 5000**
   * Set **Scale Y = 5000**

## 3. Scale height to 2500 m

* If the mesh Z is also ~1.0:
  * Set **Scale Z = 2500**

## 4. Apply

* `Ctrl+A → Scale`

That’s it: footprint becomes 5000 m, and elevation range becomes 0..2500 m.
