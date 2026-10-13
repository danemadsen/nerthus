# 3dsmax

Assumptions:

* Terrain footprint = **5000 m × 5000 m**
* Heightmap is **0..1** where **black = 0 m** and **white = 2500 m**
* You want real-world scale in meters

> [!TIP]
> These are Nerthus defaults. Please adjust the steps if your terrain size is different.

---

# A. Import as a displacement map (heightfield)

## 1. Set Max units to meters

1. **Customize → Units Setup…**
   * Display Unit Scale: **Meters**
2. Click **System Unit Setup…**
   * System Unit Scale: **1 Unit = 1.0 Meters**

(Do this before importing/creating geometry.)

## 2. Create a 5000 m plane

1. **Create → Geometry → Standard Primitives → Plane**
2. In the Plane parameters:
   * **Length = 5000**
   * **Width = 5000**
   * Segments: set reasonably high (or use TurboSmooth later)

## 3. Apply displacement

1. Select the plane.
2. Add **Modifier: Displace**
3. In Displace:
   * **Map**: assign a **Bitmap** and load the heightmap
   * Ensure the bitmap is treated as linear/non-color if you’re in a color-managed pipeline (i.e., don’t apply sRGB/gamma to the height data)

## 4. Set height to 2500 m

In **Displace**:

* **Strength = 2500**
* **Decay = 0**

If the displacement is inverted, flip it by either:

* Checking **Flip** in the Displace modifier (if available), or
* Inverting the bitmap via **Output** (RGB Level/Invert) or a simple inversion map.

## 5. Add geometry detail

* Increase plane segments until you get acceptable fidelity, or:
* Add **TurboSmooth** *before* Displace (common for clean results), or
* Use a subdivision workflow suitable to your renderer.

Result: white areas displace up to ~2500 m above black.

---

# Import as a mesh (OBJ/FBX)

## 1. Import the mesh

* **File → Import → Import…** (OBJ/FBX)

## 2. Scale the mesh to meters

1. Select the mesh.
2. Open the **Transform Type-In** (right-click Move/Rotate/Scale or use the toolbar).
3. Scale it to:
   * **X = 5000**
   * **Y = 5000**
   * **Z = 2500**

## 3. Reset transforms (important)

After scaling:

* **Utilities → Reset XForm → Reset Selected**
* Collapse the stack (right-click → **Convert to Editable Poly** or collapse modifier stack)

This prevents downstream modifier/render issues caused by non-uniform scaling.

---

# Common gotchas

* **Gamma/Color management:** heightmaps must be treated as data (linear), not as a color texture.
* **Insufficient subdivisions:** displacement needs dense geometry; otherwise it will look faceted or "steppy".
* **Clipping/viewport:** very large scenes may require viewport clip adjustments; use meters consistently.
