# Node

The **Node** tab manages the appearance and interaction behavior of nodes in the graph workspace, allowing customization of visual style, connection handling, and data display precision.

## **Node Style**

* **Muted** – Uses a softer, less saturated color scheme for nodes, improving focus on the graph structure. _(Default)_
* **Vibrant** – Uses brighter, high-contrast colors.

## **Show Shine Effect on Node When Build Completes**

* **Enabled:** A visual shine animation plays on a node after it finishes building, providing a visual cue for completion.
* **Disabled:** No build-complete effect is shown.

## **Show Build Time on Node**

* **Enabled:** Displays the time taken to build each node directly on the node itself.
* **Disabled:** Build times are hidden.

## **Show Values with High Precision**

* **Enabled:** Displays node parameter values with extended decimal precision for finer adjustments.
* **Disabled:** Values are rounded for cleaner display.

## **Accept Values When Clicking Outside of Editable Field**

* **Enabled:** A value being typed is kept when you click elsewhere.
* **Disabled:** Only **Enter** or **Tab** keep a typed value; clicking elsewhere drops it.

## **Use Slider for Seed Instead of New Seed Control**

* **Enabled:** Seeds are set with a slider, like other numbers.
* **Disabled:** Seeds use the seed control, with its button for a new random seed.

## **Snap Connections to Available Ports**

* **Enabled:** Connection lines automatically snap to valid ports when dragging links between nodes. _(Recommended)_
* **Disabled:** Freeform connections without snapping, requiring precise placement.

## **Double-Click on Port to Connect**

* **Enabled:** Allows connecting two ports by double-clicking instead of dragging.
* **Disabled:** Only drag-and-drop connections are available.

## **Always Show Mask Ports**

* **Enabled:** Shows a "Mask" port on every node that can be masked.
* **Disabled (Recommended):** Drop a connection onto the middle of a node and select **"Add as Mask"** instead of using dedicated ports.
* _(Existing files are unaffected either way.)_

## **Notes**

* The **modern mask workflow** is the preferred approach, simplifying port layouts and improving graph readability.
* High precision display is useful for technical fine-tuning but may visually clutter the node UI.
* Visual effects such as **shine on build completion** can be disabled for performance on lower-end systems.
