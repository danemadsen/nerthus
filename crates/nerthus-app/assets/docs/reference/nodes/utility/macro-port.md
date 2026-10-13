# MacroPort

A MacroPort defines an external input or output for a Macro source graph. Each MacroPort becomes a port on the exported Macro node, so the port name, direction, type, and optional state should be set before export.

For the full authoring flow, see [Building Macros](../../../developers/extensibility/macros/building-macros.md).

## When to Use It
Use MacroPort when you are building a Macro and need to declare what data can enter or leave the packaged graph.

## Naming Ports

Rename MacroPort nodes to clear names such as `In`, `Mask`, or `Out` before export. Those names make the Macro source easier to read and help the exported Macro's ports make sense in other graphs.

## Properties

#### Direction

Specifies whether the macro port is used for input or output connections. Determines the flow of data into or out of the macro node.

- **Input**: Designates the port as an input, allowing data to be received by the macro node.
- **Output**: Designates the port as an output, allowing data to be sent from the macro node.

#### Display

Defines the type of data visualized or represented by the macro port. This helps users identify the nature of the connection in the node graph.

- **Heightfield**: Represents terrain elevation data, typically used for heightmaps.
- **Mask**: Represents mask data, used for isolating or selecting specific regions in the terrain.
- **Color**: Represents color data, used for visual effects or texture information.

#### Is optional

Allows the exported Macro to build even when this port is not connected.

#### Fallback

When the given input fails or is intended to short-circuit, return the selected map.

- **Null**: Return a null value. May cause node to be disrupted, but needed for Switch and other mechanisms.
- **Black**: A black (0) value map.
- **White**: A white (1) value map.
- **Grey**: A grey (0.5) value map.
