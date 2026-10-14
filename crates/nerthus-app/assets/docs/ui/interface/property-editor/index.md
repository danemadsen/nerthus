# Property Editor

## Modifying Properties

Every node exposes a set of properties or settings that you can modify to influence the effect that node has on the terrain. When you select a node, the settings become available in the Properties panel in the right side of the interface.

When you change a property, the effect is immediately applied and made visible in the viewport. Most nodes have a limited type of properties:

* **Integer**: such as `1`, `2`, `3`, `4`, etc. They usually define quantity or a finite value, like angle.
* **Decimals**: such as `0.1`, `0.256`, `0.4885`, etc. They usually define the strength or amount of a certain setting. In most cases, small changes can be quite powerful.
* **Toggle**: They usually allow you to turn a feature on and off.
* **Choices**: These take the form of a dropdown where you can select one of many options.
* **Color**: They allow you to choose a color for the color production nodes.
* **File**: They allow you to choose a file from your computer.

Sliders with unmodified (default) values are shown with slightly muted colors. This can be controlled through [Options](../options/index.md).

Some nodes may have unique controls specific to them.

> [!NOTE]
> One of the easiest ways to learn about a node's properties is to experiment. Try different values across the range for each property to observe the effect they have.

> [!WARNING]
> If a setting says "Iteration", "Strength", "Cycles", or "Amount", be cautious. High values may take longer to process.

### Editing Values

Click or drag a slider to set its value; hold **Shift** while dragging to snap to its ticks. To type a value instead, right-click the number at the slider's right end, type, and press **Enter** (or **Tab**) to keep it or **Esc** to cancel. Clicking elsewhere keeps it only when Options › Node › **Accept values when clicking outside of editable field** is on.

What you type can be a formula. It can use the project's variables (by name), `Resolution` (the resolution nodes build at), `PX` (the width of one pixel, `1 / Resolution`), and `m()`, `cm()` and `km()`, which turn a height in metres into the terrain's 0 to 1 heights. For example, `m(250)` on a height slider is 250 metres. A value outside the slider's range is held to it.

### Nudge Bar

Right-click anywhere else on a slider for the nudge bar: nine buttons that change the value by set steps, and a box (when the slider is wide enough) where you can type a value or a formula and press **Enter**. A tenth of the slider's range is its large step, and a hundredth its small one. The bar closes when the pointer leaves it.

| Button  | Description                                  |
| ------- | -------------------------------------------- |
| Half    | Halves the current value.                    |
| Double  | Doubles the current value.                   |
| Min     | Uses the lowest allowed value.               |
| Down 10 | Decreases by a large step.                   |
| Down 1  | Decreases by a small step.                   |
| Default | Resets to the default value.                 |
| Up 1    | Increases by a small step.                   |
| Up 10   | Increases by a large step.                   |
| Max     | Uses the highest allowed value.              |

### Range Sliders

A range slider has two ends. Drag either end, or hold **Shift** and drag to move the whole range. Right-click the left fifth of the slider to type the low value, or the right fifth to type the high value. Right-click in between for the nudge bar of the nearer end; on a range, its large step is a hundredth of the range and its small step a thousandth, and **Default** puts the whole range back. The high end never goes below the low one.

### Seeds

A seed shows on a button: click it for a new random seed, or right-click it to type one. You can type a formula there too, using the project's variables. The **Seeds** menu beside it lists the seeds you saved and the ones drawn recently, so you can go back to one; **Add Current to Memory** saves the current seed, and **Clear Memory** forgets them.

With Options › Node › **Use slider for Seed instead of new Seed control** on, seeds are sliders like other numbers.
