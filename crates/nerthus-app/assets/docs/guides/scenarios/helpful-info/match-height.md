# Match or Correct Heights

When you bring real-world DEMs or heightmaps into Nerthus, the numbers often **don’t match Nerthus’s expected height conventions**. Some datasets are already in meters (with negatives), some are biased, some are scaled oddly, and many don’t line up with how you *intend* to use vertical range in your graph.

A clean way to "re-home" that data into a predictable altitude ceiling - without guessing multipliers - is to use a **Constant + MATCH** workflow. It lets you say: "This imported terrain should top out at **X meters**", and then forces everything to conform.

## The goal: define a correct "highest altitude"

Instead of trying to eyeball scale, you explicitly define the maximum elevation you want your imported terrain to represent (e.g., 2400 m, 8849 m, 500 m - whatever your project needs). Then you match the imported data to that reference.

That means:

* You keep your import **unclamped** (so you don’t crush data).
* You normalize the incoming range (so it behaves nicely).
* You **match** it to a known reference height (so the final amplitude makes sense in Nerthus terms).

## Step-by-step workflow

### 1. Create a Constant as your "altitude ruler"

1. Add a **Constant** node.
2. Set its **Height** to the *highest altitude you want to use*, **in meters**.
3. Nerthus will convert that meter value into its internal percent-based height representation automatically.

Think of this Constant as your "this is what 100% height means for this project" reference.

### 2. Import your dataset with File (keep it unclamped)

1. Add a **File** node and load your DEM/heightmap.
2. Make sure it stays **Unclamped**.

Why: clamping can silently flatten anything below/above the expected range, and imported DEMs often include values outside a neat 0–1 span.

### 3. Autolevel right after the import

Place an **Autolevel** node immediately after **File**.

This does two helpful things:

* It remaps the incoming terrain to a clean min→max range (no wasted headroom).
* It makes the next step (matching) stable and predictable.

### 4. Use MATCH to conform to your Constant reference

1. Add a **MATCH** node after **Autolevel**.
2. Plug the **Autolevel output** into MATCH’s main input.
3. Plug the **Constant** into MATCH’s **Reference Input**.

Now MATCH uses the Constant as the "target height context", scaling your normalized imported terrain so its overall height aligns with the altitude you defined.

## Why this works so well

* **You declare intent** ("max altitude is 2400 m") instead of guessing scale factors.
* **Autolevel** ensures you’re matching a well-behaved, full-range signal.
* **MATCH** then handles the conversion cleanly, so your terrain sits correctly within Nerthus’s vertical expectations.

## Quick practical notes

* If your dataset includes bathymetry / negative elevations, keeping **File unclamped** is important. You can decide later how you want to treat below-sea regions (offset, mask, clamp, etc.).
* If you’re working with tiles, try to apply the same reference Constant across the whole set to keep consistent vertical scale.
* This workflow is about **getting the imported terrain into a correct, predictable altitude framework inside Nerthus**; you can still do final vertical scaling in your DCC/engine if you’re targeting a specific displacement range.

---

# Alternate Technique

If you expect your data to have no negative values, use the File node to import the data, then apply the Autolevel modifier in the [Modifier Stack](../../../ui/interface/property-editor/modifier-stack.md). Then apply the Height Remap modifier and clamp the values to whatever height you want.

This is a much simpler technique and may be useful in most cases. However, for exact precision, it is better to use the sophisticated technique shown above.
