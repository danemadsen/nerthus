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

Click or drag a slider to set its value; hold **Shift** while dragging to snap to its ticks. To type a value instead, double-click the slider, type a number, and press **Enter** (or **Tab**) to keep it or **Esc** to cancel. Clicking elsewhere keeps it only when Options › Node › **Accept values when clicking outside of editable field** is on.

### Nudge Bar

Right-click a slider for the nudge bar: nine buttons that change the value by set steps. A tenth of the slider's range is its large step, and a hundredth its small one.

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

### Seeds

A seed shows on a button: click it for a new random seed, or right-click it to type one. You can type a formula there too, using the project's variables. The **Seeds** menu beside it lists the seeds you saved and the ones drawn recently, so you can go back to one; **Add Current to Memory** saves the current seed, and **Clear Memory** forgets them.

With Options › Node › **Use slider for Seed instead of new Seed control** on, seeds are sliders like other numbers.
