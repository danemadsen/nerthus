# Measurement Tools

## Height Picker

The Height Picker tool in Nerthus allows you to sample the exact height at any point on your terrain, making it easy to understand elevation and analyze terrain features.

### **How to Use the Height Picker**

1. **Hover to View Height**: Move your cursor over any part of the terrain in the 2D viewport. The Height Picker will display the current height in real-time.
2. **Click to Sample Height**: When you click on a point, a dialog box will appear with detailed information about the height at that specific location.
3. **Information Displayed**:
   * **Metric Height**: Shows the height in meters at the sampled point.
   * **Percentage**: Indicates the height as a percentage of the maximum terrain height.
   * **Raw Value**: A normalized value (0 to 1) representing height, useful for specific procedural operations.
4. **Copy Options**:
   * **Copy in Metric**: Copies the height in meters to your clipboard.
   * **Copy in Percentage**: Copies the height as a percentage to your clipboard.

### **Practical Usage**

**Precise Elevation Analysis**: Use the Height Picker to measure specific elevations and ensure accuracy in terrain features.

**Design Reference**: Quickly compare different points on your terrain to maintain consistency in elevation.

**Integration with Other Tools**: The raw value and percentage are handy for procedural adjustments or setting reference points in other nodes.

The Height Picker tool provides essential elevation data with a single click, making it a powerful ally for accurate terrain design in Nerthus.

## Measurement Tool

The Measurement Tool in Nerthus’s 2D viewport allows you to measure the distance between two points on your terrain. 

### **How to Use the Measurement Tool**

1. **Select Two Points**: Click on any two points in the 2D viewport to define the measurement line. Nerthus will automatically connect the points and display the measurement.
2. **Information Displayed**:
   * **Coordinates**: Displays the XYZ coordinates of each selected point.
   * **Distance 2D**: The straight-line distance (ignoring elevation) between the two points.
   * **Distance 3D**: The actual spatial distance considering the elevation difference between the points.
   * **Difference**: The difference between the 2D and 3D distances, highlighting the impact of elevation on the measurement.

### **Practical Usage**

**Accurate Terrain Scaling**: Use the tool to gauge distances accurately on your terrain for scaling purposes.

**Elevation Analysis**: The 2D vs. 3D distance comparison helps assess how much elevation affects travel distance across the terrain.

***

The Measurement Tool provides quick, precise measurements to assist in planning, scaling, and understanding the terrain’s topography in Nerthus.
