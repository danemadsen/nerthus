# Var

Reads or modifies named graph variables for procedural control and automation.

## When to Use It
Use Var when a node should read a project variable, or change one as the graph builds.

## Properties

#### Edit mode

Specifies how the variable is used in the node. 'Consume' reads the value of an existing variable, while 'Modify' changes the value or applies an expression to the variable.

- **Consume**: Reads and uses the value of the specified variable without altering it.
- **Modify**: Applies an expression or new value to the variable, updating its value for subsequent nodes.

#### Variable name

The name of the variable to consume or modify. This should match the variable defined elsewhere in the graph or macro.

#### Expression

An optional mathematical or logical expression to apply when modifying the variable. Used only in 'Modify' mode.

#### Default value

The value to use if the specified variable is not set or available. Ensures the node has a fallback value.
