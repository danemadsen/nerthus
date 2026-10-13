# Procedural Workflow

## Understanding Node-Based Terrain Creation

In Nerthus, terrain creation and manipulation are handled through a node-based system. This system may be new if you're accustomed to different types of software interfaces. Understanding how nodes work and interact in Nerthus will enable you to efficiently create complex and realistic terrains.

> [!NOTE]
> You can skip straight to creating your first terrain in [Introduction](../../../using/getting-started/basics/introduction.md).

### What is a Node?

A node is a fundamental element in Nerthus’s terrain design process. It represents a specific operation or function - such as generating a basic terrain shape, applying erosion, modifying texture, or altering elevation. Each node contains settings that you can adjust to change its effect on the terrain.

### Node Graph

The node graph is the workspace in Nerthus where you visually connect different nodes to build and customize your terrain. It functions similarly to a flowchart, where the output of one node becomes the input to another, creating a chain of processes that define the final landscape.

### Key Concepts

#### **Node Chain**

* **Sequential Connection**: Nodes are connected in a sequence to form a chain. Each node processes its input and passes the result to the next node in the sequence.
* **Dependency**: The output of one node often depends on its predecessor, which underscores how crucial the order of the nodes is to the final outcome. 

#### **Adjustable Parameters**

* Each node features parameters that can be adjusted to refine its effect. These parameters are visually represented in the node’s properties panel.

#### **Visual Feedback**

* The node graph gives immediate visual feedback, showing how changes to a node’s parameters affect the terrain. This real-time update facilitates iterative and experimental design processes.

### Getting Started with Nodes

1. **Creating Nodes**: Right-click within the node graph area and select a node from the context menu, or type the name of the node if you know it.
2. **Connecting Nodes**: Drag from the output port of one node and connect it to the input port of another node to establish a data flow.
3. **Configuring Nodes**: Click on a node to view and adjust its parameters in the properties panel to achieve the desired terrain effect.
4. **Previewing Changes**: Use the preview window to see how your adjustments affect the terrain. Experiment with different settings to understand the role of each node.

### Tips for New Users

* **Start Simple**: Begin with basic nodes and gradually add more complex nodes as you become comfortable with the workflow.
* **Use Presets**: Many nodes come with presets that are good starting points for achieving specific terrain types.
* **Experiment**: The node-based system encourages experimentation. Try different combinations of nodes and settings to discover new terrain possibilities.

Node-based terrain creation in Nerthus offers a powerful, flexible approach to landscape design. By understanding and utilizing the node graph effectively, you can create detailed, customized terrains that are limited only by your imagination.
